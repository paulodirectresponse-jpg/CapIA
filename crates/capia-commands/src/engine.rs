//! Command Engine: transações atômicas, histórico/undo/redo, idempotência por `operation_id`
//! (ADR-029) e `preview → apply_plan` por token HMAC (ADR-030). Puro: sem IO, sem relógio, sem
//! aleatoriedade — o chamador injeta `now_ms` e a chave do token.

use crate::command::{Command, CommandEnvelope, RippleScope, Transaction};
use crate::ctx::{CommandOutput, Ctx, touched_sequences, touches_nested_graph};
use crate::error::{CommandError, Result};
use crate::exec::execute_command;
use crate::hash::{canonical_json, constant_time_eq, hex, hmac_sha256, sha256_hex};
use crate::journal::{Journal, JournalRecord};
use capia_model::{
    Document, EntityRef, ErrorCode, PrimitiveOp, validate_document, validate_nested_graph,
    validate_sequence,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    Agent,
    Api,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

impl Actor {
    pub fn new(kind: ActorKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }

    pub fn user(id: impl Into<String>) -> Self {
        Self::new(ActorKind::User, id)
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self::new(ActorKind::Agent, id)
    }

    pub fn api(id: impl Into<String>) -> Self {
        Self::new(ActorKind::Api, id)
    }

    pub fn system() -> Self {
        Self::new(ActorKind::System, "system")
    }

    /// `Agent` e `Api` só escrevem por `preview → apply_plan` (ADR-030).
    pub fn requires_preview(&self) -> bool {
        matches!(self.kind, ActorKind::Agent | ActorKind::Api)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSummary {
    pub operation_id: String,
    pub command_type: String,
    pub label: String,
}

/// Uma entrada do histórico: o que foi pedido (auditoria) e o que mudou (undo/redo/replay).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    pub revision_before: u64,
    pub revision_after: u64,
    pub label: String,
    pub actor: Actor,
    pub transaction_id: Option<String>,
    pub plan_id: Option<String>,
    pub commands: Vec<CommandSummary>,
    pub ops: Vec<PrimitiveOp>,
    /// Já na ordem de aplicação do undo (inversas em ordem reversa).
    pub inverse_ops: Vec<PrimitiveOp>,
    pub affected: BTreeSet<EntityRef>,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedOperation {
    pub payload_hash: String,
    pub history_entry_id: u64,
    pub applied_at_ms: u64,
    pub actor: Actor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitResult {
    pub entry_id: u64,
    pub revision_before: u64,
    pub revision: u64,
    /// `true` quando é o resultado original devolvido por reenvio idempotente (nada reaplicado).
    pub replayed: bool,
    /// `ref` simbólica → id real.
    pub refs: BTreeMap<String, String>,
    pub results: Vec<CommandOutput>,
    pub affected: Vec<EntityRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewResult {
    /// `None` quando a transação já foi aplicada por inteiro (`already_applied`).
    pub plan_token: Option<String>,
    pub already_applied: bool,
    pub base_revision: u64,
    pub expires_at_ms: u64,
    pub plan_digest: String,
    pub diff_digest: String,
    /// O diff: ops primitivas que seriam aplicadas.
    pub ops: Vec<PrimitiveOp>,
    pub results: Vec<CommandOutput>,
    pub refs: BTreeMap<String, String>,
    pub affected: Vec<EntityRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    Commit,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub kind: AuditKind,
    pub entry_id: u64,
    pub revision: u64,
    pub timestamp_ms: u64,
    pub actor: Actor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    pub plan_ttl_ms: u64,
    pub max_plans: usize,
    pub default_max_ops: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            plan_ttl_ms: 15 * 60 * 1000,
            max_plans: 64,
            default_max_ops: 10_000,
        }
    }
}

const HARD_MAX_OPS: usize = 1_000_000;
const MAX_OPERATION_ID_LEN: usize = 128;
const PLAN_SCOPE: &str = "document";

struct StoredPlan {
    tx: Transaction,
    base_revision: u64,
    actor: Actor,
    plan_digest: String,
    diff_digest: String,
    expires_at_ms: u64,
    consumed_entry: Option<u64>,
}

struct Executed {
    doc: Document,
    ops: Vec<PrimitiveOp>,
    results: Vec<CommandOutput>,
    refs: BTreeMap<String, String>,
    affected: BTreeSet<EntityRef>,
}

enum Idempotency {
    Fresh,
    Replay(u64),
}

#[derive(Serialize)]
struct PayloadView<'a> {
    #[serde(rename = "ref")]
    reference: &'a Option<String>,
    command: &'a Command,
}

pub struct Engine {
    doc: Document,
    history: Vec<HistoryEntry>,
    /// Quantas entradas de `history` estão aplicadas (o resto é o ramo de redo).
    cursor: usize,
    applied: BTreeMap<String, AppliedOperation>,
    commit_results: BTreeMap<u64, CommitResult>,
    change_log: Vec<(u64, BTreeSet<EntityRef>)>,
    audit: Vec<AuditEvent>,
    plans: BTreeMap<String, StoredPlan>,
    plan_order: VecDeque<String>,
    next_plan: u64,
    next_entry: u64,
    key: [u8; 32],
    config: EngineConfig,
    journal: Option<Box<dyn Journal>>,
}

/// Estado durável do engine (tudo menos planos/chave, que nunca sobrevivem a reinício — ADR-030).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineState {
    pub doc: Document,
    /// Ramo ativo + ramo de redo, na ordem da pilha.
    pub history: Vec<HistoryEntry>,
    pub cursor: usize,
    pub applied: BTreeMap<String, AppliedOperation>,
    pub commit_results: BTreeMap<u64, CommitResult>,
    pub change_log: Vec<(u64, BTreeSet<EntityRef>)>,
    pub audit: Vec<AuditEvent>,
    pub next_entry: u64,
}

impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // a chave do token nunca aparece em Debug/logs
        f.debug_struct("Engine")
            .field("revision", &self.doc.revision)
            .field("history", &self.history.len())
            .field("cursor", &self.cursor)
            .field("plans", &self.plans.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// `plan_key`: 256 bits aleatórios **por processo**, só em memória (nunca persistir/logar).
    pub fn new(doc: Document, plan_key: [u8; 32]) -> Self {
        Self::with_config(doc, plan_key, EngineConfig::default())
    }

    pub fn with_config(doc: Document, plan_key: [u8; 32], config: EngineConfig) -> Self {
        Self {
            doc,
            history: Vec::new(),
            cursor: 0,
            applied: BTreeMap::new(),
            commit_results: BTreeMap::new(),
            change_log: Vec::new(),
            audit: Vec::new(),
            plans: BTreeMap::new(),
            plan_order: VecDeque::new(),
            next_plan: 1,
            next_entry: 1,
            key: plan_key,
            config,
            journal: None,
        }
    }

    /// Injeta o backend durável. A partir daqui toda mudança é persistida **antes** de publicada.
    pub fn set_journal(&mut self, journal: Box<dyn Journal>) {
        self.journal = Some(journal);
    }

    /// Reconstrói um engine a partir de estado persistido, validando a consistência interna.
    pub fn restore(state: EngineState, plan_key: [u8; 32], config: EngineConfig) -> Result<Self> {
        let bad = |msg: String| CommandError::new(ErrorCode::InvariantViolation, msg);
        let EngineState {
            doc,
            history,
            cursor,
            applied,
            commit_results,
            change_log,
            audit,
            next_entry,
        } = state;
        if i64::try_from(doc.revision).is_err() || i64::try_from(next_entry).is_err() {
            return Err(bad(
                "revision or entry counter beyond the supported range".into()
            ));
        }
        if cursor > history.len() {
            return Err(bad(format!(
                "cursor {cursor} beyond history length {}",
                history.len()
            )));
        }
        if let Some(max) = history.iter().map(|h| h.id).max()
            && next_entry <= max
        {
            return Err(bad("next_entry is not greater than every history id".into()));
        }
        if let Some(v) = validate_document(&doc).first() {
            return Err(CommandError::from_violation(v));
        }
        if let Some(last) = audit.last()
            && last.revision != doc.revision
        {
            return Err(bad(format!(
                "audit head revision {} != document revision {}",
                last.revision, doc.revision
            )));
        }
        if let Some(op) = applied
            .iter()
            .find(|(_, a)| !commit_results.contains_key(&a.history_entry_id))
        {
            return Err(bad(format!(
                "operation {} points to a missing result",
                op.0
            )));
        }
        let mut engine = Self::with_config(doc, plan_key, config);
        engine.history = history;
        engine.cursor = cursor;
        engine.applied = applied;
        engine.commit_results = commit_results;
        engine.change_log = change_log;
        engine.audit = audit;
        engine.next_entry = next_entry;
        Ok(engine)
    }

    /// Cópia do estado durável (para snapshots, testes e ferramentas).
    pub fn export_state(&self) -> EngineState {
        EngineState {
            doc: self.doc.clone(),
            history: self.history.clone(),
            cursor: self.cursor,
            applied: self.applied.clone(),
            commit_results: self.commit_results.clone(),
            change_log: self.change_log.clone(),
            audit: self.audit.clone(),
            next_entry: self.next_entry,
        }
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn revision(&self) -> u64 {
        self.doc.revision
    }

    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// Entradas aplicadas (as que um undo desfaria, da mais antiga à mais nova).
    pub fn applied_history(&self) -> &[HistoryEntry] {
        &self.history[..self.cursor]
    }

    pub fn audit_log(&self) -> &[AuditEvent] {
        &self.audit
    }

    pub fn applied_operation(&self, operation_id: &str) -> Option<&AppliedOperation> {
        self.applied.get(operation_id)
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.history.len()
    }

    pub fn pending_plans(&self) -> usize {
        self.plans.len()
    }

    // ---- escrita direta (User/System) ---------------------------------------------------------

    /// Executa e commita. Atores `Agent`/`Api` recebem `PREVIEW_REQUIRED` (ADR-030).
    pub fn execute(&mut self, actor: &Actor, tx: Transaction, now_ms: u64) -> Result<CommitResult> {
        if actor.requires_preview() {
            return Err(CommandError::new(
                ErrorCode::PreviewRequired,
                "this actor may only write through preview -> apply_plan",
            ));
        }
        if let Idempotency::Replay(entry) = self.check_idempotency(&tx)? {
            return Ok(self.replay_result(entry));
        }
        let executed = self.prepare(&tx, true)?;
        self.commit(actor, &tx, executed, now_ms, None)
    }

    // ---- preview → apply_plan -----------------------------------------------------------------

    pub fn preview(
        &mut self,
        actor: &Actor,
        tx: Transaction,
        now_ms: u64,
    ) -> Result<PreviewResult> {
        if let Idempotency::Replay(_) = self.check_idempotency(&tx)? {
            return Ok(PreviewResult {
                plan_token: None,
                already_applied: true,
                base_revision: self.doc.revision,
                expires_at_ms: now_ms,
                plan_digest: sha256_hex(canonical_json(&tx).as_bytes()),
                diff_digest: sha256_hex(b"[]"),
                ops: Vec::new(),
                results: Vec::new(),
                refs: BTreeMap::new(),
                affected: Vec::new(),
            });
        }
        let executed = self.prepare(&tx, true)?;
        self.purge_plans(now_ms);
        while self.plans.len() >= self.config.max_plans {
            match self.plan_order.pop_front() {
                Some(old) => {
                    self.plans.remove(&old);
                }
                None => break,
            }
        }
        let plan_id = format!("plan_{}", self.next_plan);
        self.next_plan += 1;
        let plan_digest = sha256_hex(canonical_json(&tx).as_bytes());
        let diff_digest = sha256_hex(canonical_json(&executed.ops).as_bytes());
        let expires_at_ms = now_ms.saturating_add(self.config.plan_ttl_ms);
        let base_revision = self.doc.revision;
        let mac = self.mac(
            &plan_id,
            &plan_digest,
            &diff_digest,
            base_revision,
            &actor.id,
            expires_at_ms,
        );
        let token = format!("{plan_id}.{}", hex(&mac));
        self.plans.insert(
            plan_id.clone(),
            StoredPlan {
                tx,
                base_revision,
                actor: actor.clone(),
                plan_digest: plan_digest.clone(),
                diff_digest: diff_digest.clone(),
                expires_at_ms,
                consumed_entry: None,
            },
        );
        self.plan_order.push_back(plan_id);
        Ok(PreviewResult {
            plan_token: Some(token),
            already_applied: false,
            base_revision,
            expires_at_ms,
            plan_digest,
            diff_digest,
            ops: executed.ops,
            results: executed.results,
            refs: executed.refs,
            affected: executed.affected.into_iter().collect(),
        })
    }

    /// Aplica **só** o plano revisado (recebe o token, nunca o plano de novo).
    pub fn apply_plan(&mut self, actor: &Actor, token: &str, now_ms: u64) -> Result<CommitResult> {
        let invalid = || CommandError::new(ErrorCode::PlanTokenInvalid, "invalid plan token");
        let (plan_id, mac_hex) = token.split_once('.').ok_or_else(invalid)?;
        let plan = self.plans.get(plan_id).ok_or_else(invalid)?;
        let expected = self.mac(
            plan_id,
            &plan.plan_digest,
            &plan.diff_digest,
            plan.base_revision,
            &plan.actor.id,
            plan.expires_at_ms,
        );
        if !constant_time_eq(hex(&expected).as_bytes(), mac_hex.as_bytes()) {
            return Err(invalid());
        }
        if &plan.actor != actor {
            return Err(CommandError::new(
                ErrorCode::PermissionDenied,
                "plan token belongs to a different actor",
            ));
        }
        if let Some(entry) = plan.consumed_entry {
            // reenvio após sucesso: idempotente, devolve o resultado original
            return Ok(self.replay_result(entry));
        }
        if now_ms > plan.expires_at_ms {
            return Err(CommandError::new(
                ErrorCode::PlanExpired,
                "plan expired: preview again",
            ));
        }
        let tx = plan.tx.clone();
        let diff_digest = plan.diff_digest.clone();
        let plan_id = plan_id.to_owned();
        if let Idempotency::Replay(entry) = self.check_idempotency(&tx)? {
            return Ok(self.replay_result(entry));
        }
        // Rebase: recomputa sobre o estado atual; só prossegue se o diff for idêntico (§4.2.5).
        let executed = self.prepare(&tx, false).map_err(|cause| {
            CommandError::new(
                ErrorCode::PlanStateChanged,
                format!(
                    "the document changed since the preview and the plan no longer applies ({})",
                    cause.code
                ),
            )
            .with_hint(json!({ "cause": cause }))
        })?;
        if sha256_hex(canonical_json(&executed.ops).as_bytes()) != diff_digest {
            return Err(CommandError::new(
                ErrorCode::PlanStateChanged,
                "the document changed since the preview: the resulting diff is different, preview again",
            ));
        }
        let result = self.commit(actor, &tx, executed, now_ms, Some(plan_id.clone()))?;
        if let Some(p) = self.plans.get_mut(&plan_id) {
            p.consumed_entry = Some(result.entry_id);
        }
        Ok(result)
    }

    // ---- undo / redo --------------------------------------------------------------------------

    /// Desfaz a última entrada aplicada (aplica as `inverse_ops` gravadas — não recalcula comandos).
    pub fn undo(&mut self, actor: &Actor, now_ms: u64) -> Result<CommitResult> {
        if self.cursor == 0 {
            return Err(CommandError::new(
                ErrorCode::NothingToUndo,
                "nothing to undo",
            ));
        }
        let entry = self.history[self.cursor - 1].clone();
        let result = self.replay_ops(&entry, &entry.inverse_ops, actor, now_ms, AuditKind::Undo)?;
        self.cursor -= 1;
        Ok(result)
    }

    pub fn redo(&mut self, actor: &Actor, now_ms: u64) -> Result<CommitResult> {
        if self.cursor >= self.history.len() {
            return Err(CommandError::new(
                ErrorCode::NothingToRedo,
                "nothing to redo",
            ));
        }
        let entry = self.history[self.cursor].clone();
        let result = self.replay_ops(&entry, &entry.ops, actor, now_ms, AuditKind::Redo)?;
        self.cursor += 1;
        Ok(result)
    }

    fn replay_ops(
        &mut self,
        entry: &HistoryEntry,
        ops: &[PrimitiveOp],
        actor: &Actor,
        now_ms: u64,
        kind: AuditKind,
    ) -> Result<CommitResult> {
        let mut doc = self.doc.clone();
        doc.apply_ops(ops)?;
        self.validate_touched(&doc, ops)?;
        let revision_before = self.doc.revision;
        doc.revision = next_revision(revision_before)?;
        let event = AuditEvent {
            kind,
            entry_id: entry.id,
            revision: doc.revision,
            timestamp_ms: now_ms,
            actor: actor.clone(),
        };
        // persistir antes de publicar: se o journal falhar, o engine continua exatamente como estava
        self.append_journal(
            &match kind {
                AuditKind::Undo => JournalRecord::Undo { event: &event },
                _ => JournalRecord::Redo { event: &event },
            },
            &doc,
        )?;
        self.doc = doc;
        self.change_log
            .push((self.doc.revision, entry.affected.clone()));
        self.audit.push(event);
        Ok(CommitResult {
            entry_id: entry.id,
            revision_before,
            revision: self.doc.revision,
            replayed: false,
            refs: BTreeMap::new(),
            results: Vec::new(),
            affected: entry.affected.iter().cloned().collect(),
        })
    }

    fn append_journal(&mut self, record: &JournalRecord<'_>, doc_after: &Document) -> Result<()> {
        let Some(journal) = self.journal.as_mut() else {
            return Ok(());
        };
        journal.append(record, doc_after).map_err(|e| {
            CommandError::new(
                ErrorCode::PersistenceFailed,
                format!("could not persist the change: {}", e.message),
            )
            .with_hint(json!({ "store_code": e.kind, "cause": e.cause }))
        })
    }

    // ---- internos -----------------------------------------------------------------------------

    fn mac(
        &self,
        plan_id: &str,
        plan_digest: &str,
        diff_digest: &str,
        base_revision: u64,
        actor_id: &str,
        expires_at_ms: u64,
    ) -> [u8; 32] {
        let msg = format!(
            "{plan_id}|{plan_digest}|{diff_digest}|{base_revision}|{actor_id}|{PLAN_SCOPE}|{expires_at_ms}"
        );
        hmac_sha256(&self.key, msg.as_bytes())
    }

    fn purge_plans(&mut self, now_ms: u64) {
        let expired: Vec<String> = self
            .plans
            .iter()
            .filter(|(_, p)| p.expires_at_ms < now_ms)
            .map(|(k, _)| k.clone())
            .collect();
        for id in expired {
            self.plans.remove(&id);
            self.plan_order.retain(|p| p != &id);
        }
    }

    fn payload_hash(env: &CommandEnvelope) -> String {
        sha256_hex(
            canonical_json(&PayloadView {
                reference: &env.reference,
                command: &env.command,
            })
            .as_bytes(),
        )
    }

    /// Regras de idempotência (§4.1): tudo conhecido com o mesmo hash ⇒ replay; mesmo id com hash
    /// diferente ⇒ `OPERATION_ID_REUSED`; parte conhecida ⇒ `OPERATION_ID_CONFLICT`.
    fn check_idempotency(&self, tx: &Transaction) -> Result<Idempotency> {
        let mut seen = BTreeSet::new();
        let (mut known, mut entry) = (0usize, None);
        for env in &tx.commands {
            let id = &env.operation_id;
            if id.is_empty() || id.len() > MAX_OPERATION_ID_LEN {
                return Err(CommandError::invalid(format!(
                    "operation_id must have 1..={MAX_OPERATION_ID_LEN} characters"
                )));
            }
            if !seen.insert(id.as_str()) {
                return Err(CommandError::new(
                    ErrorCode::OperationIdConflict,
                    format!("operation_id {id} appears twice in the transaction"),
                ));
            }
            if let Some(applied) = self.applied.get(id) {
                if applied.payload_hash != Self::payload_hash(env) {
                    return Err(CommandError::new(
                        ErrorCode::OperationIdReused,
                        format!("operation_id {id} was already used with a different command"),
                    ));
                }
                known += 1;
                entry.get_or_insert(applied.history_entry_id);
            }
        }
        match (known, entry) {
            (0, _) => Ok(Idempotency::Fresh),
            (n, Some(e)) if n == tx.commands.len() => Ok(Idempotency::Replay(e)),
            _ => Err(CommandError::new(
                ErrorCode::OperationIdConflict,
                "some operation_ids of this transaction were already applied and others were not",
            )),
        }
    }

    fn replay_result(&self, entry_id: u64) -> CommitResult {
        match self.commit_results.get(&entry_id) {
            Some(r) => CommitResult {
                replayed: true,
                ..r.clone()
            },
            None => CommitResult {
                entry_id,
                revision_before: self.doc.revision,
                revision: self.doc.revision,
                replayed: true,
                refs: BTreeMap::new(),
                results: Vec::new(),
                affected: Vec::new(),
            },
        }
    }

    fn validate_touched(&self, doc: &Document, ops: &[PrimitiveOp]) -> Result<()> {
        for seq_id in touched_sequences(ops) {
            if let Some(seq) = doc.sequence(&seq_id)
                && let Some(v) = validate_sequence(doc, &seq_id, seq).first()
            {
                return Err(CommandError::from_violation(v));
            }
        }
        if touches_nested_graph(ops)
            && let Some(v) = validate_nested_graph(doc).first()
        {
            return Err(CommandError::from_violation(v));
        }
        Ok(())
    }

    /// Executa a transação numa working copy do estado atual, sem commitar.
    fn prepare(&self, tx: &Transaction, check_conflicts: bool) -> Result<Executed> {
        if tx.commands.is_empty() {
            return Err(CommandError::invalid("transaction has no commands"));
        }
        let current = self.doc.revision;
        if let Some(base) = tx.base_revision
            && base > current
        {
            return Err(CommandError::invalid(format!(
                "base_revision {base} is newer than the current revision {current}"
            )));
        }
        let max_ops = tx
            .max_ops
            .unwrap_or(self.config.default_max_ops)
            .min(HARD_MAX_OPS);
        let mut ctx = Ctx::new(self.doc.clone(), max_ops);
        let mut refs: BTreeMap<String, String> = BTreeMap::new();
        let mut results = Vec::with_capacity(tx.commands.len());
        for (index, env) in tx.commands.iter().enumerate() {
            ctx.begin_command(&env.operation_id);
            let mut command = env.command.clone();
            resolve_refs(&mut command, &refs).map_err(|e| e.at(index))?;
            let mut out = execute_command(&mut ctx, &command).map_err(|e| e.at(index))?;
            // follow_length: reconcilia nested que acompanham a duração da filha (ADR-045)
            crate::exec::reconcile_follow(&mut ctx).map_err(|e| e.at(index))?;
            out.warnings.append(&mut ctx.warnings);
            if let Some(name) = &env.reference {
                if !name.starts_with('$') || name.len() < 2 {
                    return Err(CommandError::invalid("ref must look like $name").at(index));
                }
                let Some(id) = out.id.clone() else {
                    return Err(CommandError::invalid(
                        "this command creates nothing to bind to a ref",
                    )
                    .at(index));
                };
                if refs.insert(name.clone(), id).is_some() {
                    return Err(
                        CommandError::invalid(format!("ref {name} defined twice")).at(index)
                    );
                }
            }
            results.push(out);
        }
        let Ctx { mut doc, ops, .. } = ctx;
        self.validate_touched(&doc, &ops)?;
        let affected: BTreeSet<EntityRef> = ops.iter().flat_map(PrimitiveOp::affected).collect();
        if check_conflicts
            && let Some(base) = tx.base_revision
            && base < current
        {
            // rebase automático só se nada do que a transação toca mudou desde `base`
            let clash: BTreeSet<&EntityRef> = self
                .change_log
                .iter()
                .filter(|(rev, _)| *rev > base)
                .flat_map(|(_, set)| set.iter())
                .filter(|e| affected.contains(*e))
                .collect();
            if !clash.is_empty() {
                return Err(CommandError::new(
                    ErrorCode::Conflict,
                    format!(
                        "entities changed since revision {base}: re-plan this part of the edit"
                    ),
                )
                .with_entities(clash.into_iter().cloned()));
            }
        }
        doc.revision = next_revision(current)?;
        Ok(Executed {
            doc,
            ops,
            results,
            refs,
            affected,
        })
    }

    fn commit(
        &mut self,
        actor: &Actor,
        tx: &Transaction,
        executed: Executed,
        now_ms: u64,
        plan_id: Option<String>,
    ) -> Result<CommitResult> {
        let Executed {
            doc,
            ops,
            results,
            refs,
            affected,
        } = executed;
        let id = self.next_entry;
        let revision_before = self.doc.revision;
        let revision = doc.revision;
        let inverse_ops: Vec<PrimitiveOp> = ops.iter().rev().map(PrimitiveOp::inverse).collect();
        let entry = HistoryEntry {
            id,
            revision_before,
            revision_after: revision,
            label: tx.label.clone(),
            actor: actor.clone(),
            transaction_id: tx.transaction_id.clone(),
            plan_id,
            commands: tx
                .commands
                .iter()
                .map(|e| CommandSummary {
                    operation_id: e.operation_id.clone(),
                    command_type: e.command.type_name().to_owned(),
                    label: e.command.label(),
                })
                .collect(),
            ops,
            inverse_ops,
            affected: affected.clone(),
            timestamp_ms: now_ms,
        };
        let applied: Vec<(String, AppliedOperation)> = tx
            .commands
            .iter()
            .map(|env| {
                (
                    env.operation_id.clone(),
                    AppliedOperation {
                        payload_hash: Self::payload_hash(env),
                        history_entry_id: id,
                        applied_at_ms: now_ms,
                        actor: actor.clone(),
                    },
                )
            })
            .collect();
        let event = AuditEvent {
            kind: AuditKind::Commit,
            entry_id: id,
            revision,
            timestamp_ms: now_ms,
            actor: actor.clone(),
        };
        let result = CommitResult {
            entry_id: id,
            revision_before,
            revision,
            replayed: false,
            refs,
            results,
            affected: affected.iter().cloned().collect(),
        };
        // persistir antes de publicar (ADR-043): falha ⇒ nada mudou em memória
        self.append_journal(
            &JournalRecord::Commit {
                entry: &entry,
                result: &result,
                applied: &applied,
                event: &event,
            },
            &doc,
        )?;
        self.next_entry = self.next_entry.checked_add(1).ok_or_else(|| {
            CommandError::new(ErrorCode::LimitExceeded, "history entry id space exhausted")
        })?;
        for (op_id, record) in applied {
            self.applied.insert(op_id, record);
        }
        // nova edição descarta o ramo de redo (as ops dele continuam na auditoria)
        self.history.truncate(self.cursor);
        self.history.push(entry);
        self.cursor = self.history.len();
        self.doc = doc;
        self.change_log.push((revision, affected));
        self.audit.push(event);
        self.commit_results.insert(id, result.clone());
        Ok(result)
    }
}

/// Revisão seguinte, sem overflow silencioso: o teto é `i64::MAX` (o que o store consegue gravar).
fn next_revision(current: u64) -> Result<u64> {
    match current.checked_add(1) {
        Some(n) if i64::try_from(n).is_ok() => Ok(n),
        _ => Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "document revision space exhausted",
        )),
    }
}

// ---- referências simbólicas ($nome) -------------------------------------------------------------

fn sub(s: &mut String, refs: &BTreeMap<String, String>) -> Result<()> {
    if !s.starts_with('$') {
        return Ok(());
    }
    match refs.get(s.as_str()) {
        Some(real) => {
            s.clone_from(real);
            Ok(())
        }
        None => Err(CommandError::new(
            ErrorCode::UnresolvedRef,
            format!("unresolved reference {s}"),
        )
        .with_hint(json!({ "known_refs": refs.keys().collect::<Vec<_>>() }))),
    }
}

fn sub_scope(scope: &mut RippleScope, refs: &BTreeMap<String, String>) -> Result<()> {
    if let RippleScope::Tracks { tracks } = scope {
        for t in tracks {
            sub(&mut t.0, refs)?;
        }
    }
    Ok(())
}

/// Troca `$ref` por ids reais nos campos que **referenciam** entidades.
fn resolve_refs(cmd: &mut Command, refs: &BTreeMap<String, String>) -> Result<()> {
    use capia_model::ClipContent;
    match cmd {
        Command::RegisterAsset { .. }
        | Command::DeleteAsset { .. }
        | Command::UpdateAsset { .. }
        | Command::CreateSequence { .. } => {}
        Command::AddTrack { sequence, .. } | Command::AddMarker { sequence, .. } => {
            sub(&mut sequence.0, refs)?
        }
        Command::SetTrackFlags { track, .. } | Command::DeleteTrack { track } => {
            sub(&mut track.0, refs)?
        }
        Command::DeleteSequence { sequence } | Command::RenameSequence { sequence, .. } => {
            sub(&mut sequence.0, refs)?
        }
        Command::InsertNested {
            track, sequence, ..
        } => {
            sub(&mut track.0, refs)?;
            sub(&mut sequence.0, refs)?;
        }
        Command::SetNestedTarget { clip, sequence } => {
            sub(&mut clip.0, refs)?;
            sub(&mut sequence.0, refs)?;
        }
        Command::SetFollowLength { clip, .. } => sub(&mut clip.0, refs)?,
        Command::DuplicateSequence {
            source,
            new_sequence,
            ..
        } => {
            sub(&mut source.0, refs)?;
            if let Some(n) = new_sequence {
                sub(&mut n.0, refs)?;
            }
        }
        Command::MakeUnique {
            clip, new_sequence, ..
        } => {
            sub(&mut clip.0, refs)?;
            if let Some(n) = new_sequence {
                sub(&mut n.0, refs)?;
            }
        }
        Command::FlattenNested { clip, .. } => sub(&mut clip.0, refs)?,
        Command::CreateNestedFromSelection {
            clips,
            new_sequence,
            clip_id,
            track,
            ..
        } => {
            for c in clips {
                sub(&mut c.0, refs)?;
            }
            if let Some(n) = new_sequence {
                sub(&mut n.0, refs)?;
            }
            if let Some(c) = clip_id {
                sub(&mut c.0, refs)?;
            }
            if let Some(t) = track {
                sub(&mut t.0, refs)?;
            }
        }
        Command::GenerateVariants { template, variants } => {
            sub(&mut template.0, refs)?;
            for v in variants {
                if let Some(s) = &mut v.sequence {
                    sub(&mut s.0, refs)?;
                }
                for swap in &mut v.swaps {
                    match swap {
                        crate::command::VariantSwap::SetNested { clip, sequence } => {
                            sub(&mut clip.0, refs)?;
                            sub(&mut sequence.0, refs)?;
                        }
                        crate::command::VariantSwap::ReplaceMedia { clip, asset } => {
                            sub(&mut clip.0, refs)?;
                            sub(&mut asset.0, refs)?;
                        }
                    }
                }
            }
        }
        Command::MoveMarker {
            sequence, marker, ..
        }
        | Command::DeleteMarker { sequence, marker } => {
            sub(&mut sequence.0, refs)?;
            sub(&mut marker.0, refs)?;
        }
        Command::InsertClip { track, clip, .. } => {
            sub(&mut track.0, refs)?;
            match &mut clip.content {
                ClipContent::Media { asset, .. } | ClipContent::Image { asset } => {
                    sub(&mut asset.0, refs)?
                }
                ClipContent::Nested { sequence, .. } => sub(&mut sequence.0, refs)?,
                ClipContent::Text { .. } | ClipContent::Solid { .. } => {}
            }
        }
        Command::MoveClips { moves } => {
            for m in moves {
                sub(&mut m.clip.0, refs)?;
                if let Some(t) = &mut m.track {
                    sub(&mut t.0, refs)?;
                }
            }
        }
        Command::DeleteClip { clip, scope, .. }
        | Command::TrimClip { clip, scope, .. }
        | Command::SetClipSpeed { clip, scope, .. } => {
            sub(&mut clip.0, refs)?;
            sub_scope(scope, refs)?;
        }
        Command::SetSequenceFormat { sequence, .. }
        | Command::SetSequenceFolder { sequence, .. } => sub(&mut sequence.0, refs)?,
        Command::CreateFolder { .. }
        | Command::RenameFolder { .. }
        | Command::MoveFolder { .. }
        | Command::DeleteFolder { .. }
        | Command::CreateDeliverable { .. }
        | Command::UpdateDeliverable { .. }
        | Command::DeleteDeliverable { .. } => {}
        Command::RenameClip { clip, .. }
        | Command::SetClipEnabled { clip, .. }
        | Command::SetText { clip, .. }
        | Command::SetTransition { clip, .. } => sub(&mut clip.0, refs)?,
        Command::DetachAudio {
            clip,
            audio_track,
            audio_clip_id,
        } => {
            sub(&mut clip.0, refs)?;
            if let Some(t) = audio_track {
                sub(&mut t.0, refs)?;
            }
            if let Some(c) = audio_clip_id {
                sub(&mut c.0, refs)?;
            }
        }
        Command::GroupClips { clips, .. } | Command::Ungroup { clips } => {
            for c in clips {
                sub(&mut c.0, refs)?;
            }
        }
        Command::SplitClip { clip, .. }
        | Command::SetProperty { clip, .. }
        | Command::AddKeyframe { clip, .. }
        | Command::MoveKeyframe { clip, .. }
        | Command::DeleteKeyframe { clip, .. }
        | Command::SetKeyframeInterp { clip, .. } => sub(&mut clip.0, refs)?,
    }
    Ok(())
}
