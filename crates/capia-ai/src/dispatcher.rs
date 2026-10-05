//! `AiRuntime`: executa chamadas lógicas sobre o registry — roteia (Router), aplica orçamento,
//! faz retry com backoff (só transitórios), fallback compatível, valida saída estruturada com
//! reparo limitado, consulta o cache determinístico e registra uso/custo/latência por tentativa.

use crate::brain::{BrainProfile, DataClass};
use crate::cancel::CancelToken;
use crate::capability::{Capabilities, Capability};
use crate::error::{ErrorCode, ProviderError};
use crate::probe::{ProbeOptions, ProbeResult, probe};
use crate::providers::{CallCtx, ModelProvider, build_provider, collect, replay::ReplayProvider};
use crate::registry::{ModelEndpoint, ProviderConfig, ProviderKind, Registry};
use crate::router::{Decision, RouteRequest, chain};
use crate::stt::{SttRequest, Transcript};
use crate::types::{ChatEvent, ChatRequest, ChatResponse, Message, ResponseSchema, Role};
use crate::usage::{BudgetCheck, BudgetTracker, CallStatus, Cost, UsageRecord, UsageSink, cost_of};
use capia_secrets::SecretStore;
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Tentativas por endpoint (1 = sem retry). Spec: máx. 3.
    pub max_attempts: u32,
    pub base_ms: u64,
    pub max_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_ms: 250,
            max_ms: 8_000,
        }
    }
}

fn backoff(p: &RetryPolicy, attempt: u32, retry_after_ms: Option<u64>) -> Duration {
    let exp = p
        .base_ms
        .saturating_mul(1u64 << attempt.saturating_sub(1).min(16))
        .min(p.max_ms);
    // jitter barato e sem relógio global: derivado do nº da tentativa e do tempo
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let jitter = if p.base_ms == 0 {
        0
    } else {
        u64::from(nanos) % p.base_ms.max(1)
    };
    let ms = retry_after_ms
        .map_or(exp + jitter, |ra| ra.max(exp))
        .min(120_000);
    Duration::from_millis(ms)
}

/// Evento para a UI durante o streaming.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamNotice {
    Event(ChatEvent),
    /// Nova tentativa: descarte o texto parcial já mostrado.
    Retry {
        attempt: u32,
        reason: String,
    },
    /// Mudou para outro endpoint (fallback).
    Fallback {
        endpoint_id: String,
        reason: String,
    },
}

pub type OnNotice<'a> = Option<&'a (dyn Fn(StreamNotice) + Send + Sync)>;

/// Cache determinístico de respostas (chave = digest do pedido final).
pub trait ResponseCache: Send + Sync + core::fmt::Debug {
    fn get(&self, key: &str) -> Option<ChatResponse>;
    fn put(&self, key: &str, value: &ChatResponse);
}

#[derive(Default)]
pub struct MemoryCache {
    map: Mutex<HashMap<String, ChatResponse>>,
}

impl core::fmt::Debug for MemoryCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemoryCache").finish_non_exhaustive()
    }
}

impl ResponseCache for MemoryCache {
    fn get(&self, key: &str) -> Option<ChatResponse> {
        self.map.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: &str, value: &ChatResponse) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(key.to_owned(), value.clone());
        }
    }
}

/// Contexto de uma tarefa (rastreio, cancelamento, orçamento).
#[derive(Clone, Debug)]
pub struct TaskCtx {
    pub task_id: String,
    pub role: Option<String>,
    pub purpose: Option<String>,
    pub cancel: CancelToken,
    pub budget: Arc<BudgetTracker>,
}

impl TaskCtx {
    pub fn new(task_id: impl Into<String>, profile: &BrainProfile) -> Self {
        Self {
            task_id: task_id.into(),
            role: None,
            purpose: None,
            cancel: CancelToken::new(),
            budget: Arc::new(BudgetTracker::new(profile.budgets.clone())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatOptions {
    pub route: RouteRequest,
    /// Só vale quando o pedido é determinístico (temperatura 0) — o chamador decide.
    pub cacheable: bool,
}

impl ChatOptions {
    pub fn text() -> Self {
        Self {
            route: RouteRequest::for_capability(Capability::TextGeneration),
            cacheable: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Attempt {
    pub endpoint_id: String,
    pub attempt: u32,
    pub error: Option<ProviderError>,
}

#[derive(Clone, Debug)]
pub struct ChatOutcome {
    pub response: ChatResponse,
    pub decision: Decision,
    pub attempts: Vec<Attempt>,
    pub cost: Cost,
    pub cache_hit: bool,
}

#[derive(Clone, Debug)]
pub struct SttOutcome {
    pub transcript: Transcript,
    pub decision: Decision,
    pub attempts: Vec<Attempt>,
    pub cost: Cost,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// id do provider → (assinatura da config, instância)
type ProviderCache = HashMap<String, (String, Arc<dyn ModelProvider>)>;

pub struct AiRuntime {
    registry: RwLock<Registry>,
    store: Arc<dyn SecretStore>,
    sink: Arc<dyn UsageSink>,
    cache: Option<Arc<dyn ResponseCache>>,
    replays: RwLock<HashMap<String, Arc<ReplayProvider>>>,
    providers: Mutex<ProviderCache>,
    pub retry: RetryPolicy,
    req_seq: std::sync::atomic::AtomicU64,
}

impl core::fmt::Debug for AiRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AiRuntime").finish_non_exhaustive()
    }
}

impl AiRuntime {
    pub fn new(registry: Registry, store: Arc<dyn SecretStore>, sink: Arc<dyn UsageSink>) -> Self {
        Self {
            registry: RwLock::new(registry),
            store,
            sink,
            cache: None,
            replays: RwLock::new(HashMap::new()),
            providers: Mutex::new(HashMap::new()),
            retry: RetryPolicy::default(),
            req_seq: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn with_cache(mut self, c: Arc<dyn ResponseCache>) -> Self {
        self.cache = Some(c);
        self
    }

    pub fn registry(&self) -> Registry {
        self.registry.read().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn update_registry<R>(&self, f: impl FnOnce(&mut Registry) -> R) -> R {
        let mut g = self
            .registry
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut g)
    }

    pub fn register_replay(&self, provider_id: &str, p: Arc<ReplayProvider>) {
        if let Ok(mut m) = self.replays.write() {
            m.insert(provider_id.to_owned(), p);
        }
    }

    pub fn store(&self) -> &Arc<dyn SecretStore> {
        &self.store
    }

    fn provider(&self, cfg: &ProviderConfig) -> Result<Arc<dyn ModelProvider>, ProviderError> {
        if cfg.kind == ProviderKind::Replay {
            return self
                .replays
                .read()
                .ok()
                .and_then(|m| m.get(&cfg.id).cloned())
                .map(|p| p as Arc<dyn ModelProvider>)
                .ok_or_else(|| {
                    ProviderError::new(
                        ErrorCode::NotConfigured,
                        "replay fixture not loaded for this provider",
                    )
                });
        }
        let sig = serde_json::to_string(cfg).unwrap_or_default();
        let mut cache = self.providers.lock().map_err(|_| {
            ProviderError::new(ErrorCode::ProviderUnavailable, "provider cache poisoned")
        })?;
        if let Some((s, p)) = cache.get(&cfg.id)
            && *s == sig
        {
            return Ok(p.clone());
        }
        let p = build_provider(cfg, self.store.clone())?;
        cache.insert(cfg.id.clone(), (sig, p.clone()));
        Ok(p)
    }

    fn endpoint(&self, id: &str) -> Result<(ModelEndpoint, ProviderConfig), ProviderError> {
        let reg = self
            .registry
            .read()
            .map_err(|_| ProviderError::new(ErrorCode::NotConfigured, "registry poisoned"))?;
        let m =
            reg.models.get(id).cloned().ok_or_else(|| {
                ProviderError::new(ErrorCode::ModelNotFound, "endpoint not found")
            })?;
        let p =
            reg.providers.get(&m.provider_id).cloned().ok_or_else(|| {
                ProviderError::new(ErrorCode::NotConfigured, "provider not found")
            })?;
        Ok((m, p))
    }

    fn profile(&self) -> Result<BrainProfile, ProviderError> {
        self.registry
            .read()
            .ok()
            .and_then(|r| r.active().cloned())
            .ok_or_else(|| ProviderError::new(ErrorCode::NotConfigured, "no active Brain Profile"))
    }

    fn routes(&self, ropts: &RouteRequest) -> Result<(Vec<Decision>, BrainProfile), ProviderError> {
        let profile = self.profile()?;
        let reg = self.registry();
        Ok((chain(&reg, &profile, ropts)?, profile))
    }

    /// Decisões de roteamento (sem chamar ninguém): permite saber **antes** qual modelo serve e
    /// compor chaves de cache/auditoria. Erro estruturado quando nenhuma rota é válida.
    pub fn route_preview(&self, route: &RouteRequest) -> Result<Vec<Decision>, ProviderError> {
        Ok(self.routes(route)?.0)
    }

    /// O destino roda na máquina do usuário (Ollama/LM Studio/whisper.cpp/Replay)?
    pub fn is_endpoint_local(&self, endpoint_id: &str) -> bool {
        self.endpoint(endpoint_id)
            .is_ok_and(|(_, p)| p.kind.is_local())
    }

    fn mark(&self, endpoint_id: &str, ok: bool) {
        self.update_registry(|r| {
            if let Some(m) = r.models.get_mut(endpoint_id) {
                m.record_outcome(ok);
            }
        });
    }

    fn rid(&self) -> String {
        format!(
            "req_{}",
            self.req_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        task: &TaskCtx,
        ep: &ModelEndpoint,
        cap: Capability,
        status: CallStatus,
        usage: &crate::types::Usage,
        cost: &Cost,
        latency: Duration,
        attempt: u32,
        err: Option<&ProviderError>,
        rid: &str,
    ) {
        self.sink.record(&UsageRecord {
            request_id: rid.to_owned(),
            task_id: Some(task.task_id.clone()),
            provider_id: ep.provider_id.clone(),
            endpoint_id: ep.id.clone(),
            model_id: ep.model_id.clone(),
            capability: format!("{cap:?}"),
            purpose: task.purpose.clone(),
            usage: usage.clone(),
            cost: cost.clone(),
            latency_ms: latency.as_millis() as u64,
            attempt,
            status,
            error_code: err.map(|e| e.code.as_str().to_owned()),
            pricing_date: ep.pricing.as_ref().map(|p| p.effective_date.clone()),
            timestamp_ms: now_ms(),
        });
    }

    /// Uma chamada lógica de chat: rota → (cache) → tentativas/fallback.
    pub async fn chat(
        &self,
        task: &TaskCtx,
        req: ChatRequest,
        opts: ChatOptions,
        on: OnNotice<'_>,
    ) -> Result<ChatOutcome, ProviderError> {
        let mut route = opts.route.clone();
        if route.role.is_none() {
            route.role.clone_from(&task.role);
        }
        if req.needs_vision()
            && route.capability != Some(Capability::VisionInput)
            && !route.also_needs.contains(&Capability::VisionInput)
        {
            route.also_needs.push(Capability::VisionInput);
        }
        if !req.tools.is_empty() && !route.also_needs.contains(&Capability::ToolCalling) {
            route.also_needs.push(Capability::ToolCalling);
        }
        if req.needs_vision() && !route.data.contains(&DataClass::Frames) {
            route.data.push(DataClass::Frames);
        }
        let (decisions, _profile) = self.routes(&route)?;
        let mut attempts: Vec<Attempt> = Vec::new();
        let mut last_err: Option<ProviderError> = None;
        let cap = route.capability.unwrap_or(Capability::TextGeneration);

        for (di, d) in decisions.iter().enumerate() {
            let (ep, pcfg) = self.endpoint(&d.endpoint_id)?;
            let provider = match self.provider(&pcfg) {
                Ok(p) => p,
                Err(e) => {
                    attempts.push(Attempt {
                        endpoint_id: ep.id.clone(),
                        attempt: 0,
                        error: Some(e.clone()),
                    });
                    last_err = Some(e);
                    continue;
                }
            };
            if di > 0
                && let Some(f) = on
            {
                f(StreamNotice::Fallback {
                    endpoint_id: ep.id.clone(),
                    reason: last_err
                        .as_ref()
                        .map_or_else(String::new, |e| e.code.as_str().to_owned()),
                });
            }
            let mut r = req.clone();
            r.model = ep.model_id.clone();
            if r.params.temperature.is_none() {
                r.params.temperature = ep.default_params.temperature;
            }
            if r.params.max_output_tokens.is_none() {
                r.params.max_output_tokens = ep.default_params.max_output_tokens;
            }
            r.meta.task_id = Some(task.task_id.clone());
            r.meta.role.clone_from(&task.role);

            // cache determinístico: só hit; o custo de provider não se repete
            let key = r.digest();
            if opts.cacheable
                && let Some(c) = &self.cache
                && let Some(hit) = c.get(&key)
            {
                let rid = self.rid();
                self.record(
                    task,
                    &ep,
                    cap,
                    CallStatus::CacheHit,
                    &Default::default(),
                    &Cost::zero(ep.pricing.as_ref().map(|p| p.currency.clone())),
                    Duration::ZERO,
                    1,
                    None,
                    &rid,
                );
                return Ok(ChatOutcome {
                    response: hit,
                    decision: d.clone(),
                    attempts,
                    cost: Cost::zero(None),
                    cache_hit: true,
                });
            }

            let max = self.retry.max_attempts.max(1);
            let mut attempt = 0u32;
            loop {
                attempt += 1;
                if task.cancel.is_cancelled() {
                    return Err(ProviderError::cancelled());
                }
                match task.budget.check(None) {
                    BudgetCheck::Exceeded(m) => {
                        return Err(ProviderError::new(ErrorCode::BudgetExceeded, m));
                    }
                    BudgetCheck::Warn(_) | BudgetCheck::Ok => {}
                }
                let rid = self.rid();
                let t0 = Instant::now();
                let ctx = CallCtx::new(task.cancel.clone());
                let result = async {
                    let stream = provider.chat(&r, &ctx).await?;
                    drain(stream, &task.cancel, on).await
                }
                .await;
                match result {
                    Ok(resp) => {
                        let cost = cost_of(ep.pricing.as_ref(), &resp.usage);
                        task.budget.charge(&resp.usage, &cost);
                        self.record(
                            task,
                            &ep,
                            cap,
                            CallStatus::Ok,
                            &resp.usage,
                            &cost,
                            t0.elapsed(),
                            attempt,
                            None,
                            &rid,
                        );
                        self.mark(&ep.id, true);
                        attempts.push(Attempt {
                            endpoint_id: ep.id.clone(),
                            attempt,
                            error: None,
                        });
                        if opts.cacheable
                            && let Some(c) = &self.cache
                        {
                            c.put(&key, &resp);
                        }
                        return Ok(ChatOutcome {
                            response: resp,
                            decision: d.clone(),
                            attempts,
                            cost,
                            cache_hit: false,
                        });
                    }
                    Err(e) => {
                        let cancelled = e.code == ErrorCode::Cancelled;
                        self.record(
                            task,
                            &ep,
                            cap,
                            if cancelled {
                                CallStatus::Cancelled
                            } else {
                                CallStatus::Failed
                            },
                            &Default::default(),
                            &Cost::unknown(),
                            t0.elapsed(),
                            attempt,
                            Some(&e),
                            &rid,
                        );
                        attempts.push(Attempt {
                            endpoint_id: ep.id.clone(),
                            attempt,
                            error: Some(e.clone()),
                        });
                        if cancelled {
                            return Err(e);
                        }
                        if e.code.retryable() {
                            self.mark(&ep.id, false);
                        }
                        if e.code.retryable() && attempt < max {
                            if let Some(f) = on {
                                f(StreamNotice::Retry {
                                    attempt: attempt + 1,
                                    reason: e.code.as_str().to_owned(),
                                });
                            }
                            let wait = backoff(&self.retry, attempt, e.retry_after_ms);
                            tokio::select! {
                                () = task.cancel.cancelled() => return Err(ProviderError::cancelled()),
                                () = tokio::time::sleep(wait) => {}
                            }
                            continue;
                        }
                        let eligible = e.code.fallback_eligible();
                        last_err = Some(e);
                        if !eligible {
                            return Err(last_err.unwrap_or_else(|| {
                                ProviderError::new(ErrorCode::NoCapableModel, "no model")
                            }));
                        }
                        break;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            ProviderError::new(
                ErrorCode::NoCapableModel,
                "no model could serve the request",
            )
        }))
    }

    /// Saída estruturada validada por JSON Schema. Usa o recurso nativo quando o endpoint tem
    /// `StructuredOutput`; senão emula (schema no prompt) e valida. Reparo limitado a `max_repairs`.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat_structured(
        &self,
        task: &TaskCtx,
        mut req: ChatRequest,
        schema_name: &str,
        schema: &Value,
        mut opts: ChatOptions,
        max_repairs: u32,
        on: OnNotice<'_>,
    ) -> Result<(Value, ChatOutcome), ProviderError> {
        if !opts.route.also_needs.contains(&Capability::TextGeneration)
            && opts.route.capability != Some(Capability::TextGeneration)
        {
            opts.route.also_needs.push(Capability::TextGeneration);
        }
        // decide nativo × emulado olhando o primeiro candidato
        let (decisions, _) = self.routes(&opts.route)?;
        let first = decisions
            .first()
            .ok_or_else(|| ProviderError::new(ErrorCode::NoCapableModel, "no model"))?;
        let (ep, _) = self.endpoint(&first.endpoint_id)?;
        let native = ep.has(Capability::StructuredOutput);
        if native {
            req.response_schema = Some(ResponseSchema {
                name: schema_name.to_owned(),
                schema: schema.clone(),
            });
        } else {
            req.messages.insert(
                0,
                Message::system(format!(
                    "Reply with ONLY one JSON document (no prose, no code fences) that validates against this JSON Schema:\n{schema}"
                )),
            );
        }
        let mut total = Cost::default();
        let mut tried = 0u32;
        loop {
            let out = self.chat(task, req.clone(), opts.clone(), on).await?;
            total.add(&out.cost);
            match parse_and_validate(&out.response.text, schema) {
                Ok(v) => return Ok((v, ChatOutcome { cost: total, ..out })),
                Err(msg) => {
                    if tried >= max_repairs {
                        return Err(ProviderError::new(
                            ErrorCode::StructuredOutputInvalid,
                            format!(
                                "model output does not match the schema after {tried} repair attempt(s): {msg}"
                            ),
                        ));
                    }
                    tried += 1;
                    req.messages
                        .push(Message::assistant(out.response.text.clone()));
                    req.messages.push(Message {
                        role: Role::User,
                        parts: vec![crate::types::Part::text(format!(
                            "Your previous output was invalid ({msg}). Return ONLY the corrected JSON document."
                        ))],
                    });
                }
            }
        }
    }

    /// STT com roteamento por privacidade (áudio local-only ⇒ nunca nuvem), retry e fallback.
    pub async fn transcribe(
        &self,
        task: &TaskCtx,
        mut req: SttRequest,
        mut route: RouteRequest,
    ) -> Result<SttOutcome, ProviderError> {
        route.capability = Some(Capability::SpeechToText);
        if !route.data.contains(&DataClass::Audio) {
            route.data.push(DataClass::Audio);
        }
        if route.role.is_none() {
            route.role.clone_from(&task.role);
        }
        let (decisions, _) = self.routes(&route)?;
        let mut attempts = Vec::new();
        let mut last_err = None;
        for d in &decisions {
            let (ep, pcfg) = self.endpoint(&d.endpoint_id)?;
            let provider = match self.provider(&pcfg) {
                Ok(p) => p,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            req.model = ep.model_id.clone();
            let max = self.retry.max_attempts.max(1);
            let mut attempt = 0;
            loop {
                attempt += 1;
                if task.cancel.is_cancelled() {
                    return Err(ProviderError::cancelled());
                }
                if let BudgetCheck::Exceeded(m) = task.budget.check(None) {
                    return Err(ProviderError::new(ErrorCode::BudgetExceeded, m));
                }
                let rid = self.rid();
                let t0 = Instant::now();
                let ctx = CallCtx::new(task.cancel.clone());
                let res = tokio::select! {
                    () = task.cancel.cancelled() => Err(ProviderError::cancelled()),
                    r = provider.transcribe(&req, &ctx) => r,
                };
                match res {
                    Ok(t) => {
                        let usage = crate::types::Usage::default();
                        // custo de áudio só com preço por segundo declarado e duração conhecida
                        let cost = match (&ep.pricing, t.duration_us) {
                            (Some(p), Some(us)) if p.audio_micros_per_second.is_some() => Cost {
                                known: true,
                                micros: (u128::from(p.audio_micros_per_second.unwrap_or(0))
                                    * u128::try_from(us.max(0)).unwrap_or(0))
                                .div_ceil(1_000_000) as u64,
                                currency: Some(p.currency.clone()),
                            },
                            _ => Cost::unknown(),
                        };
                        task.budget.charge(&usage, &cost);
                        self.record(
                            task,
                            &ep,
                            Capability::SpeechToText,
                            CallStatus::Ok,
                            &usage,
                            &cost,
                            t0.elapsed(),
                            attempt,
                            None,
                            &rid,
                        );
                        self.mark(&ep.id, true);
                        attempts.push(Attempt {
                            endpoint_id: ep.id.clone(),
                            attempt,
                            error: None,
                        });
                        return Ok(SttOutcome {
                            transcript: t,
                            decision: d.clone(),
                            attempts,
                            cost,
                        });
                    }
                    Err(e) => {
                        let cancelled = e.code == ErrorCode::Cancelled;
                        self.record(
                            task,
                            &ep,
                            Capability::SpeechToText,
                            if cancelled {
                                CallStatus::Cancelled
                            } else {
                                CallStatus::Failed
                            },
                            &Default::default(),
                            &Cost::unknown(),
                            t0.elapsed(),
                            attempt,
                            Some(&e),
                            &rid,
                        );
                        attempts.push(Attempt {
                            endpoint_id: ep.id.clone(),
                            attempt,
                            error: Some(e.clone()),
                        });
                        if cancelled {
                            return Err(e);
                        }
                        if e.code.retryable() {
                            self.mark(&ep.id, false);
                        }
                        if e.code.retryable() && attempt < max {
                            let wait = backoff(&self.retry, attempt, e.retry_after_ms);
                            tokio::select! {
                                () = task.cancel.cancelled() => return Err(ProviderError::cancelled()),
                                () = tokio::time::sleep(wait) => {}
                            }
                            continue;
                        }
                        let eligible = e.code.fallback_eligible();
                        last_err = Some(e);
                        if !eligible {
                            return Err(last_err.unwrap_or_else(|| {
                                ProviderError::new(ErrorCode::NoCapableModel, "no model")
                            }));
                        }
                        break;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            ProviderError::new(ErrorCode::NoCapableModel, "no model could transcribe")
        }))
    }

    /// Sonda um endpoint e grava a evidência (`Probed`) + saúde no registry.
    pub async fn probe_endpoint(
        &self,
        endpoint_id: &str,
        cancel: &CancelToken,
    ) -> Result<ProbeResult, ProviderError> {
        let (ep, pcfg) = self.endpoint(endpoint_id)?;
        let provider = self.provider(&pcfg)?;
        let opts = ProbeOptions::from_declared(&ep.capabilities);
        let res = probe(provider.as_ref(), &ep.model_id, opts, cancel).await;
        let failed: Vec<Capability> = res.failures.keys().copied().collect();
        let verified = res.verified.clone();
        self.update_registry(|r| {
            if let Some(m) = r.models.get_mut(endpoint_id) {
                m.capabilities.apply_probe(&verified, &failed);
                m.last_probe = Some(res.clone());
                m.record_outcome(res.connection_error.is_none());
            }
        });
        Ok(res)
    }

    pub fn declared_capabilities(&self, endpoint_id: &str) -> Option<Capabilities> {
        self.registry
            .read()
            .ok()?
            .models
            .get(endpoint_id)
            .map(|m| m.capabilities.clone())
    }
}

/// Consome o stream propagando eventos à UI, com cancelamento.
async fn drain(
    mut stream: crate::types::ChatStream,
    cancel: &CancelToken,
    on: OnNotice<'_>,
) -> Result<ChatResponse, ProviderError> {
    let mut out = ChatResponse::default();
    loop {
        let ev = tokio::select! {
            () = cancel.cancelled() => return Err(ProviderError::cancelled()),
            ev = stream.next() => ev,
        };
        match ev {
            None => break,
            Some(Err(e)) => return Err(e),
            Some(Ok(e)) => {
                if let Some(f) = on {
                    f(StreamNotice::Event(e.clone()));
                }
                match e {
                    ChatEvent::TextDelta { text } => out.text.push_str(&text),
                    ChatEvent::ToolCallDelta { .. } => {}
                    ChatEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    } => out.tool_calls.push(crate::types::ToolCallOut {
                        id,
                        name,
                        arguments,
                    }),
                    ChatEvent::Usage { usage } => out.usage.add(&usage),
                    ChatEvent::Finish { reason } => out.finish = Some(reason),
                }
            }
        }
    }
    if out.finish.is_none() {
        out.finish = Some(if out.tool_calls.is_empty() {
            crate::types::FinishReason::Stop
        } else {
            crate::types::FinishReason::ToolCalls
        });
    }
    // a collect() pública não é usada aqui para propagar eventos à UI; manter a equivalência:
    let _ = collect;
    Ok(out)
}

/// Extrai o JSON (aceita ```json … ```), valida contra o schema. `Err` ⇒ mensagem curta de motivo.
pub fn parse_and_validate(text: &str, schema: &Value) -> Result<Value, String> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map_or(t, |s| s.trim_start())
        .trim_end_matches("```")
        .trim();
    let v: Value = serde_json::from_str(t).map_err(|e| format!("not valid JSON: {e}"))?;
    validate(&v, schema)?;
    Ok(v)
}

/// Valida `v` contra `schema` (JSON Schema). Mensagens agregadas e curtas.
pub fn validate(v: &Value, schema: &Value) -> Result<(), String> {
    let validator =
        jsonschema::validator_for(schema).map_err(|e| format!("invalid schema: {e}"))?;
    let errs: Vec<String> = validator
        .iter_errors(v)
        .take(5)
        .map(|e| format!("{} at {}", e, e.instance_path))
        .collect();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}
