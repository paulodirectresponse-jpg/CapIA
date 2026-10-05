//! Memória controlada em **4 escopos** (PHASE5_MEMORY_GATEWAY §1..§9): System (produto) · User ·
//! Client · Project. Precedência `Project > Client > User > System`.
//!
//! Regra dura: **nada de User/Client é promovido sem aprovação humana explícita.** A IA só
//! *propõe*; o único caminho para `Active` em User/Client é [`MemoryManager::approve`], que exige
//! um [`UserApproval`] — valor que só o serviço (ação explícita da UI) constrói. Propostas vindas
//! de saída de modelo, de tools, de brief importado ou de correções repetidas ficam `Proposed`.
//!
//! Persistência: Project em `ai_memory` (`.capia`); User/Client no banco global do app (`AppDb`,
//! sem segredos); System embutido e versionado. Toda mudança é registrada (`ai_memory_log`).

use crate::error::{IntelError, IntelResult};
use crate::records::now_ms;
use capia_store::{AppDb, AutonomyStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const SYSTEM_MEMORY_VERSION: u32 = 1;
/// Correções repetidas necessárias antes de *propor* uma memória.
pub const SIGNAL_THRESHOLD: usize = 3;
const NS_USER: &str = "memory.user";
const NS_CLIENT: &str = "memory.client";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    System,
    User,
    Client,
    Project,
}

impl MemoryScope {
    /// Maior = vence (Project > Client > User > System).
    pub fn precedence(self) -> u8 {
        match self {
            Self::System => 0,
            Self::User => 1,
            Self::Client => 2,
            Self::Project => 3,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "client" => Some(Self::Client),
            "project" => Some(Self::Project),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Client => "client",
            Self::Project => "project",
        }
    }

    /// Escopos que exigem aprovação humana explícita para ficarem ativos.
    pub fn needs_human(self) -> bool {
        matches!(self, Self::User | Self::Client)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Rule,
    Style,
    Fact,
    Correction,
}

impl MemoryKind {
    pub fn parse(s: &str) -> Self {
        match s {
            "rule" => Self::Rule,
            "style" => Self::Style,
            "fact" => Self::Fact,
            "correction" => Self::Correction,
            _ => Self::Preference,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Proposed,
    Active,
    Rejected,
    Archived,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::Archived => "archived",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    Briefing,
    Correction,
    UserDeclared,
    Agent,
    System,
    Import,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub scope: MemoryScope,
    #[serde(default)]
    pub client_id: Option<String>,
    pub kind: MemoryKind,
    pub content: String,
    #[serde(default)]
    pub structured: Option<Value>,
    pub source: MemorySource,
    pub status: MemoryStatus,
    pub confidence: f64,
    pub evidence: Vec<EvidenceRef>,
    pub created_ms: u64,
    pub updated_ms: u64,
    #[serde(default)]
    pub last_used_ms: Option<u64>,
    /// Chave lógica para detectar conflitos (`tone`, `cta_style`, …).
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub origin_run: Option<String>,
}

impl MemoryItem {
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.content.as_bytes());
        let d = h.finalize();
        d[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Rascunho de proposta (vem de papéis/Briefing/sinais — nunca ativa sozinho).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryDraft {
    pub scope: MemoryScope,
    pub client_id: Option<String>,
    pub kind: MemoryKind,
    pub content: String,
    pub structured: Option<Value>,
    pub source: MemorySource,
    pub confidence: f64,
    pub evidence: Vec<EvidenceRef>,
    pub key: Option<String>,
}

/// Prova de que **um humano** aprovou. Só o serviço a constrói (a partir de uma ação explícita da
/// UI); roteiros de modelo, tools e importação não têm como fabricá-la.
#[derive(Clone, Debug)]
pub struct UserApproval {
    actor: String,
    at_ms: u64,
}

impl UserApproval {
    /// Construtor para a **camada de serviço** (chamada `ai.memory.approve` vinda da UI).
    pub fn explicit_from_ui(actor: &str) -> Self {
        Self {
            actor: actor.to_owned(),
            at_ms: now_ms(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryQuery {
    pub client_id: Option<String>,
    pub terms: Vec<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryConflict {
    pub key: String,
    pub winner: String,
    pub losers: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Retrieval {
    pub items: Vec<MemoryItem>,
    pub conflicts: Vec<MemoryConflict>,
}

fn id_for(scope: MemoryScope, client: Option<&str>, content: &str) -> String {
    let mut h = Sha256::new();
    h.update(scope.as_str().as_bytes());
    h.update([0x1f]);
    h.update(client.unwrap_or("").as_bytes());
    h.update([0x1f]);
    h.update(content.trim().to_lowercase().as_bytes());
    let d = h.finalize();
    format!(
        "mem_{}",
        d[..10]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Memória System: regras do produto, versionadas e imutáveis em runtime.
pub fn system_items() -> Vec<MemoryItem> {
    let mk = |content: &str, key: &str| MemoryItem {
        id: format!("sys_{key}"),
        scope: MemoryScope::System,
        client_id: None,
        kind: MemoryKind::Rule,
        content: content.to_owned(),
        structured: Some(json!({"version": SYSTEM_MEMORY_VERSION})),
        source: MemorySource::System,
        status: MemoryStatus::Active,
        confidence: 1.0,
        evidence: vec![],
        created_ms: 0,
        updated_ms: 0,
        last_used_ms: None,
        key: Some(key.to_owned()),
        origin_run: None,
    };
    vec![
        mk(
            "Keep captions inside the platform safe area.",
            "captions_safe_area",
        ),
        mk(
            "Never include content listed in must_avoid; always include the CTA.",
            "brief_constraints",
        ),
        mk(
            "Prefer a hard cut over a transition unless the brief or reference asks for one.",
            "transitions",
        ),
        mk(
            "Prefer existing project assets over acquiring or generating new media.",
            "asset_reuse",
        ),
    ]
}

#[derive(Clone)]
pub struct MemoryManager {
    project: Arc<AutonomyStore>,
    app: Option<Arc<AppDb>>,
}

impl core::fmt::Debug for MemoryManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemoryManager").finish_non_exhaustive()
    }
}

fn store_err(e: capia_store::StoreError) -> IntelError {
    IntelError::new("STORE_ERROR", e.to_string())
}

impl MemoryManager {
    pub fn new(project: Arc<AutonomyStore>, app: Option<Arc<AppDb>>) -> Self {
        Self { project, app }
    }

    fn log(&self, id: &str, event: &str, actor: &str, run: Option<&str>, detail: Value) {
        let _ = self
            .project
            .log_memory(id, event, actor, run, &detail, now_ms());
    }

    fn save(&self, item: &MemoryItem) -> IntelResult<()> {
        let v =
            serde_json::to_value(item).map_err(|e| IntelError::new("INTERNAL", e.to_string()))?;
        match item.scope {
            MemoryScope::Project => self
                .project
                .put_memory(&item.id, item.status.as_str(), &v, now_ms())
                .map_err(store_err),
            MemoryScope::User | MemoryScope::Client => {
                let app = self.app.as_ref().ok_or_else(|| {
                    IntelError::new("NO_APP_DB", "user/client memory needs the app database")
                })?;
                let ns = if item.scope == MemoryScope::User {
                    NS_USER
                } else {
                    NS_CLIENT
                };
                app.put(ns, &item.id, &v, now_ms()).map_err(store_err)
            }
            MemoryScope::System => {
                Err(IntelError::new("NOT_ALLOWED", "system memory is read-only"))
            }
        }
    }

    pub fn get(&self, id: &str) -> IntelResult<Option<MemoryItem>> {
        if let Some(s) = system_items().into_iter().find(|i| i.id == id) {
            return Ok(Some(s));
        }
        if let Some(r) = self.project.get_memory(id).map_err(store_err)?
            && let Ok(i) = serde_json::from_value::<MemoryItem>(r.json)
        {
            return Ok(Some(i));
        }
        if let Some(app) = &self.app {
            for ns in [NS_USER, NS_CLIENT] {
                if let Some(v) = app.get(ns, id).map_err(store_err)?
                    && let Ok(i) = serde_json::from_value::<MemoryItem>(v)
                {
                    return Ok(Some(i));
                }
            }
        }
        Ok(None)
    }

    /// Todos os itens visíveis neste projeto (System + Project + User + Client).
    pub fn list(
        &self,
        scope: Option<MemoryScope>,
        status: Option<MemoryStatus>,
    ) -> IntelResult<Vec<MemoryItem>> {
        let mut out = system_items();
        for r in self.project.list_memory(None).map_err(store_err)? {
            if let Ok(i) = serde_json::from_value::<MemoryItem>(r.json) {
                out.push(i);
            }
        }
        if let Some(app) = &self.app {
            for ns in [NS_USER, NS_CLIENT] {
                for k in app.list_keys(ns).map_err(store_err)? {
                    if let Some(v) = app.get(ns, &k).map_err(store_err)?
                        && let Ok(i) = serde_json::from_value::<MemoryItem>(v)
                    {
                        out.push(i);
                    }
                }
            }
        }
        out.retain(|i| scope.is_none_or(|s| i.scope == s) && status.is_none_or(|s| i.status == s));
        out.sort_by(|a, b| (b.scope.precedence(), &a.id).cmp(&(a.scope.precedence(), &b.id)));
        Ok(out)
    }

    /// A IA (ou o briefing, ou um sinal) **propõe**. Nunca ativa User/Client; Project só ativa se a
    /// política explícita do projeto permitir (`project_auto_activate`). Idempotente por conteúdo;
    /// item rejeitado/arquivado não ressuscita.
    pub fn propose(
        &self,
        d: MemoryDraft,
        run_id: Option<&str>,
        project_auto_activate: bool,
    ) -> IntelResult<MemoryItem> {
        if d.scope == MemoryScope::System {
            return Err(IntelError::new(
                "NOT_ALLOWED",
                "only the product defines system memory",
            ));
        }
        let content = clean(&d.content, 500);
        if content.is_empty() {
            return Err(IntelError::new("INVALID_ARGUMENT", "empty memory content"));
        }
        if d.scope == MemoryScope::Client && d.client_id.as_deref().is_none_or(str::is_empty) {
            return Err(IntelError::new(
                "INVALID_ARGUMENT",
                "client memory needs an explicit client id (never inferred)",
            ));
        }
        let client = if d.scope == MemoryScope::Client {
            d.client_id.clone()
        } else {
            None
        };
        let id = id_for(d.scope, client.as_deref(), &content);
        if let Some(existing) = self.get(&id)? {
            self.log(&id, "proposed_again", "agent", run_id, json!({}));
            return Ok(existing);
        }
        let activate = d.scope == MemoryScope::Project && project_auto_activate;
        let now = now_ms();
        let item = MemoryItem {
            id: id.clone(),
            scope: d.scope,
            client_id: client,
            kind: d.kind,
            content,
            structured: d.structured,
            source: d.source,
            // a rede de segurança central: User/Client nascem Proposed, sempre
            status: if activate {
                MemoryStatus::Active
            } else {
                MemoryStatus::Proposed
            },
            confidence: d.confidence.clamp(0.0, 1.0),
            evidence: d.evidence.into_iter().take(20).collect(),
            created_ms: now,
            updated_ms: now,
            last_used_ms: None,
            key: d.key.map(|k| clean(&k, 60)),
            origin_run: run_id.map(str::to_owned),
        };
        debug_assert!(!(item.scope.needs_human() && item.status == MemoryStatus::Active));
        self.save(&item)?;
        self.log(
            &id,
            if activate { "activated_by_project_policy" } else { "proposed" },
            "agent",
            run_id,
            json!({"scope": item.scope.as_str(), "source": item.source, "evidence": item.evidence.len()}),
        );
        Ok(item)
    }

    /// Aprovação humana explícita: `Proposed → Active`, opcionalmente mudando o escopo (promoção
    /// Project → Client/User) e/ou editando o texto.
    pub fn approve(
        &self,
        id: &str,
        to_scope: Option<MemoryScope>,
        client_id: Option<&str>,
        edited_content: Option<&str>,
        approval: &UserApproval,
    ) -> IntelResult<MemoryItem> {
        let mut item = self
            .get(id)?
            .ok_or_else(|| IntelError::new("NOT_FOUND", "unknown memory item"))?;
        if item.scope == MemoryScope::System {
            return Err(IntelError::new("NOT_ALLOWED", "system memory is read-only"));
        }
        if matches!(item.status, MemoryStatus::Rejected | MemoryStatus::Archived) {
            return Err(IntelError::new(
                "INVALID_STATE",
                "rejected/archived memory cannot be approved: propose it again",
            ));
        }
        let old = item.clone();
        let target = to_scope.unwrap_or(item.scope);
        if target == MemoryScope::System {
            return Err(IntelError::new(
                "NOT_ALLOWED",
                "cannot promote into system memory",
            ));
        }
        if let Some(c) = edited_content {
            let c = clean(c, 500);
            if c.is_empty() {
                return Err(IntelError::new("INVALID_ARGUMENT", "empty memory content"));
            }
            item.content = c;
        }
        if target == MemoryScope::Client {
            let cid = client_id
                .map(str::to_owned)
                .or_else(|| item.client_id.clone());
            if cid.as_deref().is_none_or(str::is_empty) {
                return Err(IntelError::new(
                    "INVALID_ARGUMENT",
                    "client memory needs a client id",
                ));
            }
            item.client_id = cid;
        } else {
            item.client_id = None;
        }
        item.scope = target;
        item.status = MemoryStatus::Active;
        item.updated_ms = approval.at_ms;
        // o id acompanha escopo/cliente: mudou ⇒ item novo no destino e o antigo é arquivado
        let new_id = id_for(item.scope, item.client_id.as_deref(), &item.content);
        let moved = new_id != item.id;
        let old_id = item.id.clone();
        item.id = new_id;
        self.save(&item)?;
        if moved && old.scope != MemoryScope::System {
            self.remove(&old)?;
        }
        self.log(
            &item.id,
            if old.scope == item.scope { "approved" } else { "promoted" },
            &approval.actor,
            None,
            json!({"from_scope": old.scope.as_str(), "to_scope": item.scope.as_str(), "from_id": old_id, "edited": edited_content.is_some()}),
        );
        Ok(item)
    }

    fn remove(&self, item: &MemoryItem) -> IntelResult<()> {
        match item.scope {
            MemoryScope::Project => {
                self.project.delete_memory(&item.id).map_err(store_err)?;
            }
            MemoryScope::User | MemoryScope::Client => {
                if let Some(app) = &self.app {
                    let ns = if item.scope == MemoryScope::User {
                        NS_USER
                    } else {
                        NS_CLIENT
                    };
                    app.delete(ns, &item.id).map_err(store_err)?;
                }
            }
            MemoryScope::System => {}
        }
        Ok(())
    }

    fn set_status(
        &self,
        id: &str,
        status: MemoryStatus,
        actor: &str,
        event: &str,
    ) -> IntelResult<MemoryItem> {
        let mut item = self
            .get(id)?
            .ok_or_else(|| IntelError::new("NOT_FOUND", "unknown memory item"))?;
        if item.scope == MemoryScope::System {
            return Err(IntelError::new("NOT_ALLOWED", "system memory is read-only"));
        }
        item.status = status;
        item.updated_ms = now_ms();
        self.save(&item)?;
        self.log(id, event, actor, None, json!({}));
        Ok(item)
    }

    pub fn reject(&self, id: &str, _a: &UserApproval) -> IntelResult<MemoryItem> {
        self.set_status(id, MemoryStatus::Rejected, "user", "rejected")
    }

    pub fn archive(&self, id: &str, _a: &UserApproval) -> IntelResult<MemoryItem> {
        self.set_status(id, MemoryStatus::Archived, "user", "archived")
    }

    /// Edita o texto (ação humana). O item mantém o status; o id é recalculado pelo conteúdo.
    pub fn edit(&self, id: &str, content: &str, _a: &UserApproval) -> IntelResult<MemoryItem> {
        let mut item = self
            .get(id)?
            .ok_or_else(|| IntelError::new("NOT_FOUND", "unknown memory item"))?;
        if item.scope == MemoryScope::System {
            return Err(IntelError::new("NOT_ALLOWED", "system memory is read-only"));
        }
        let old = item.clone();
        item.content = clean(content, 500);
        if item.content.is_empty() {
            return Err(IntelError::new("INVALID_ARGUMENT", "empty memory content"));
        }
        item.id = id_for(item.scope, item.client_id.as_deref(), &item.content);
        item.updated_ms = now_ms();
        self.save(&item)?;
        if item.id != old.id {
            self.remove(&old)?;
        }
        self.log(&item.id, "edited", "user", None, json!({"from_id": old.id}));
        Ok(item)
    }

    /// Exclusão (ação humana): o conteúdo some; o log mantém só o *digest* (auditoria sem
    /// reconstruir conteúdo apagado).
    pub fn delete(&self, id: &str, _a: &UserApproval) -> IntelResult<bool> {
        let Some(item) = self.get(id)? else {
            return Ok(false);
        };
        if item.scope == MemoryScope::System {
            return Err(IntelError::new("NOT_ALLOWED", "system memory is read-only"));
        }
        self.remove(&item)?;
        self.log(
            id,
            "deleted",
            "user",
            None,
            json!({"content_digest": item.digest()}),
        );
        Ok(true)
    }

    /// Recuperação **relevante** e só de itens `Active`: Client só do cliente explícito; Project só
    /// deste projeto; precedência `Project > Client > User > System`; conflitos por `key` mostram
    /// vencedor e perdedores (com origem) ao Brain.
    pub fn retrieve(&self, q: &MemoryQuery) -> IntelResult<Retrieval> {
        let limit = if q.limit == 0 { 12 } else { q.limit.min(50) };
        let terms: Vec<String> = q
            .terms
            .iter()
            .map(|t| t.to_lowercase())
            .filter(|t| t.len() > 2)
            .collect();
        let mut scored: Vec<(f64, MemoryItem)> = Vec::new();
        for i in self.list(None, Some(MemoryStatus::Active))? {
            if i.scope == MemoryScope::Client
                && (q.client_id.is_none() || i.client_id != q.client_id)
            {
                continue;
            }
            let text = i.content.to_lowercase();
            let hits = terms.iter().filter(|t| text.contains(t.as_str())).count() as f64;
            let base = match i.scope {
                MemoryScope::System => 0.5,
                _ => 1.0,
            };
            let score =
                base + hits * 2.0 + f64::from(i.scope.precedence()) * 0.1 + i.confidence * 0.1;
            scored.push((score, i));
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(a.1.id.cmp(&b.1.id))
        });
        let mut conflicts: Vec<MemoryConflict> = Vec::new();
        let mut by_key: std::collections::BTreeMap<String, Vec<&MemoryItem>> =
            std::collections::BTreeMap::new();
        for (_, i) in &scored {
            if let Some(k) = &i.key {
                by_key.entry(k.clone()).or_default().push(i);
            }
        }
        let mut drop_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (k, items) in by_key {
            if items.len() > 1 && items.iter().any(|i| i.content != items[0].content) {
                let mut sorted = items.clone();
                sorted.sort_by_key(|i| core::cmp::Reverse(i.scope.precedence()));
                let winner = sorted[0];
                let losers: Vec<String> = sorted[1..]
                    .iter()
                    .filter(|l| l.content != winner.content)
                    .map(|l| l.id.clone())
                    .collect();
                if !losers.is_empty() {
                    drop_ids.extend(losers.iter().cloned());
                    conflicts.push(MemoryConflict {
                        key: k,
                        winner: winner.id.clone(),
                        losers,
                    });
                }
            }
        }
        let items: Vec<MemoryItem> = scored
            .into_iter()
            .map(|(_, i)| i)
            .filter(|i| !drop_ids.contains(&i.id))
            .take(limit)
            .collect();
        Ok(Retrieval { items, conflicts })
    }

    /// Sinal de correção repetida: a partir de [`SIGNAL_THRESHOLD`] evidências **distintas**, *propõe*
    /// (nunca ativa em User/Client).
    pub fn record_signal(
        &self,
        signal_key: &str,
        evidence: EvidenceRef,
        draft: MemoryDraft,
        run_id: Option<&str>,
    ) -> IntelResult<Option<MemoryItem>> {
        let key = format!("signal:{}", clean(signal_key, 80));
        let seen = self.project.memory_log(Some(&key)).map_err(store_err)?;
        if seen
            .iter()
            .any(|r| r.json["detail"] == json!(evidence.detail))
        {
            return Ok(None);
        }
        let _ = self.project.log_memory(
            &key,
            "signal",
            "agent",
            run_id,
            &json!({"kind": evidence.kind, "detail": evidence.detail}),
            now_ms(),
        );
        let n = self
            .project
            .memory_log(Some(&key))
            .map_err(store_err)?
            .len();
        if n >= SIGNAL_THRESHOLD {
            let mut d = draft;
            d.source = MemorySource::Correction;
            d.evidence.push(evidence);
            return self.propose(d, run_id, false).map(Some);
        }
        Ok(None)
    }

    pub fn audit(&self, id: Option<&str>) -> IntelResult<Vec<Value>> {
        Ok(self
            .project
            .memory_log(id)
            .map_err(store_err)?
            .into_iter()
            .map(|r| json!({"seq": r.seq, "memory_id": r.memory_id, "event": r.event, "actor": r.actor, "run_id": r.run_id, "ts_ms": r.ts_ms}))
            .collect())
    }
}
