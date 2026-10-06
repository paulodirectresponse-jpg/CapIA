//! `IntelligenceService`: a **única** superfície `ai.*` da Fase 4, hospedada pelos composition
//! roots (devserver, desktop). Concentra: configuração de providers (credencial **write-only**),
//! Brain Profile, probe, tarefas assíncronas canceláveis (transcrição, legendas, silêncio, referência,
//! DemandSpec, chat) e eventos por *poll*. Nada aqui devolve segredo; nenhum caminho de edição
//! depende deste serviço (o editor funciona com ele ausente ou com a IA desligada).

mod autonomy_api;
mod connect;
#[cfg(feature = "testkit")]
mod demo;

use crate::assistant::{self, ApprovalMode, AssistantEvent, AssistantOptions};
use crate::autonomy::gateway::GatewayRegistry;
use crate::autonomy::generation::GenerationRegistry;
use crate::autonomy::orchestrator::Orchestrator;
use crate::captions::{self, CaptionOptions};
use crate::ctx::IntelCtx;
use crate::demand::{self, InterpretOptions};
use crate::docs;
use crate::engine::Engine;
use crate::error::{IntelError, IntelResult};
use crate::records::{KIND_DEMAND, KIND_REFERENCE, KIND_TRANSCRIPT, now_ms};
use crate::reference::{self, ReferenceOptions};
use crate::scenes::{self, SceneParams};
use crate::silence::{self, SilenceParams};
use crate::transcript::{self, TranscribeParams};
use capia_ai::CancelToken;
use capia_ai::brain::BrainProfile;
use capia_ai::dispatcher::{AiRuntime, MemoryCache, TaskCtx};
use capia_ai::registry::{ModelEndpoint, ProviderConfig, Registry, provider_presets};
use capia_ai::types::Message;
use capia_ai::usage::{UsageRecord, UsageSink};
use capia_secrets::{CredentialRef, SecretStore, SecretString, register_global};
use capia_store::{AppDb, UsageRow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_EVENTS: usize = 2_000;
const MAX_FINISHED: usize = 64;
const NS: &str = "ai";
const KEY_REGISTRY: &str = "registry";

#[derive(Debug)]
struct TaskEntry {
    kind: String,
    cancel: CancelToken,
    state: &'static str,
    result: Option<Value>,
    error: Option<(String, String)>,
    started_ms: u64,
}

/// Grava o uso/custo de cada chamada no `.capia` do projeto aberto (e só nele).
struct ProjectUsageSink {
    engine: Arc<dyn Engine>,
    store: Mutex<Option<(PathBuf, capia_store::AiStore)>>,
}

impl core::fmt::Debug for ProjectUsageSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProjectUsageSink").finish_non_exhaustive()
    }
}

impl UsageSink for ProjectUsageSink {
    fn record(&self, r: &UsageRecord) {
        let Some(path) = self.engine.project_path() else {
            return;
        };
        let Ok(mut g) = self.store.lock() else { return };
        if g.as_ref().is_none_or(|(p, _)| *p != path) {
            *g = capia_store::AiStore::open(&path, Duration::from_secs(5))
                .ok()
                .map(|s| (path, s));
        }
        if let Some((_, s)) = g.as_ref() {
            let _ = s.add_usage(&UsageRow {
                request_id: r.request_id.clone(),
                task_id: r.task_id.clone(),
                provider_id: r.provider_id.clone(),
                endpoint_id: r.endpoint_id.clone(),
                model_id: r.model_id.clone(),
                capability: r.capability.clone(),
                purpose: r.purpose.clone(),
                input_tokens: r.usage.input_tokens,
                output_tokens: r.usage.output_tokens,
                cached_tokens: r.usage.cached_input_tokens,
                synthetic: r.usage.synthetic,
                cost_known: r.cost.known,
                cost_micros: r.cost.micros,
                currency: r.cost.currency.clone(),
                latency_ms: r.latency_ms,
                attempt: r.attempt,
                status: format!("{:?}", r.status).to_lowercase(),
                error_code: r.error_code.clone(),
                pricing_date: r.pricing_date.clone(),
                timestamp_ms: r.timestamp_ms,
            });
        }
    }
}

#[derive(Debug)]
pub struct ServiceConfig {
    /// Banco global do app (registry/perfis; sem segredos). `None` ⇒ só memória.
    pub appdb_path: Option<PathBuf>,
    pub secrets: Arc<dyn SecretStore>,
}

pub struct IntelligenceService {
    rt: tokio::runtime::Runtime,
    engine: Arc<dyn Engine>,
    ai: Arc<AiRuntime>,
    appdb: Option<Arc<AppDb>>,
    gateways: Arc<GatewayRegistry>,
    generators: Arc<GenerationRegistry>,
    autonomy: Mutex<Option<(PathBuf, Arc<Orchestrator>)>>,
    secrets: Arc<dyn SecretStore>,
    tasks: Arc<Mutex<HashMap<String, TaskEntry>>>,
    events: Arc<Mutex<VecDeque<Value>>>,
    convs: Arc<Mutex<HashMap<String, Vec<Message>>>>,
    issued_plans: Arc<Mutex<HashMap<String, String>>>,
    seq: AtomicU64,
}

impl core::fmt::Debug for IntelligenceService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IntelligenceService")
            .finish_non_exhaustive()
    }
}

fn bad(msg: impl Into<String>) -> IntelError {
    IntelError::new("INVALID_ARGUMENT", msg)
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> IntelResult<T> {
    serde_json::from_value(v).map_err(|e| bad(e.to_string()))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl IntelligenceService {
    pub fn new(engine: Arc<dyn Engine>, cfg: ServiceConfig) -> IntelResult<Self> {
        let appdb = match &cfg.appdb_path {
            Some(p) => Some(Arc::new(AppDb::open(p, Duration::from_secs(5))?)),
            None => None,
        };
        let registry = appdb
            .as_ref()
            .and_then(|db| db.get(NS, KEY_REGISTRY).ok().flatten())
            .and_then(|v| serde_json::from_value::<Registry>(v).ok())
            .unwrap_or_else(Registry::new);
        // segredos já guardados voltam ao redator do processo (nunca aparecem em logs/erros)
        for p in registry.providers.values() {
            if let Some(r) = p
                .credential_ref
                .as_deref()
                .and_then(|r| CredentialRef::new(r).ok())
                && let Ok(s) = cfg.secrets.get(&r)
            {
                register_global(s.expose());
            }
        }
        let sink = Arc::new(ProjectUsageSink {
            engine: engine.clone(),
            store: Mutex::new(None),
        });
        let ai = Arc::new(
            AiRuntime::new(registry, cfg.secrets.clone(), sink)
                .with_cache(Arc::new(MemoryCache::default())),
        );
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("capia-ai")
            .enable_all()
            .build()
            .map_err(|e| IntelError::new("INTERNAL", e.to_string()))?;
        let gateways = Arc::new(GatewayRegistry::new());
        let generators = Arc::new(GenerationRegistry::new());
        autonomy_api::load_gateway_config(appdb.as_deref(), &gateways, &generators);
        Ok(Self {
            rt,
            engine,
            ai,
            appdb,
            gateways,
            generators,
            autonomy: Mutex::new(None),
            secrets: cfg.secrets,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(Mutex::new(VecDeque::new())),
            convs: Arc::new(Mutex::new(HashMap::new())),
            issued_plans: Arc::new(Mutex::new(HashMap::new())),
            seq: AtomicU64::new(1),
        })
    }

    pub fn handles(method: &str) -> bool {
        method.starts_with("ai.")
    }

    pub fn runtime(&self) -> &Arc<AiRuntime> {
        &self.ai
    }

    /// Eventos pendentes (consumidos): progresso, texto em streaming, aprovação, fim, erro.
    pub fn poll_events(&self) -> Vec<Value> {
        lock(&self.events).drain(..).collect()
    }

    fn emit(events: &Arc<Mutex<VecDeque<Value>>>, v: Value) {
        let mut q = lock(events);
        if q.len() >= MAX_EVENTS {
            q.pop_front();
        }
        q.push_back(v);
    }

    fn persist_registry(&self) -> IntelResult<()> {
        if let Some(db) = &self.appdb {
            let reg = self.ai.registry();
            let v = serde_json::to_value(&reg)
                .map_err(|e| IntelError::new("INTERNAL", e.to_string()))?;
            db.put(NS, KEY_REGISTRY, &v, now_ms())?;
        }
        Ok(())
    }

    fn profile(&self) -> BrainProfile {
        self.ai
            .registry()
            .active()
            .cloned()
            .unwrap_or_else(|| BrainProfile::new("none", "none", ""))
    }

    fn ctx(&self) -> IntelCtx {
        IntelCtx::new(self.engine.clone(), self.ai.clone(), self.profile())
    }

    fn provider_view(&self, p: &ProviderConfig) -> Value {
        let configured = p
            .credential_ref
            .as_deref()
            .and_then(|r| CredentialRef::new(r).ok())
            .is_some_and(|r| self.secrets.exists(&r).unwrap_or(false));
        json!({
            "id": p.id, "kind": p.kind, "display_name": p.display_name, "base_url": p.base_url,
            "enabled": p.enabled, "local": p.kind.is_local(), "needs_credential": p.kind.needs_credential(),
            // a UI nunca recebe a chave — só se existe
            "credential_configured": configured,
            "bound_host": p.bound_host, "extra_headers": p.extra_headers,
            "timeout_s": p.timeout_s, "max_concurrency": p.max_concurrency, "allow_loopback": p.allow_loopback,
        })
    }

    fn status(&self) -> Value {
        let reg = self.ai.registry();
        let usable = reg.models.values().any(|m| reg.usable(m));
        json!({
            "enabled": reg.ai_enabled,
            "any_usable_model": usable,
            "secret_backend": self.secrets.backend(),
            "providers": reg.providers.values().map(|p| self.provider_view(p)).collect::<Vec<_>>(),
            "models": reg.models.values().collect::<Vec<_>>(),
            "profiles": reg.profiles.values().collect::<Vec<_>>(),
            "active_profile": reg.active_profile,
            "presets": provider_presets(),
        })
    }

    fn new_task_id(&self, kind: &str) -> String {
        format!(
            "{kind}-{}-{}",
            now_ms(),
            self.seq.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Dispara uma tarefa assíncrona cancelável; devolve `task_id` na hora.
    fn spawn<F, Fut>(&self, kind: &str, f: F) -> Value
    where
        F: FnOnce(IntelCtx, TaskCtx, Arc<dyn Fn(Value) + Send + Sync>) -> Fut + Send + 'static,
        Fut: Future<Output = IntelResult<Value>> + Send + 'static,
    {
        let id = self.new_task_id(kind);
        let profile = self.profile();
        let task = TaskCtx::new(id.clone(), &profile);
        let ctx = IntelCtx::new(self.engine.clone(), self.ai.clone(), profile);
        lock(&self.tasks).insert(
            id.clone(),
            TaskEntry {
                kind: kind.to_owned(),
                cancel: task.cancel.clone(),
                state: "running",
                result: None,
                error: None,
                started_ms: now_ms(),
            },
        );
        let events = self.events.clone();
        let tasks = self.tasks.clone();
        let emit_id = id.clone();
        let emit_events = events.clone();
        let emitter: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |data: Value| {
            Self::emit(
                &emit_events,
                json!({"kind": "ai_task", "task_id": emit_id, "phase": data["phase"].as_str().unwrap_or("progress"), "data": data}),
            );
        });
        Self::emit(
            &events,
            json!({"kind": "ai_task", "task_id": id, "phase": "started", "data": {"task": kind}}),
        );
        let tid = id.clone();
        let cancel = task.cancel.clone();
        self.rt.spawn(async move {
            let res = f(ctx, task, emitter).await;
            let (state, result, error, phase, data) = match res {
                Ok(v) => ("done", Some(v.clone()), None, "done", json!({"result": v})),
                Err(e) if e.is_cancelled() || cancel.is_cancelled() => {
                    ("cancelled", None, None, "cancelled", json!({}))
                }
                Err(e) => (
                    "failed",
                    None,
                    Some((e.code.clone(), e.message.clone())),
                    "error",
                    json!({"code": e.code, "message": e.message}),
                ),
            };
            {
                let mut t = lock(&tasks);
                if let Some(en) = t.get_mut(&tid) {
                    en.state = state;
                    en.result = result;
                    en.error = error;
                }
                // mantém só as últimas tarefas terminadas
                let mut done: Vec<(String, u64)> = t
                    .iter()
                    .filter(|(_, e)| e.state != "running")
                    .map(|(k, e)| (k.clone(), e.started_ms))
                    .collect();
                if done.len() > MAX_FINISHED {
                    done.sort_by_key(|(_, s)| *s);
                    for (k, _) in done.iter().take(done.len() - MAX_FINISHED) {
                        t.remove(k);
                    }
                }
            }
            Self::emit(
                &events,
                json!({"kind": "ai_task", "task_id": tid, "phase": phase, "data": data}),
            );
        });
        json!({ "task_id": id })
    }

    /// Mesma entrada para os hospedeiros: erro vira `{code, message}` (já redigido).
    pub fn call_json(&self, method: &str, params: Value) -> Result<Value, Value> {
        self.call(method, params)
            .map_err(|e| json!({"code": e.code, "message": e.message}))
    }

    /// Acrescenta os eventos de IA ao `events.poll` do editor: a UI mantém **um** laço de poll.
    pub fn merge_events(&self, reply: &mut Value) {
        let extra = self.poll_events();
        if extra.is_empty() {
            return;
        }
        if let Some(arr) = reply.get_mut("events").and_then(Value::as_array_mut) {
            arr.extend(extra);
        }
    }

    /// DEV/E2E: carrega roteiros Replay (`{"<provider_id>": [ReplayResponse…]}`) e os registra. O
    /// produto não chama isto: sem roteiro, um provider `replay` falha com `NOT_CONFIGURED`.
    #[cfg(feature = "testkit")]
    pub fn load_replay_scripts(&self, scripts: &Value) -> IntelResult<usize> {
        use capia_ai::providers::replay::{ReplayProvider, ReplayResponse};
        let obj = scripts
            .as_object()
            .ok_or_else(|| bad("expected an object"))?;
        for (id, v) in obj {
            let script: Vec<ReplayResponse> = parse(v.clone())?;
            self.ai
                .register_replay(id, Arc::new(ReplayProvider::scripted(id.clone(), script)));
        }
        Ok(obj.len())
    }

    /// Ponto único de entrada síncrono (retorna rápido; o trabalho pesado roda em segundo plano).
    pub fn call(&self, method: &str, p: Value) -> IntelResult<Value> {
        match method {
            "ai.status" => Ok(self.status()),
            m if m.starts_with("ai.run.")
                || m.starts_with("ai.memory.")
                || m.starts_with("ai.gateway.")
                || m.starts_with("ai.generation.") =>
            {
                self.autonomy_call(m, p)
            }
            "ai.enabled.set" => {
                let enabled = p["enabled"]
                    .as_bool()
                    .ok_or_else(|| bad("`enabled` is required"))?;
                self.ai.update_registry(|r| r.ai_enabled = enabled);
                self.persist_registry()?;
                Ok(self.status())
            }
            "ai.provider.save" => self.provider_save(p),
            "ai.connect" => {
                let preset_key = p["preset"]
                    .as_str()
                    .ok_or_else(|| bad("`preset` is required"))?
                    .to_owned();
                let preset = provider_presets()
                    .into_iter()
                    .find(|x| x.key == preset_key)
                    .ok_or_else(|| bad(format!("unknown preset `{preset_key}`")))?;
                let mut cfg = ProviderConfig::new(preset.key, preset.kind, preset.display_name);
                cfg.base_url = p["base_url"]
                    .as_str()
                    .filter(|u| !u.is_empty())
                    .map(str::to_owned)
                    .or_else(|| preset.base_url.map(str::to_owned));
                cfg.enabled = true;
                // loopback só para gateway/proxy local explícito (e testes)
                cfg.allow_loopback = p["allow_loopback"].as_bool().unwrap_or(false);
                let provider = serde_json::to_value(&cfg)
                    .map_err(|e| IntelError::new("INTERNAL", e.to_string()))?;
                self.provider_save(json!({"provider": provider, "api_key": p["api_key"]}))?;
                let wanted = p["model"].as_str().map(str::to_owned);
                let this = self.ai.clone();
                let db = self.appdb.clone();
                let provider_id = preset_key;
                Ok(self.spawn("connect", move |_ctx, task, _em| async move {
                    let persist_ai = this.clone();
                    connect::finish_connect(
                        this,
                        provider_id,
                        wanted,
                        task.cancel.clone(),
                        move || {
                            if let (Some(db), Ok(v)) =
                                (&db, serde_json::to_value(persist_ai.registry()))
                            {
                                let _ = db.put(NS, KEY_REGISTRY, &v, now_ms());
                            }
                        },
                    )
                    .await
                }))
            }
            "ai.provider.delete" => {
                let id = p["id"]
                    .as_str()
                    .ok_or_else(|| bad("`id` is required"))?
                    .to_owned();
                if let Some(r) = self
                    .ai
                    .registry()
                    .providers
                    .get(&id)
                    .and_then(|pc| pc.credential_ref.clone())
                    .and_then(|r| CredentialRef::new(r).ok())
                {
                    let _ = self.secrets.delete(&r);
                }
                self.ai.update_registry(|r| r.remove_provider(&id));
                self.persist_registry()?;
                Ok(self.status())
            }
            "ai.credential.delete" => {
                let id = p["provider_id"]
                    .as_str()
                    .ok_or_else(|| bad("`provider_id` is required"))?;
                let cref = CredentialRef::for_provider(id).map_err(|e| bad(e.to_string()))?;
                self.secrets
                    .delete(&cref)
                    .map_err(|e| IntelError::new("SECRET_STORE", e.to_string()))?;
                Ok(self.status())
            }
            "ai.model.save" => {
                let m: ModelEndpoint = parse(p["endpoint"].clone())?;
                let mut res = Ok(());
                self.ai.update_registry(|r| {
                    // preserva evidência de probe/saúde de um endpoint já conhecido
                    let mut m = m.clone();
                    if let Some(old) = r.models.get(&m.id) {
                        m.last_probe.clone_from(&old.last_probe);
                        m.health = old.health;
                        m.consecutive_failures = old.consecutive_failures;
                    }
                    res = r.upsert_model(m);
                });
                res.map_err(bad)?;
                self.persist_registry()?;
                Ok(self.status())
            }
            "ai.model.delete" => {
                let id = p["id"]
                    .as_str()
                    .ok_or_else(|| bad("`id` is required"))?
                    .to_owned();
                self.ai.update_registry(|r| {
                    r.models.remove(&id);
                });
                self.persist_registry()?;
                Ok(self.status())
            }
            "ai.brain.set" => {
                let prof: BrainProfile = parse(p["profile"].clone())?;
                let reg = self.ai.registry();
                if !prof.brain.is_empty() && !reg.models.contains_key(&prof.brain) {
                    return Err(bad(format!("unknown brain endpoint `{}`", prof.brain)));
                }
                self.ai.update_registry(|r| {
                    r.active_profile = Some(prof.id.clone());
                    r.profiles.insert(prof.id.clone(), prof.clone());
                });
                self.persist_registry()?;
                Ok(self.status())
            }
            "ai.diagnostics" => {
                let text = serde_json::to_string_pretty(&self.status()).unwrap_or_default();
                Ok(json!({ "text": capia_secrets::redact(&text) }))
            }
            "ai.model.probe" => {
                let id = p["endpoint_id"]
                    .as_str()
                    .ok_or_else(|| bad("`endpoint_id` is required"))?
                    .to_owned();
                let this = self.ai.clone();
                let persist = self.appdb.is_some();
                let _ = persist;
                Ok(self.spawn("probe", move |_ctx, task, _em| async move {
                    let r = this.probe_endpoint(&id, &task.cancel).await?;
                    Ok(serde_json::to_value(&r).unwrap_or_default())
                }))
            }
            "ai.models.import" => {
                let id = p["provider_id"]
                    .as_str()
                    .ok_or_else(|| bad("`provider_id` is required"))?
                    .to_owned();
                let this = self.ai.clone();
                Ok(
                    self.spawn("import_models", move |_ctx, task, _em| async move {
                        let n = this.import_models(&id, &task.cancel).await?;
                        Ok(json!({ "imported": n }))
                    }),
                )
            }
            "ai.transcribe" => {
                let tp: TranscribeParams = parse(p)?;
                Ok(self.spawn("transcribe", move |ctx, task, em| async move {
                    let em2 = em.clone();
                    let rec = transcript::transcribe_asset(&ctx, &task, &tp, &move |i, n| {
                        em2(json!({"phase": "progress", "chunk": i, "chunks": n}));
                    })
                    .await?;
                    Ok(json!({
                        "transcript_id": rec.id, "segments": rec.transcript.segments.len(),
                        "language": rec.transcript.language, "from_cache": rec.from_cache,
                        "local": rec.local, "cost_micros": rec.cost_micros,
                        "model": rec.model_id,
                    }))
                }))
            }
            "ai.transcript.get" => {
                let asset = p["asset_id"]
                    .as_str()
                    .ok_or_else(|| bad("`asset_id` is required"))?;
                let rows =
                    self.ctx()
                        .records()?
                        .store()
                        .list(KIND_TRANSCRIPT, Some(asset), true, 1)?;
                Ok(rows.into_iter().next().map_or(
                    json!({"available": false}),
                    |r| json!({"available": true, "record": r.json}),
                ))
            }
            "ai.captions.plan" => {
                #[derive(Deserialize)]
                struct P {
                    sequence: String,
                    asset_id: String,
                    #[serde(default)]
                    options: Option<CaptionOptions>,
                    #[serde(default)]
                    language: Option<String>,
                }
                let q: P = parse(p)?;
                let issued = self.issued_plans.clone();
                Ok(self.spawn("captions", move |ctx, task, em| async move {
                    let mut tp = TranscribeParams::new(&q.asset_id);
                    tp.language = q.language.clone();
                    let em2 = em.clone();
                    let rec = transcript::transcribe_asset(&ctx, &task, &tp, &move |i, n| {
                        em2(json!({"phase": "progress", "stage": "transcribe", "chunk": i, "chunks": n}));
                    })
                    .await?;
                    let plan = captions::plan_captions(
                        &ctx,
                        &rec,
                        &q.sequence,
                        &q.options.unwrap_or_default(),
                        &task.task_id,
                    )?;
                    if let Some(t) = &plan.plan_token {
                        lock(&issued).insert(t.clone(), "Legendas automáticas".into());
                    }
                    Ok(json!({"cue_count": plan.cue_count, "plan_token": plan.plan_token, "op_count": plan.preview["op_count"]}))
                }))
            }
            "ai.silence.plan" => {
                #[derive(Deserialize)]
                struct P {
                    sequence: String,
                    asset_id: String,
                    #[serde(default)]
                    clip: Option<String>,
                    #[serde(default)]
                    params: Option<SilenceParams>,
                    #[serde(default)]
                    ripple_scope: Option<String>,
                }
                let q: P = parse(p)?;
                let issued = self.issued_plans.clone();
                Ok(self.spawn("silence", move |ctx, task, _em| async move {
                    let cancel = task.cancel.clone();
                    let tid = task.task_id.clone();
                    let plan = tokio::task::spawn_blocking(move || {
                        silence::plan_silence_cut(
                            &ctx,
                            &q.asset_id,
                            &q.sequence,
                            &q.params.unwrap_or_default(),
                            q.ripple_scope.as_deref().unwrap_or("track"),
                            &tid,
                            q.clip.as_deref(),
                            &cancel,
                        )
                    })
                    .await
                    .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??;
                    if let Some(t) = &plan.plan_token {
                        lock(&issued).insert(t.clone(), "Remover silêncios".into());
                    }
                    Ok(json!({"cut_count": plan.cut_count, "removed_us": plan.removed_us, "plan_token": plan.plan_token}))
                }))
            }
            "ai.plan.apply" => {
                let token = p["plan_token"]
                    .as_str()
                    .ok_or_else(|| bad("`plan_token` is required"))?;
                if !lock(&self.issued_plans).contains_key(token) {
                    return Err(IntelError::new(
                        "UNKNOWN_PLAN",
                        "this plan was not produced by an AI task in this session",
                    ));
                }
                let r = captions::apply_plan(&self.ctx(), token)?;
                Ok(r)
            }
            "ai.scenes.detect" => {
                let asset = p["asset_id"]
                    .as_str()
                    .ok_or_else(|| bad("`asset_id` is required"))?
                    .to_owned();
                Ok(self.spawn("scenes", move |ctx, task, _em| async move {
                    let file = transcript::asset_file(&ctx, &asset)?;
                    let tc = ctx.engine.toolchain().ok_or_else(|| {
                        IntelError::new("FFMPEG_NOT_FOUND", "ffmpeg is not available")
                    })?;
                    let cancel = task.cancel.clone();
                    let b = tokio::task::spawn_blocking(move || {
                        scenes::detect_media(
                            &tc,
                            &file.path,
                            0,
                            (25, 1),
                            &SceneParams::default(),
                            Duration::from_secs(1800),
                            &move || cancel.is_cancelled(),
                        )
                    })
                    .await
                    .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??;
                    Ok(json!({"boundaries": b, "scan_fps": 25}))
                }))
            }
            "ai.reference.analyze" => {
                let asset = p["asset_id"]
                    .as_str()
                    .ok_or_else(|| bad("`asset_id` is required"))?
                    .to_owned();
                let force = p["force"].as_bool().unwrap_or(false);
                Ok(self.spawn("reference", move |ctx, task, em| async move {
                    let o = ReferenceOptions {
                        force,
                        ..ReferenceOptions::default()
                    };
                    let (g, cached) = reference::analyze_reference(
                        &ctx,
                        &task,
                        &asset,
                        &o,
                        &move |stage, i, n| {
                            em(json!({"phase": "progress", "stage": stage, "step": i, "steps": n}));
                        },
                    )
                    .await?;
                    Ok(json!({"cached": cached, "grammar": g}))
                }))
            }
            "ai.reference.get" => {
                let asset = p["asset_id"]
                    .as_str()
                    .ok_or_else(|| bad("`asset_id` is required"))?;
                let rows =
                    self.ctx()
                        .records()?
                        .store()
                        .list(KIND_REFERENCE, Some(asset), true, 1)?;
                Ok(rows.into_iter().next().map_or(
                    json!({"available": false}),
                    |r| json!({"available": true, "grammar": r.json}),
                ))
            }
            "ai.demand.interpret" => {
                #[derive(Deserialize)]
                struct P {
                    #[serde(default)]
                    documents: Vec<String>,
                    #[serde(default)]
                    assets: Vec<String>,
                    #[serde(default)]
                    note: Option<String>,
                    #[serde(default)]
                    force: bool,
                }
                let q: P = parse(p)?;
                if q.documents.is_empty() && q.assets.is_empty() {
                    return Err(bad("provide at least one document or video asset"));
                }
                if q.documents.len() + q.assets.len() > 12 {
                    return Err(bad("at most 12 sources per interpretation"));
                }
                Ok(self.spawn("demand", move |ctx, task, em| async move {
                    let mut all = Vec::new();
                    for path in &q.documents {
                        em(json!({"phase": "progress", "stage": "extract", "source": path.rsplit(['/', '\\']).next()}));
                        let p2 = PathBuf::from(path);
                        all.push(
                            tokio::task::spawn_blocking(move || docs::extract_file(&p2))
                                .await
                                .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??,
                        );
                    }
                    for asset in &q.assets {
                        em(json!({"phase": "progress", "stage": "transcribe", "source": asset}));
                        let rec = transcript::transcribe_asset(&ctx, &task, &TranscribeParams::new(asset), &|_, _| {}).await?;
                        all.push(docs::extract_transcript(asset, asset, &rec.transcript));
                    }
                    em(json!({"phase": "progress", "stage": "interpret"}));
                    let (spec, cached) = demand::interpret(
                        &ctx,
                        &task,
                        &all,
                        &InterpretOptions { user_note: q.note, force: q.force },
                    )
                    .await?;
                    Ok(json!({"cached": cached, "spec": spec}))
                }))
            }
            "ai.demand.get" => {
                let rows = self
                    .ctx()
                    .records()?
                    .store()
                    .list(KIND_DEMAND, None, true, 5)?;
                Ok(json!({"specs": rows.into_iter().map(|r| r.json).collect::<Vec<_>>()}))
            }
            "ai.demand.save" => {
                let spec: demand::DemandSpec = parse(p["spec"].clone())?;
                Ok(json!({"spec": demand::save_edit(&self.ctx(), spec)?}))
            }
            "ai.assistant.send" => {
                #[derive(Deserialize)]
                struct P {
                    #[serde(default)]
                    conversation_id: Option<String>,
                    text: String,
                    #[serde(default)]
                    mode: Option<ApprovalMode>,
                    #[serde(default)]
                    selected_clips: Vec<String>,
                    #[serde(default)]
                    playhead_ticks: Option<i64>,
                    #[serde(default)]
                    sequence: Option<String>,
                }
                let q: P = parse(p)?;
                let ui_context = assistant::render_ui_context(
                    &q.selected_clips,
                    q.playhead_ticks,
                    q.sequence.as_deref(),
                );
                if q.text.trim().is_empty() || q.text.chars().count() > 8_000 {
                    return Err(bad("the message must have 1..8000 characters"));
                }
                let conv_id = q
                    .conversation_id
                    .unwrap_or_else(|| self.new_task_id("conv"));
                let history = lock(&self.convs).get(&conv_id).cloned().unwrap_or_default();
                let convs = self.convs.clone();
                let mode = q.mode.unwrap_or(ApprovalMode::Ask);
                let cid = conv_id.clone();
                let mut out = self.spawn("assistant", move |ctx, task, em| async move {
                    let opts = AssistantOptions { mode, ui_context: Some(ui_context), ..AssistantOptions::default() };
                    let em2 = em.clone();
                    let on = move |e: AssistantEvent| match e {
                        AssistantEvent::Text(t) => em2(json!({"phase": "text", "delta": t})),
                        AssistantEvent::Reset { reason } => em2(json!({"phase": "reset", "reason": reason})),
                        AssistantEvent::ToolStarted { name, step } => em2(json!({"phase": "tool", "name": name, "step": step, "state": "started"})),
                        AssistantEvent::ToolFinished { name, ok } => em2(json!({"phase": "tool", "name": name, "ok": ok, "state": "finished"})),
                        AssistantEvent::ApprovalRequired(p) => em2(json!({"phase": "approval", "plan": p})),
                    };
                    let out = assistant::run_turn(&ctx, &task, &history, &q.text, &opts, &on).await;
                    lock(&convs).entry(cid).or_default().extend(out.history_delta.clone());
                    if out.task.status == assistant::TaskStatus::Cancelled {
                        return Err(IntelError::cancelled());
                    }
                    Ok(serde_json::to_value(&out.task).unwrap_or_default())
                });
                out["conversation_id"] = json!(conv_id);
                Ok(out)
            }
            "ai.assistant.approve" => {
                let task = p["task_id"]
                    .as_str()
                    .ok_or_else(|| bad("`task_id` is required"))?;
                let token = p["plan_token"]
                    .as_str()
                    .ok_or_else(|| bad("`plan_token` is required"))?;
                Ok(json!({"task": assistant::approve(&self.ctx(), task, token)?}))
            }
            "ai.assistant.reject" => {
                let task = p["task_id"]
                    .as_str()
                    .ok_or_else(|| bad("`task_id` is required"))?;
                Ok(json!({"task": assistant::reject(&self.ctx(), task)?}))
            }
            "ai.task.cancel" => {
                let id = p["task_id"]
                    .as_str()
                    .ok_or_else(|| bad("`task_id` is required"))?;
                match lock(&self.tasks).get(id) {
                    Some(t) if t.state == "running" => {
                        t.cancel.cancel();
                        Ok(json!({"cancelled": true}))
                    }
                    Some(_) => Ok(json!({"cancelled": false, "reason": "already finished"})),
                    None => Err(IntelError::new("NOT_FOUND", "unknown task")),
                }
            }
            "ai.task.get" => {
                let id = p["task_id"]
                    .as_str()
                    .ok_or_else(|| bad("`task_id` is required"))?;
                lock(&self.tasks)
                    .get(id)
                    .map(|t| json!({"task_id": id, "kind": t.kind, "state": t.state, "result": t.result, "error": t.error}))
                    .ok_or_else(|| IntelError::new("NOT_FOUND", "unknown task"))
            }
            "ai.usage.summary" => {
                let rec = self.ctx().records()?;
                let task = p["task_id"].as_str();
                Ok(serde_json::to_value(rec.store().usage_summary(task)?).unwrap_or_default())
            }
            other => Err(IntelError::new(
                "UNKNOWN_METHOD",
                format!("unknown method `{other}`"),
            )),
        }
    }

    fn provider_save(&self, p: Value) -> IntelResult<Value> {
        let mut cfg: ProviderConfig = parse(p["provider"].clone())?;
        // a UI nunca define referência nem host de credencial: o servidor decide
        let existing = self.ai.registry().providers.get(&cfg.id).cloned();
        cfg.credential_ref = existing.as_ref().and_then(|e| e.credential_ref.clone());
        cfg.created_at = existing
            .as_ref()
            .map(|e| e.created_at)
            .filter(|t| *t != 0)
            .unwrap_or_else(now_ms);
        cfg.validate().map_err(bad)?;
        // host ao qual a credencial fica presa (vem da URL efetiva, validada)
        cfg.bound_host = match cfg.effective_base_url() {
            Some(u) => capia_ai::http::UrlPolicy::for_config(&cfg)
                .check(&u)
                .map_err(|e| bad(e.message))?
                .host_str()
                .map(str::to_owned),
            None => None,
        };
        if let Some(key) = p["api_key"].as_str().filter(|k| !k.is_empty()) {
            if !cfg.kind.needs_credential() && !cfg.kind.is_local() {
                return Err(bad("this provider does not take a credential"));
            }
            if key.len() > 4_096 || key.chars().any(char::is_control) {
                return Err(bad("the credential has an invalid format"));
            }
            let cref = CredentialRef::for_provider(&cfg.id).map_err(|e| bad(e.to_string()))?;
            // registra ANTES de gravar: daqui em diante qualquer texto que o contenha é redigido
            register_global(key);
            self.secrets
                .put(&cref, SecretString::new(key))
                .map_err(|e| IntelError::new("SECRET_STORE", e.to_string()))?;
            cfg.credential_ref = Some(cref.as_str().to_owned());
        }
        let id = cfg.id.clone();
        let mut res = Ok(());
        self.ai.update_registry(|r| res = r.upsert_provider(cfg));
        res.map_err(bad)?;
        self.persist_registry()?;
        let reg = self.ai.registry();
        Ok(json!({"provider": reg.providers.get(&id).map(|p| self.provider_view(p))}))
    }
}
