//! Orchestrator da AI Run (PHASE5_ORCHESTRATOR): executa a máquina de estados, persiste cada
//! transição **atomicamente** (cursor + registro de stage + eventos numa transação, com CAS por
//! revisão), mantém o livro de efeitos (idempotência), orçamento (reserva/liquidação), decisões
//! humanas, pausa/cancelamento e a **retomada** depois de crash.
//!
//! O Orchestrator não edita a timeline diretamente, não escolhe provider fora do router, não lê
//! segredos, não pula aprovações e não chama nada fora do stage. Escrita só em `Edit`/`Correct`,
//! sempre por `preview → apply_plan` com o ator da Run.

use super::failpoint::fp;
use super::gateway::GatewayRegistry;
use super::generation::GenerationRegistry;
use super::machine::{
    IllegalTransition, Next, Outcome, RunErrorKind, RunStage, RunStatus, transition,
};
use super::memory::MemoryManager;
use super::model::{
    AiRun, ApprovalRecord, BudgetLimit, DecisionKind, DecisionOption, PendingDecision, RunBudget,
    RunErrorInfo, RunInputs, RunPolicy, RunUsage,
};
use super::roles::EffectCache;
use crate::ctx::IntelCtx;
use crate::engine::Engine;
use crate::error::{IntelError, IntelResult};
use crate::records::now_ms;
use capia_ai::CancelToken;
use capia_ai::dispatcher::{AiRuntime, TaskCtx};
use capia_store::{AppDb, AutonomyStore, Claim, RunRow, RunUpdate, StageRow, StoreErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type LiveSink = Arc<dyn Fn(Value) + Send + Sync>;

/// Dependências do Orchestrator (tudo opcional exceto engine/IA: sem Gateway/geração o app segue).
#[derive(Clone)]
pub struct Deps {
    pub engine: Arc<dyn Engine>,
    pub ai: Arc<AiRuntime>,
    pub gateways: Arc<GatewayRegistry>,
    pub generators: Arc<GenerationRegistry>,
    pub app_db: Option<Arc<AppDb>>,
    pub sink: LiveSink,
    /// Runtime onde rodam os drivers das Runs e as chamadas assíncronas de cancelamento.
    pub rt: tokio::runtime::Handle,
}

impl core::fmt::Debug for Deps {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Deps").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct Flags {
    pub cancel: CancelToken,
    pub pause: Arc<std::sync::atomic::AtomicBool>,
}

pub struct Orchestrator {
    pub(super) deps: Deps,
    pub(super) store: Arc<AutonomyStore>,
    pub(super) memory: MemoryManager,
    pub(super) project: PathBuf,
    pub(super) staging: PathBuf,
    pub(super) media_dir: PathBuf,
    flags: Mutex<HashMap<String, Flags>>,
    drivers: Mutex<std::collections::HashSet<String>>,
    seq: std::sync::atomic::AtomicU64,
}

impl core::fmt::Debug for Orchestrator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Orchestrator")
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

pub(crate) fn store_err(e: capia_store::StoreError) -> IntelError {
    if e.code == StoreErrorCode::StoreConflict {
        IntelError::new("RUN_CONFLICT", e.message)
    } else {
        IntelError::new("STORE_ERROR", e.to_string())
    }
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Resultado de um handler de stage.
#[derive(Debug)]
pub(super) struct StageResult {
    pub outcome: Outcome,
    pub pending: Option<PendingDecision>,
    pub events: Vec<(String, Value)>,
    pub output: Value,
}

impl StageResult {
    pub(super) fn ok(outcome: Outcome) -> Self {
        Self {
            outcome,
            pending: None,
            events: Vec::new(),
            output: Value::Null,
        }
    }

    pub(super) fn wait(d: PendingDecision) -> Self {
        Self {
            outcome: Outcome::NeedsUser,
            pending: Some(d),
            events: Vec::new(),
            output: Value::Null,
        }
    }
}

/// Classificação de retomada (PHASE5_ORCHESTRATOR §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeClass {
    SafeResume,
    Revalidate,
    NeedsUser,
    Irrecoverable,
    Terminal,
}

#[derive(Clone, Debug, Serialize)]
pub struct Recovered {
    pub run_id: String,
    pub stage: String,
    pub class: ResumeClass,
    pub detail: String,
}

pub fn classify_resume(run: &AiRun) -> ResumeClass {
    match run.status {
        RunStatus::Failed | RunStatus::Cancelled | RunStatus::Completed => ResumeClass::Terminal,
        RunStatus::WaitingUser => ResumeClass::NeedsUser,
        _ => match run.stage {
            // os tokens de preview vivem na memória do engine: sempre revalida antes de aplicar
            RunStage::ValidatePlan | RunStage::Edit | RunStage::Correct => ResumeClass::Revalidate,
            RunStage::Done => ResumeClass::Terminal,
            _ => ResumeClass::SafeResume,
        },
    }
}

impl Orchestrator {
    pub fn open(deps: Deps, project: PathBuf) -> IntelResult<Arc<Self>> {
        let store =
            Arc::new(AutonomyStore::open(&project, Duration::from_secs(10)).map_err(store_err)?);
        let memory = MemoryManager::new(store.clone(), deps.app_db.clone());
        let mut staging = project.clone().into_os_string();
        staging.push("-cache");
        let staging = PathBuf::from(staging).join("autonomy").join("staging");
        let mut media_dir = project.clone().into_os_string();
        media_dir.push("-media");
        let media_dir = PathBuf::from(media_dir).join("ai");
        Ok(Arc::new(Self {
            deps,
            store,
            memory,
            project,
            staging,
            media_dir,
            flags: Mutex::new(HashMap::new()),
            drivers: Mutex::new(std::collections::HashSet::new()),
            seq: std::sync::atomic::AtomicU64::new(1),
        }))
    }

    pub fn store(&self) -> &Arc<AutonomyStore> {
        &self.store
    }

    pub fn memory(&self) -> &MemoryManager {
        &self.memory
    }

    pub fn project_path(&self) -> &PathBuf {
        &self.project
    }

    // ---- ids / eventos ------------------------------------------------------------------------

    fn new_run_id(&self) -> String {
        let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut h = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(
            &mut h,
            format!("{}:{}:{n}", self.project.display(), now_ms()).as_bytes(),
        );
        let d = sha2::Digest::finalize(h);
        format!(
            "run-{}",
            d[..5]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }

    /// Evento durável + espelho ao vivo (a UI reconecta por snapshot + `events_after`).
    pub(super) fn emit(&self, run_id: &str, kind: &str, data: Value) {
        let _ = self.store.append_event(run_id, kind, &data, now_ms());
        self.live(run_id, kind, &data);
    }

    pub(super) fn live(&self, run_id: &str, kind: &str, data: &Value) {
        (self.deps.sink)(json!({"kind": "ai_run", "run_id": run_id, "event": kind, "data": data}));
    }

    // ---- persistência -------------------------------------------------------------------------

    fn parse(row: &RunRow) -> IntelResult<AiRun> {
        let mut run: AiRun = serde_json::from_value(row.json.clone()).map_err(|e| {
            IntelError::new(
                "RUN_CORRUPTED",
                format!("run {} is unreadable: {e}", row.run_id),
            )
        })?;
        // o cursor autoritativo é o da linha (a coluna é o que o CAS protege)
        run.revision = row.revision;
        run.status = RunStatus::parse(&row.status)
            .ok_or_else(|| IntelError::new("RUN_CORRUPTED", "unknown run status"))?;
        run.stage = RunStage::parse(&row.stage)
            .ok_or_else(|| IntelError::new("RUN_CORRUPTED", "unknown run stage"))?;
        Ok(run)
    }

    pub fn load(&self, run_id: &str) -> IntelResult<AiRun> {
        let row = self
            .store
            .get_run(run_id)
            .map_err(store_err)?
            .ok_or_else(|| IntelError::new("RUN_NOT_FOUND", "unknown run"))?;
        Self::parse(&row)
    }

    pub fn list(&self, limit: u32) -> IntelResult<Vec<Value>> {
        let rows = self.store.list_runs(None, limit).map_err(store_err)?;
        Ok(rows
            .iter()
            .map(|r| match Self::parse(r) {
                Ok(run) => self.summary(&run),
                Err(e) => json!({"id": r.run_id, "status": "corrupted", "resume_class": ResumeClass::Irrecoverable, "error": e.message}),
            })
            .collect())
    }

    pub fn summary(&self, run: &AiRun) -> Value {
        let u = self.usage_from_ledger(&run.id, &run.usage);
        json!({
            "id": run.id, "status": run.status, "stage": run.stage, "revision": run.revision,
            "created_ms": run.created_ms, "updated_ms": run.updated_ms, "completed_ms": run.completed_ms,
            "usage": u, "budget": run.budget, "pending": run.pending, "error": run.error,
            "parent_run_id": run.parent_run_id, "variant_group_id": run.variant_group_id,
            "deliverables": run.production_plan.as_ref().map(|p| p.deliverables.iter().map(|d| d.key.clone()).collect::<Vec<_>>()),
            "sequences": run.sequences, "resume_class": classify_resume(run),
        })
    }

    /// Snapshot completo (UI reconecta por aqui; planos/reviews/decisões inclusos).
    pub fn snapshot(&self, run_id: &str) -> IntelResult<Value> {
        let run = self.load(run_id)?;
        let stages = self.store.list_stages(run_id).map_err(store_err)?;
        let effects = self.store.effects_for_run(run_id).map_err(store_err)?;
        let prov = self
            .store
            .list_provenance(Some(run_id))
            .map_err(store_err)?;
        let last_event = self
            .store
            .events_after(run_id, 0, 5_000)
            .map_err(store_err)?
            .last()
            .map_or(0, |e| e.seq);
        Ok(json!({
            "run": run, "usage": self.usage_from_ledger(run_id, &run.usage),
            "stages": stages, "effects": effects.iter().map(|e| json!({"key": e.effect_key, "kind": e.kind, "state": e.state, "external_id": e.external_id})).collect::<Vec<_>>(),
            "provenance": prov, "last_event_seq": last_event, "resume_class": classify_resume(&run),
        }))
    }

    pub fn events_after(&self, run_id: &str, after: u64) -> IntelResult<Vec<Value>> {
        Ok(self
            .store
            .events_after(run_id, after, 500)
            .map_err(store_err)?
            .into_iter()
            .map(|e| json!({"seq": e.seq, "kind": e.kind, "ts_ms": e.ts_ms, "data": e.json}))
            .collect())
    }

    /// Uso **autoritativo**: o livro-razão (cada chamada paga é lançada quando acontece).
    pub fn usage_from_ledger(&self, run_id: &str, base: &RunUsage) -> RunUsage {
        let mut u = base.clone();
        u.cost_micros = 0;
        u.tokens = 0;
        u.provider_calls = 0;
        u.generations = 0;
        u.unknown_cost_calls = 0;
        for r in self.store.ledger_rows(run_id).unwrap_or_default() {
            if r.kind != "settle" {
                continue;
            }
            u.cost_micros += r.micros;
            match r.json["kind"].as_str() {
                Some("llm") => {
                    u.provider_calls += 1;
                    u.tokens += r.json["tokens"].as_u64().unwrap_or(0);
                    if r.json["unknown"].as_bool() == Some(true) {
                        u.unknown_cost_calls += 1;
                    }
                }
                Some("generation") => u.generations += 1,
                _ => {}
            }
        }
        u
    }

    fn row_update(run: &AiRun) -> RunUpdate {
        RunUpdate {
            status: run.status.as_str().to_owned(),
            stage: run.stage.as_str().to_owned(),
            json: serde_json::to_value(run).unwrap_or(Value::Null),
        }
    }

    /// Persiste o estado da Run (CAS) e devolve a nova revisão. `events` entram na MESMA transação.
    pub(super) fn save(
        &self,
        run: &mut AiRun,
        stage_row: Option<&StageRow>,
        events: &[(String, Value)],
    ) -> IntelResult<()> {
        run.updated_ms = now_ms();
        let rev = self
            .store
            .advance(
                &run.id,
                run.revision,
                &Self::row_update(run),
                stage_row,
                events,
                now_ms(),
            )
            .map_err(store_err)?;
        run.revision = rev;
        for (k, v) in events {
            self.live(&run.id, k, v);
        }
        Ok(())
    }

    /// Muta uma Run por fora do driver (API: decidir, pausar, cancelar) com CAS e novas tentativas.
    pub(super) fn mutate<R>(
        &self,
        run_id: &str,
        mut f: impl FnMut(&mut AiRun) -> IntelResult<(R, Vec<(String, Value)>)>,
    ) -> IntelResult<R> {
        for _ in 0..8 {
            let mut run = self.load(run_id)?;
            let (out, events) = f(&mut run)?;
            match self.save(&mut run, None, &events) {
                Ok(()) => return Ok(out),
                Err(e) if e.code == "RUN_CONFLICT" => std::thread::sleep(Duration::from_millis(15)),
                Err(e) => return Err(e),
            }
        }
        Err(IntelError::new(
            "RUN_CONFLICT",
            "the run keeps changing; try again",
        ))
    }

    // ---- criação ------------------------------------------------------------------------------

    pub fn create_run(
        &self,
        inputs: RunInputs,
        policy: Option<RunPolicy>,
        budget: Option<RunBudget>,
        profile_id: &str,
        parent: Option<(&str, Option<&str>)>,
    ) -> IntelResult<AiRun> {
        let id = self.new_run_id();
        let mut run = AiRun::new(
            id,
            self.project.display().to_string(),
            inputs,
            profile_id.to_owned(),
            now_ms(),
        );
        if let Some(p) = policy {
            run.policy = p;
        }
        if let Some(b) = budget {
            run.budget = b;
        }
        if let Some((parent_id, group)) = parent {
            run.parent_run_id = Some(parent_id.to_owned());
            run.variant_group_id =
                Some(group.map_or_else(|| format!("vg-{parent_id}"), str::to_owned));
        }
        self.store
            .create_run(
                &run.id,
                run.status.as_str(),
                run.stage.as_str(),
                run.parent_run_id.as_deref(),
                run.variant_group_id.as_deref(),
                &serde_json::to_value(&run).unwrap_or(Value::Null),
                now_ms(),
            )
            .map_err(store_err)?;
        self.emit(
            &run.id,
            "run_created",
            json!({"run_id": run.id, "parent": run.parent_run_id}),
        );
        Ok(run)
    }

    /// "Rerun as new": nova Run (novos ids/operation ids; nunca reaproveita `plan_token`), reaproveitando
    /// caches determinísticos (transcrições/gramática/DemandSpec ficam nos `ai_records` do projeto).
    pub fn duplicate(
        &self,
        run_id: &str,
        brief: Option<String>,
        budget: Option<RunBudget>,
        policy: Option<RunPolicy>,
    ) -> IntelResult<AiRun> {
        let src = self.load(run_id)?;
        let mut inputs = src.inputs.clone();
        if let Some(b) = brief {
            inputs.brief_text = Some(b);
            inputs.demand_spec_id = None;
        }
        let run = self.create_run(
            inputs,
            Some(policy.unwrap_or(src.policy.clone())),
            Some(budget.unwrap_or(src.budget.clone())),
            &src.brain_profile_id,
            Some((&src.id, src.variant_group_id.as_deref())),
        )?;
        Ok(run)
    }

    // ---- bandeiras de execução ------------------------------------------------------------------

    pub(super) fn flags_for(&self, run_id: &str) -> Flags {
        lock(&self.flags)
            .entry(run_id.to_owned())
            .or_insert_with(|| Flags {
                cancel: CancelToken::new(),
                pause: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })
            .clone()
    }

    fn reset_flags(&self, run_id: &str) {
        lock(&self.flags).remove(run_id);
    }

    pub fn is_driving(&self, run_id: &str) -> bool {
        lock(&self.drivers).contains(run_id)
    }

    // ---- livro de efeitos / cache de papéis -----------------------------------------------------

    pub(super) fn effect_cache<'a>(&'a self, run_id: &'a str) -> LedgerCache<'a> {
        LedgerCache { o: self, run_id }
    }

    /// Orçamento de uma chamada de papel, **lançado quando acontece** (não no fim do stage).
    fn ledger_llm(&self, run_id: &str, key: &str, meta: &super::roles::RoleMeta) {
        let micros = meta.cost_micros.unwrap_or(0);
        let unknown = meta.cost_micros.is_none();
        let _ = self
            .store
            .ledger_reserve(run_id, key, 0, None, &json!({"kind": "llm"}), now_ms());
        let _ = self.store.ledger_settle(
            run_id,
            key,
            micros,
            &json!({"kind": "llm", "tokens": meta.tokens, "unknown": unknown, "model": meta.model_id}),
            now_ms(),
        );
    }

    // ---- loop principal -------------------------------------------------------------------------

    /// Dirige a Run até ela precisar de alguém (decisão/pausa) ou terminar. Idempotente: se já há
    /// um driver para esta Run, não faz nada.
    pub async fn drive(self: &Arc<Self>, run_id: String) {
        if !lock(&self.drivers).insert(run_id.clone()) {
            return;
        }
        // uma Run que recomeça a ser dirigida começa sem pedido de pausa/cancelamento antigo
        self.reset_flags(&run_id);
        let flags = self.flags_for(&run_id);
        let started = Instant::now();
        while let Ok(run) = self.load(&run_id) {
            if matches!(run.status, RunStatus::Pending) {
                let mut r = run;
                r.status = RunStatus::Running;
                r.started_ms.get_or_insert(now_ms());
                let st = r.stage;
                if self
                    .save(
                        &mut r,
                        None,
                        &[("run_started".into(), json!({"stage": st}))],
                    )
                    .is_err()
                {
                    continue;
                }
                continue;
            }
            if run.status != RunStatus::Running {
                break;
            }
            if flags.cancel.is_cancelled() {
                let _ = self.finish_cancelled(&run_id);
                break;
            }
            if flags.pause.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = self.apply_pause(&run_id);
                break;
            }
            match self.step(run, &flags).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) if e.code == "RUN_CONFLICT" => {}
                Err(e) if e.code == "FAILPOINT" => break,
                Err(e) => {
                    let _ = self.fail(&run_id, e);
                    break;
                }
            }
            if started.elapsed() > Duration::from_secs(6 * 3600) {
                break;
            }
        }
        self.reset_flags(&run_id);
        lock(&self.drivers).remove(&run_id);
    }

    fn finish_cancelled(&self, run_id: &str) -> IntelResult<()> {
        self.cancel_external(run_id);
        self.mutate(run_id, |r| {
            if r.status.is_terminal() {
                return Ok(((), vec![]));
            }
            r.status = RunStatus::Cancelled;
            r.pending = None;
            r.completed_ms = Some(now_ms());
            Ok((
                (),
                vec![("run_cancelled".into(), json!({"stage": r.stage}))],
            ))
        })
    }

    fn apply_pause(&self, run_id: &str) -> IntelResult<()> {
        self.mutate(run_id, |r| {
            if r.status != RunStatus::Running {
                return Ok(((), vec![]));
            }
            r.status = RunStatus::Paused;
            r.resume_stage = Some(r.stage);
            Ok(((), vec![("run_paused".into(), json!({"stage": r.stage}))]))
        })
    }

    fn fail(&self, run_id: &str, e: IntelError) -> IntelResult<()> {
        let kind = classify_error(&e);
        self.mutate(run_id, |r| {
            if r.status.is_terminal() {
                return Ok(((), vec![]));
            }
            r.status = RunStatus::Failed;
            r.completed_ms = Some(now_ms());
            r.error = Some(RunErrorInfo {
                kind,
                code: e.code.clone(),
                message: e.message.clone(),
                stage: r.stage,
                recoverable: kind.recoverable(),
            });
            Ok((
                (),
                vec![(
                    "run_failed".into(),
                    json!({"code": e.code, "message": e.message, "stage": r.stage}),
                )],
            ))
        })
    }

    /// Cancela efeitos externos em voo (geração/imports) — melhor esforço, nunca bloqueia.
    fn cancel_external(&self, run_id: &str) {
        let this_store = self.store.clone();
        let gens = self.deps.generators.clone();
        let engine = self.deps.engine.clone();
        let run = run_id.to_owned();
        self.deps.rt.spawn(async move {
            Self::cancel_external_async(this_store, gens, engine, run).await;
        });
    }

    async fn cancel_external_async(
        store: Arc<AutonomyStore>,
        gens: Arc<GenerationRegistry>,
        engine: Arc<dyn Engine>,
        run_id: String,
    ) {
        for e in store.effects_for_run(&run_id).unwrap_or_default() {
            if e.kind == "generation"
                && e.state == "submitted"
                && let (Some(job), Some(pid)) = (e.external_id.clone(), e.json["provider"].as_str())
                && let Some(p) = gens.get(pid)
            {
                let _ = p.cancel(&job).await;
                let _ = store.update_effect(
                    &e.effect_key,
                    "failed",
                    None,
                    Some(&json!({"cancelled": true, "provider": pid})),
                    now_ms(),
                );
            }
            if e.kind == "import"
                && e.state == "submitted"
                && let Some(t) = e.external_id.clone()
            {
                let _ = engine.import_cancel(&t);
            }
        }
    }

    /// Um passo: executa o stage atual, decide a transição pela **tabela** e persiste tudo junto.
    /// `Ok(true)` ⇒ continuar o laço; `Ok(false)` ⇒ parar (espera/terminal).
    async fn step(self: &Arc<Self>, mut run: AiRun, flags: &Flags) -> IntelResult<bool> {
        let t0 = Instant::now();
        let stage = run.stage;
        // limite de orçamento já atingido (custo/tokens/chamadas/tempo): para e pede decisão
        if let Some(limit) = run
            .budget
            .exceeded(&self.usage_from_ledger(&run.id, &run.usage))
        {
            let d = self.budget_decision(&run, limit);
            return self.park_for_decision(run, d).await;
        }
        let seq = self.store.next_stage_seq(&run.id).map_err(store_err)?;
        let key = format!("{}:{}:{}", run.id, stage.as_str(), run.current_attempt);
        let input_digest = super::plan::digest_value(&json!({
            "stage": stage, "attempt": run.current_attempt, "plan": run.production_plan.as_ref().map(super::plan::ProductionPlan::digest),
            "revision": run.revision,
        }));
        let existing = self.store.stage_by_key(&key).map_err(store_err)?;
        let seq = existing.as_ref().map_or(seq, |s| s.seq);
        let started_ms = existing.as_ref().map_or_else(now_ms, |s| s.started_ms);
        let attempt_label = existing.as_ref().map_or(0, |s| s.attempt + 1);
        let started_row = StageRow {
            run_id: run.id.clone(),
            seq,
            stage: stage.as_str().to_owned(),
            attempt: attempt_label,
            idem_key: key.clone(),
            input_digest: input_digest.clone(),
            output_digest: None,
            status: "started".into(),
            started_ms,
            ended_ms: None,
            json: json!({"retry": attempt_label > 0}),
        };
        self.store.put_stage(&started_row).map_err(store_err)?;
        self.emit(
            &run.id,
            "stage_started",
            json!({"stage": stage, "attempt": run.current_attempt}),
        );
        fp!("autonomy_stage_started");

        let ic = {
            let mut ic = IntelCtx::new(
                self.deps.engine.clone(),
                self.deps.ai.clone(),
                self.profile_for(&run),
            );
            ic.actor = capia_commands::Actor::agent(run.actor_id());
            ic
        };
        let mut task = TaskCtx::new(format!("{}:{}", run.id, stage.as_str()), &ic.profile);
        task.cancel = flags.cancel.clone();
        let res = tokio::select! {
            () = flags.cancel.cancelled() => Err(IntelError::cancelled()),
            r = self.run_stage(&mut run, &ic, &task, flags) => r,
        };
        run.usage = self.usage_from_ledger(&run.id, &run.usage);
        run.usage.wall_time_ms += u64::try_from(t0.elapsed().as_millis()).unwrap_or(0);

        let result = match res {
            Ok(r) => r,
            Err(e) if e.code == "FAILPOINT" || e.code == "RUN_CONFLICT" => return Err(e),
            Err(e) if e.is_cancelled() => {
                let _ = self.finish_cancelled(&run.id);
                return Ok(false);
            }
            Err(e) => match self.handle_stage_error(&mut run, stage, &e).await? {
                Some(r) => r,
                None => return Ok(true), // retry
            },
        };

        fp!("autonomy_stage_output_before_persist");
        let next =
            transition(stage, result.outcome).map_err(|IllegalTransition { from, outcome }| {
                IntelError::new(
                    "ILLEGAL_TRANSITION",
                    format!("{} --{outcome:?}-->", from.as_str()),
                )
            })?;
        let mut events = result.events.clone();
        let out_digest = super::plan::digest_value(&result.output);
        match next {
            Next::Go { stage: to } => {
                run.stage = to;
                run.current_attempt += 1;
                run.status = RunStatus::Running;
                run.pending = None;
                run.resume_stage = None;
                events.push((
                    "stage_completed".into(),
                    json!({"stage": stage, "outcome": result.outcome, "next": to}),
                ));
            }
            Next::Wait { resume } => {
                run.status = RunStatus::WaitingUser;
                run.resume_stage = Some(resume);
                run.pending = result.pending.clone();
                if let Some(p) = &run.pending {
                    events.push(("approval_required".into(), json!({"decision": p})));
                }
                events.push((
                    "stage_completed".into(),
                    json!({"stage": stage, "outcome": result.outcome, "waiting": true}),
                ));
            }
            Next::Complete => {
                run.stage = RunStage::Done;
                run.status = RunStatus::Completed;
                run.completed_ms = Some(now_ms());
                run.pending = None;
                run.report = Some(self.final_report(&run));
                events.push(("run_done".into(), json!({"report": run.report})));
            }
            Next::Fail => {
                run.status = RunStatus::Failed;
                run.completed_ms = Some(now_ms());
                events.push((
                    "run_failed".into(),
                    json!({"stage": stage, "error": run.error}),
                ));
            }
            Next::Cancelled => {
                run.status = RunStatus::Cancelled;
                run.completed_ms = Some(now_ms());
                events.push(("run_cancelled".into(), json!({"stage": stage})));
            }
        }
        let done_row = StageRow {
            status: "completed".into(),
            output_digest: Some(out_digest),
            ended_ms: Some(now_ms()),
            json: json!({"outcome": result.outcome, "next": next, "output": result.output}),
            ..started_row
        };
        events.push(("cost_updated".into(), json!({"usage": run.usage})));
        self.save(&mut run, Some(&done_row), &events)?;
        fp!("autonomy_stage_after_persist");
        Ok(matches!(next, Next::Go { .. }))
    }

    async fn park_for_decision(
        self: &Arc<Self>,
        mut run: AiRun,
        d: PendingDecision,
    ) -> IntelResult<bool> {
        run.status = RunStatus::WaitingUser;
        run.resume_stage = Some(run.stage);
        run.pending = Some(d.clone());
        self.save(
            &mut run,
            None,
            &[("approval_required".into(), json!({"decision": d}))],
        )?;
        Ok(false)
    }

    pub(super) fn budget_decision(&self, run: &AiRun, limit: BudgetLimit) -> PendingDecision {
        let u = self.usage_from_ledger(&run.id, &run.usage);
        PendingDecision {
            id: format!("dec-{}-budget-{}", run.id, run.current_attempt),
            kind: DecisionKind::BudgetExtension,
            question: format!("The run reached its {limit:?} limit. Extend it or stop?"),
            options: vec![
                DecisionOption::new("extend", "Extend the limit and continue"),
                DecisionOption::new("stop", "Stop the run"),
            ],
            context: json!({"limit": limit, "usage": u, "budget": run.budget}),
            consequences:
                "Continuing may spend more money/time; stopping keeps everything done so far."
                    .into(),
            default_option: Some("stop".into()),
            expires_ms: None,
            bound_digest: None,
            resume_stage: run.stage,
            created_ms: now_ms(),
        }
    }

    fn profile_for(&self, run: &AiRun) -> capia_ai::brain::BrainProfile {
        let reg = self.deps.ai.registry();
        reg.profiles
            .get(&run.brain_profile_id)
            .cloned()
            .or_else(|| reg.active().cloned())
            .unwrap_or_else(|| capia_ai::brain::BrainProfile::new("none", "none", ""))
    }

    /// Erro de stage: retenta o que é transitório (limitado); o resto vira a transição sugerida.
    async fn handle_stage_error(
        &self,
        run: &mut AiRun,
        stage: RunStage,
        e: &IntelError,
    ) -> IntelResult<Option<StageResult>> {
        let kind = classify_error(e);
        let retries = run.checkpoint["retries"][stage.as_str()]
            .as_u64()
            .unwrap_or(0);
        if kind.retryable() && is_transient(e) && retries < 2 {
            if !run.checkpoint.is_object() {
                run.checkpoint = json!({});
            }
            run.checkpoint["retries"][stage.as_str()] = json!(retries + 1);
            let mut r = run.clone();
            self.save(
                &mut r,
                None,
                &[(
                    "stage_retry".into(),
                    json!({"stage": stage, "code": e.code, "retry": retries + 1}),
                )],
            )?;
            *run = r;
            tokio::time::sleep(Duration::from_millis(150 * (1 << retries))).await;
            return Ok(None);
        }
        run.error = Some(RunErrorInfo {
            kind,
            code: e.code.clone(),
            message: e.message.clone(),
            stage,
            recoverable: kind.recoverable(),
        });
        let mut outcome = kind.suggested();
        // BudgetExceeded do provider/router = pedido de decisão, não falha cega
        if transition(stage, outcome).is_err() {
            outcome = Outcome::Failure;
        }
        if outcome == Outcome::NeedsUser {
            let d = self.budget_decision(run, BudgetLimit::Cost);
            return Ok(Some(StageResult::wait(d)));
        }
        Ok(Some(StageResult::ok(outcome)))
    }

    fn final_report(&self, run: &AiRun) -> Value {
        let u = self.usage_from_ledger(&run.id, &run.usage);
        let prov = self
            .store
            .list_provenance(Some(&run.id))
            .unwrap_or_default();
        json!({
            "deliverables": run.sequences, "cost_micros": u.cost_micros, "unknown_cost_calls": u.unknown_cost_calls,
            "tokens": u.tokens, "provider_calls": u.provider_calls, "generations": u.generations,
            "review_loops": run.usage.review_loops, "replans": run.usage.replans,
            "transactions": run.applied, "provenance_records": prov.len(),
            "reviews": run.reviews, "approvals": run.approvals.len(),
        })
    }

    // ---- API: pausa / retomada / cancelamento / decisão -----------------------------------------

    pub fn pause(&self, run_id: &str) -> IntelResult<()> {
        let flags = self.flags_for(run_id);
        flags.pause.store(true, std::sync::atomic::Ordering::SeqCst);
        if !self.is_driving(run_id) {
            self.apply_pause(run_id)?;
            self.reset_flags(run_id);
        }
        Ok(())
    }

    /// Cancela: aborta chamadas em voo (o token chega aos providers), cancela jobs externos e
    /// **impede** qualquer apply tardio (o EDIT relê o estado antes de aplicar).
    pub fn cancel(&self, run_id: &str) -> IntelResult<()> {
        let run = self.load(run_id)?;
        if run.status.is_terminal() {
            return Ok(());
        }
        let flags = self.flags_for(run_id);
        flags.cancel.cancel();
        // persiste já: o driver (se houver) vê o status e para; nada continua em segundo plano
        self.finish_cancelled(run_id)?;
        if !self.is_driving(run_id) {
            self.reset_flags(run_id);
        }
        Ok(())
    }

    /// `Paused → Running` (e agenda o driver).
    pub fn resume(self: &Arc<Self>, run_id: &str) -> IntelResult<AiRun> {
        let run = self.mutate(run_id, |r| match r.status {
            RunStatus::Paused | RunStatus::Pending => {
                r.status = RunStatus::Running;
                if let Some(s) = r.resume_stage.take() {
                    r.stage = s;
                }
                Ok((
                    r.clone(),
                    vec![("run_resumed".into(), json!({"stage": r.stage}))],
                ))
            }
            RunStatus::Running => Ok((r.clone(), vec![])),
            _ => Err(IntelError::new(
                "INVALID_STATE",
                format!(
                    "a {} run cannot be resumed (waiting runs need a decision)",
                    r.status.as_str()
                ),
            )),
        })?;
        self.spawn_driver(run_id);
        Ok(run)
    }

    pub fn start(self: &Arc<Self>, run_id: &str) -> IntelResult<()> {
        let run = self.load(run_id)?;
        if !matches!(run.status, RunStatus::Pending | RunStatus::Running) {
            return Err(IntelError::new(
                "INVALID_STATE",
                "only a pending run can be started",
            ));
        }
        self.spawn_driver(run_id);
        Ok(())
    }

    pub fn spawn_driver(self: &Arc<Self>, run_id: &str) {
        let this = self.clone();
        let id = run_id.to_owned();
        self.deps.rt.spawn(async move { this.drive(id).await });
    }

    /// Resolve a decisão pendente. Decisão velha/inválida é recusada; a aprovação fica **presa** ao
    /// digest do plano — mudou, não vale. Tudo vira `ApprovalRecord` auditável.
    pub fn decide(
        self: &Arc<Self>,
        run_id: &str,
        decision_id: &str,
        option: &str,
        payload: &Value,
        actor: &str,
    ) -> IntelResult<AiRun> {
        let run = self.mutate(run_id, |r| {
            if r.status != RunStatus::WaitingUser {
                return Err(IntelError::new(
                    "INVALID_STATE",
                    "the run is not waiting for a decision",
                ));
            }
            let Some(p) = r.pending.clone() else {
                return Err(IntelError::new("INVALID_STATE", "no pending decision"));
            };
            if p.id != decision_id {
                return Err(IntelError::new(
                    "STALE_DECISION",
                    "this decision is not the pending one anymore",
                ));
            }
            if let Some(bound) = &p.bound_digest
                && Some(bound.as_str()) != self.current_bound_digest(r, p.kind).as_deref()
            {
                return Err(IntelError::new(
                    "STALE_DECISION",
                    "the plan changed after this decision was requested",
                ));
            }
            let free_text = p.kind == DecisionKind::OpenQuestion && option == "answer";
            if !free_text && !p.options.iter().any(|o| o.id == option) {
                return Err(IntelError::new(
                    "INVALID_ARGUMENT",
                    format!("`{option}` is not an option of this decision"),
                ));
            }
            let mut events = vec![(
                "decision_made".into(),
                json!({"decision": p.id, "kind": p.kind, "option": option}),
            )];
            r.approvals.push(ApprovalRecord {
                decision_id: p.id.clone(),
                kind: p.kind,
                option: option.to_owned(),
                actor: actor.to_owned(),
                at_ms: now_ms(),
                bound_digest: p.bound_digest.clone(),
                detail: payload.clone(),
            });
            self.apply_decision(r, &p, option, payload, &mut events)?;
            r.pending = None;
            Ok((r.clone(), events))
        })?;
        if run.status == RunStatus::Running {
            self.spawn_driver(run_id);
        }
        Ok(run)
    }

    /// Decisão do usuário sobre um achado do Critic: `ignore` | `lock` | `reopen`. O Critic não
    /// insiste nos próximos ciclos em achados ignorados/travados.
    pub fn review_decision(
        &self,
        run_id: &str,
        finding_key: &str,
        decision: &str,
    ) -> IntelResult<AiRun> {
        if !matches!(decision, "ignore" | "lock" | "reopen") {
            return Err(IntelError::new(
                "INVALID_ARGUMENT",
                "decision must be ignore, lock or reopen",
            ));
        }
        self.mutate(run_id, |r| {
            if !r.checkpoint.is_object() {
                r.checkpoint = json!({});
            }
            for list in ["ignored", "locked"] {
                let arr = r.checkpoint["review_decisions"][list]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let mut set: Vec<Value> = arr
                    .into_iter()
                    .filter(|v| v.as_str() != Some(finding_key))
                    .collect();
                if (list == "ignored" && decision == "ignore")
                    || (list == "locked" && decision == "lock")
                {
                    set.push(json!(finding_key));
                }
                r.checkpoint["review_decisions"][list] = Value::Array(set);
            }
            Ok((
                r.clone(),
                vec![(
                    "review_decision".into(),
                    json!({"finding": finding_key, "decision": decision}),
                )],
            ))
        })
    }

    /// Assets trazidos/gerados pela Run e se ainda são usados em alguma sequence — sugestão de
    /// limpeza depois de desfazer/cancelar (nunca apaga automaticamente).
    pub fn cleanup_candidates(&self, run_id: &str) -> IntelResult<Vec<Value>> {
        let prov = self
            .store
            .list_provenance(Some(run_id))
            .map_err(store_err)?;
        let snap = self.deps.engine.read("project.snapshot", json!({}))?;
        let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for s in snap["sequences"].as_array().into_iter().flatten() {
            if let Some(id) = s["id"].as_str()
                && let Ok(seq) = self
                    .deps
                    .engine
                    .read("sequence.get", json!({"sequence": id}))
            {
                for c in seq["clips"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.values())
                {
                    if let Some(a) = c["content"]["asset"].as_str() {
                        used.insert(a.to_owned());
                    }
                }
            }
        }
        Ok(prov
            .into_iter()
            .filter(|p| matches!(p.kind.as_str(), "downloaded" | "generated" | "library"))
            .map(|p| json!({"asset_id": p.asset_id, "kind": p.kind, "used": used.contains(&p.asset_id), "unused_suggestion": !used.contains(&p.asset_id)}))
            .collect())
    }

    // ---- recuperação ----------------------------------------------------------------------------

    /// Ao abrir o projeto: stages `started` viram `interrupted`; Runs `running` viram `paused`
    /// (resumíveis) e cada uma é **classificada**; Runs ilegíveis são listadas como irrecuperáveis.
    pub fn recover(&self) -> IntelResult<Vec<Recovered>> {
        self.store
            .interrupt_started_stages(now_ms())
            .map_err(store_err)?;
        let mut out = Vec::new();
        for row in self.store.list_runs(None, 1_000).map_err(store_err)? {
            let run = match Self::parse(&row) {
                Ok(r) => r,
                Err(e) => {
                    out.push(Recovered {
                        run_id: row.run_id.clone(),
                        stage: row.stage.clone(),
                        class: ResumeClass::Irrecoverable,
                        detail: e.message,
                    });
                    continue;
                }
            };
            if matches!(run.status, RunStatus::Running | RunStatus::Pending)
                && run.status == RunStatus::Running
            {
                self.mutate(&run.id, |r| {
                    if r.status == RunStatus::Running {
                        r.status = RunStatus::Paused;
                        r.resume_stage = Some(r.stage);
                    }
                    Ok((
                        (),
                        vec![(
                            "run_interrupted".into(),
                            json!({"stage": r.stage, "reason": "process_restart"}),
                        )],
                    ))
                })?;
            }
            let run = self.load(&run.id)?;
            let class = classify_resume(&run);
            if class != ResumeClass::Terminal {
                out.push(Recovered {
                    run_id: run.id.clone(),
                    stage: run.stage.as_str().to_owned(),
                    class,
                    detail: format!("{:?}", run.status),
                });
            }
        }
        Ok(out)
    }
}

impl core::fmt::Debug for LedgerCache<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LedgerCache")
            .field("run", &self.run_id)
            .finish_non_exhaustive()
    }
}

/// Cache de papéis sobre o livro de efeitos da Run (`llm:*`). Grava o custo **no momento** da
/// chamada (livro-razão), não no fim do stage.
pub(super) struct LedgerCache<'a> {
    o: &'a Orchestrator,
    run_id: &'a str,
}

impl EffectCache for LedgerCache<'_> {
    fn load(&self, key: &str) -> Option<Value> {
        let e = self.o.store.get_effect(key).ok().flatten()?;
        (e.state == "done").then_some(e.json)
    }

    fn save(&self, key: &str, value: &Value) {
        let claim = self
            .o
            .store
            .claim_effect(key, self.run_id, "llm", &Value::Null, now_ms());
        if matches!(claim, Ok(Claim::New(_) | Claim::Existing(_))) {
            let _ = self
                .o
                .store
                .update_effect(key, "done", None, Some(value), now_ms());
        }
        if let Ok(meta) = serde_json::from_value::<super::roles::RoleMeta>(value["meta"].clone())
            && !meta.cache_hit
        {
            self.o.ledger_llm(self.run_id, key, &meta);
        }
    }
}

/// Mapeia o erro estruturado para a taxonomia de Run (PHASE5_ORCHESTRATOR §23).
pub fn classify_error(e: &IntelError) -> RunErrorKind {
    let c = e.code.as_str();
    match c {
        "CANCELLED" => RunErrorKind::Cancelled,
        "STORE_ERROR" | "RUN_CORRUPTED" | "RUN_CONFLICT" => RunErrorKind::Persistence,
        "NOT_ALLOWED" | "PERMISSION_DENIED" | "PREVIEW_REQUIRED" | "PLAN_TOKEN_INVALID" => {
            RunErrorKind::Permission
        }
        "BUDGET_EXCEEDED" => RunErrorKind::BudgetExceeded,
        "PLAN_STATE_CHANGED" | "CONFLICT" | "OVERLAP" => RunErrorKind::Conflict,
        _ if c.starts_with("PLAN_") => RunErrorKind::PlanInvalid,
        _ if c.starts_with("GATEWAY_") => RunErrorKind::Gateway,
        _ if c.starts_with("GEN_") => RunErrorKind::Generation,
        _ if c.starts_with("ASSET_") => RunErrorKind::AssetUnavailable,
        "RATE_LIMITED"
        | "PROVIDER_TIMEOUT"
        | "PROVIDER_UNAVAILABLE"
        | "AUTH_FAILED"
        | "NO_CAPABLE_MODEL"
        | "STRUCTURED_OUTPUT_INVALID"
        | "INVALID_PROVIDER_RESPONSE"
        | "PRIVACY_POLICY_BLOCKED"
        | "NOT_CONFIGURED" => RunErrorKind::Provider,
        "SECURITY" => RunErrorKind::Security,
        _ => RunErrorKind::Internal,
    }
}

/// Transitório (vale tentar de novo): 429/timeout/indisponível. Auth/permissão/plano inválido/
/// orçamento **não**.
pub fn is_transient(e: &IntelError) -> bool {
    matches!(
        e.code.as_str(),
        "RATE_LIMITED"
            | "PROVIDER_TIMEOUT"
            | "PROVIDER_UNAVAILABLE"
            | "GATEWAY_TRANSIENT"
            | "GEN_TRANSIENT"
    )
}
