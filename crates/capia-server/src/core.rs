//! Núcleo do servidor: estado compartilhado e o **pipeline único de chamada** (`Core::call`) que
//! REST e MCP usam. Nenhum transporte escreve no documento por outro caminho: autenticação →
//! scope → validação de schema → rate limit → gate de projeto/shutdown → idempotência → handler
//! (que só fala com a Engine API) → auditoria. UI, REST e MCP são clientes da MESMA Engine API.

use crate::auth::{Principal, now_ms};
use crate::catalog::{self, Class, OpDef};
use crate::config::ServerConfig;
use crate::error::{ApiErr, ApiResult};
use crate::mac::{random_hex, sha256_hex};
use crate::ratelimit::RateLimiter;
use capia_ai::webhook::WebhookClient;
use capia_commands::Actor;
use capia_editor_api::{Session, SessionConfig};
use capia_intelligence::service::ServiceConfig;
use capia_intelligence::{IntelligenceService, SessionEngine};
use capia_store::{AuditRow, IdemBegin, ServerDb};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

pub const API_VERSION: &str = "v1";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Quem chama e por qual superfície (para auditoria).
#[derive(Clone, Debug)]
pub struct CallCtx {
    pub principal: Option<Principal>,
    /// `rest` | `mcp` | `internal`
    pub surface: &'static str,
    pub request_id: String,
    pub idempotency_key: Option<String>,
}

impl CallCtx {
    pub fn internal() -> Self {
        Self {
            principal: None,
            surface: "internal",
            request_id: new_request_id(),
            idempotency_key: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
    /// Resposta repetida de uma `Idempotency-Key` já concluída.
    pub replayed: bool,
}

pub fn new_request_id() -> String {
    format!(
        "req_{}",
        random_hex(8).unwrap_or_else(|_| "00000000".into())
    )
}

/// Sinal para o despachante de webhooks acordar (evento novo / reentrega).
#[derive(Debug, Default)]
pub struct Wake {
    pub flag: Mutex<u64>,
    pub cv: Condvar,
}

impl Wake {
    pub fn notify(&self) {
        let mut g = self
            .flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *g = g.wrapping_add(1);
        self.cv.notify_all();
    }

    /// Espera até `timeout` ou um `notify`; devolve o novo contador.
    pub fn wait(&self, seen: u64, timeout: Duration) -> u64 {
        let g = self
            .flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |v| *v == seen)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *g
    }

    pub fn current(&self) -> u64 {
        *self
            .flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Contadores locais (sem rótulos de alta cardinalidade; nada de token/projeto).
#[derive(Debug, Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub writes: AtomicU64,
    pub errors_4xx: AtomicU64,
    pub errors_5xx: AtomicU64,
    pub rate_limited: AtomicU64,
    pub replays: AtomicU64,
}

pub struct Core {
    pub cfg: ServerConfig,
    pub db: Arc<ServerDb>,
    pub session: Arc<Mutex<Session>>,
    pub ai: Arc<IntelligenceService>,
    pub limiter: RateLimiter,
    pub webhook_client: Arc<WebhookClient>,
    pub started: Instant,
    pub shutting_down: AtomicBool,
    pub open_project: Mutex<Option<String>>,
    /// Trocar o projeto aberto (create/open/close) exclui todas as chamadas por projeto em voo:
    /// uma chamada validada para P1 nunca roda contra P2.
    project_switch: RwLock<()>,
    pub wake: Wake,
    pub inflight: AtomicUsize,
    pub uploads_active: AtomicUsize,
    pub sse_active: AtomicUsize,
    pub metrics: Metrics,
    validators: HashMap<&'static str, jsonschema::Validator>,
}

impl core::fmt::Debug for Core {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

/// Operações cuja resposta carrega um segredo mostrado uma única vez: o segredo **nunca** entra na
/// tabela de idempotência (um replay devolve a resposta sem ele).
const ONE_TIME_SECRET_OPS: &[&str] = &[
    "tokens.create",
    "tokens.rotate",
    "webhooks.create",
    "webhooks.rotate_secret",
];

impl Core {
    pub fn open(cfg: ServerConfig) -> Result<Arc<Self>, String> {
        cfg.validate()?;
        std::fs::create_dir_all(&cfg.data_dir)
            .map_err(|e| format!("cannot create the data directory: {e}"))?;
        for sub in ["projects", "uploads", "exports"] {
            std::fs::create_dir_all(cfg.data_dir.join(sub))
                .map_err(|e| format!("cannot create {sub}/: {e}"))?;
        }
        let db = Arc::new(
            ServerDb::open(&cfg.data_dir.join("server.db"), Duration::from_secs(5))
                .map_err(|e| format!("cannot open the server database: {}", e.message))?,
        );
        let now = now_ms();
        // reabertura após queda: entregas em voo voltam a `retrying`; exports interrompidos falham
        let _ = db.deliveries_recover(now);
        let _ = db.exports_recover(now);
        // chaves de idempotência "em andamento" de um processo que caiu: indeterminadas já, não
        // só depois de `idempotency_stale`; e as concluídas velhas não crescem para sempre
        let _ = db.idem_recover();
        let _ = db.idem_purge_older_than(now.saturating_sub(24 * 3600 * 1000));
        let session = Arc::new(Mutex::new(Session::new(SessionConfig {
            actor: Some(Actor::new(
                capia_commands::ActorKind::System,
                "capia-server",
            )),
            ..cfg.session.clone()
        })));
        let ai = Arc::new(
            IntelligenceService::new(
                Arc::new(SessionEngine::new(Arc::clone(&session))),
                ServiceConfig {
                    appdb_path: Some(cfg.data_dir.join("app.db")),
                    secrets: Arc::clone(&cfg.secrets),
                },
            )
            .map_err(|e| format!("cannot start the intelligence service: {}", e.message))?,
        );
        #[cfg(feature = "testkit")]
        if cfg.demo_brain {
            ai.install_demo_autonomy();
        }
        let webhook_client = Arc::new(
            WebhookClient::new(cfg.webhook)
                .map_err(|e| format!("webhook client: {}", e.message))?,
        );
        let mut validators = HashMap::new();
        for o in catalog::ops() {
            let v = jsonschema::validator_for(&o.schema)
                .map_err(|e| format!("schema of {}: {e}", o.name))?;
            validators.insert(o.name, v);
        }
        Ok(Arc::new(Self {
            limiter: RateLimiter::new(cfg.rate_scale),
            cfg,
            db,
            session,
            ai,
            webhook_client,
            started: Instant::now(),
            shutting_down: AtomicBool::new(false),
            open_project: Mutex::new(None),
            project_switch: RwLock::new(()),
            wake: Wake::default(),
            inflight: AtomicUsize::new(0),
            uploads_active: AtomicUsize::new(0),
            sse_active: AtomicUsize::new(0),
            metrics: Metrics::default(),
            validators,
        }))
    }

    pub fn lock_session(&self) -> MutexGuard<'_, Session> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn open_project_id(&self) -> Option<String> {
        self.open_project
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_open_project(&self, id: Option<String>) {
        *self
            .open_project
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = id;
    }

    /// Um `Actor::Api` por token: o plan token do engine só vale para quem fez o preview.
    pub fn actor_of(ctx: &CallCtx) -> ApiResult<Actor> {
        ctx.principal
            .as_ref()
            .map(Principal::actor)
            .ok_or_else(|| ApiErr::unauthorized("authentication required"))
    }

    // ---- pipeline ---------------------------------------------------------------------------

    /// Etapas comuns **antes** do handler: scope, schema, rate limit, shutdown e gate de projeto.
    fn gate(&self, ctx: &CallCtx, def: &'static OpDef, params: &Value) -> ApiResult<()> {
        if let Some(scope) = def.scope {
            let p = ctx
                .principal
                .as_ref()
                .ok_or_else(|| ApiErr::unauthorized("authentication required"))?;
            if !p.has(scope) {
                return Err(ApiErr::forbidden(
                    "INSUFFICIENT_SCOPE",
                    format!("this operation needs the `{}` scope", scope.as_str()),
                )
                .with_details(json!({"required_scope": scope.as_str(), "operation": def.name})));
            }
        }
        if let Some(v) = self.validators.get(def.name) {
            let errs: Vec<String> = v
                .iter_errors(params)
                .take(3)
                .map(|e| format!("{} (at `{}`)", e, e.instance_path))
                .collect();
            if !errs.is_empty() {
                return Err(
                    ApiErr::invalid(errs.join("; ")).with_details(json!({"operation": def.name}))
                );
            }
        }
        if let Some(p) = ctx.principal.as_ref()
            && let Err(wait) = self.limiter.check(&p.token_id, def.class)
        {
            return Err(ApiErr::new(
                429,
                "RATE_LIMITED",
                format!("too many {} requests; retry in {wait}s", def.class.as_str()),
            )
            .with_retry(wait)
            .with_details(json!({"class": def.class.as_str()})));
        }
        if def.mutating && self.shutting_down.load(Ordering::SeqCst) {
            return Err(ApiErr::unavailable(
                "SHUTTING_DOWN",
                "the server is shutting down; writes are refused",
            )
            .with_retry(5));
        }
        if def.project {
            let want = params["project_id"].as_str().unwrap_or_default();
            let open = self.open_project_id();
            if open.as_deref() != Some(want) {
                return Err(match self.db.project_get(want)? {
                    None => ApiErr::not_found(format!("project `{want}` does not exist")),
                    Some(_) => ApiErr::conflict(
                        "PROJECT_NOT_OPEN",
                        format!(
                            "project `{want}` is not open; POST /v1/projects/{want}/open first"
                        ),
                    ),
                });
            }
        }
        Ok(())
    }

    /// Chamada única de uma operação do catálogo (REST, MCP e testes).
    pub fn call(&self, ctx: &CallCtx, name: &str, params: Value) -> ApiResult<Reply> {
        let def = catalog::find(name).ok_or_else(|| {
            ApiErr::new(
                404,
                "UNKNOWN_OPERATION",
                format!("unknown operation `{name}`"),
            )
        })?;
        if def.name == "uploads.create" {
            return Err(ApiErr::new(
                400,
                "STREAM_ONLY",
                "uploads.create is a streaming transport: use POST /v1/uploads (or uploads.create_inline)",
            ));
        }
        self.call_def(ctx, def, params, |c, d, p| c.handle(ctx, d, p))
    }

    /// Mesmo pipeline com um executor próprio (o upload em streaming passa o leitor por aqui).
    pub fn call_def<F>(
        &self,
        ctx: &CallCtx,
        def: &'static OpDef,
        params: Value,
        run: F,
    ) -> ApiResult<Reply>
    where
        F: FnOnce(&Self, &'static OpDef, Value) -> ApiResult<Value>,
    {
        let started = now_ms();
        // a chave do cliente nunca é gravada em claro (banco/auditoria): só um digest dela
        let key = ctx
            .idempotency_key
            .as_deref()
            .filter(|_| def.mutating)
            .map(|k| format!("ik_{}", &sha256_hex(k.as_bytes())[..40]));
        let token_id = ctx.principal.as_ref().map(|p| p.token_id.clone());
        let mut audit = AuditRow {
            at_ms: started,
            request_id: ctx.request_id.clone(),
            token_id: token_id.clone(),
            surface: ctx.surface.to_owned(),
            op: def.name.to_owned(),
            mutating: def.mutating,
            idempotency_key: key.clone(),
            project_id: params["project_id"].as_str().map(str::to_owned),
            ..AuditRow::default()
        };
        let gated = self.gate(ctx, def, &params);
        let result = match gated {
            Err(e) => Err(e),
            Ok(()) => self.guarded(
                ctx,
                def,
                &params,
                key.as_deref(),
                token_id.as_deref(),
                &mut audit,
                run,
            ),
        };
        self.metrics.requests.fetch_add(1, Ordering::Relaxed);
        if def.mutating {
            self.metrics.writes.fetch_add(1, Ordering::Relaxed);
        }
        match &result {
            Ok(r) => {
                if r.replayed {
                    self.metrics.replays.fetch_add(1, Ordering::Relaxed);
                }
                audit.outcome = if r.replayed { "replayed" } else { "ok" }.to_owned();
            }
            Err(e) => {
                match e.status {
                    429 => self.metrics.rate_limited.fetch_add(1, Ordering::Relaxed),
                    400..=499 => self.metrics.errors_4xx.fetch_add(1, Ordering::Relaxed),
                    _ => self.metrics.errors_5xx.fetch_add(1, Ordering::Relaxed),
                };
                audit.outcome = "error".to_owned();
                audit.code = Some(e.code.clone());
            }
        }
        // a auditoria cobre TODA chamada autenticada (e as negadas); falha de auditoria nunca
        // derruba a resposta, mas é visível no stderr redigido
        // auditoria: toda escrita e toda recusa/erro (leituras bem-sucedidas não viram escrita no
        // banco); o log é aparado para não crescer sem limite
        if def.mutating || result.is_err() {
            match self.db.audit_append(&audit) {
                Ok(seq) if seq % 1000 == 0 => {
                    let _ = self.db.audit_trim(100_000);
                }
                Ok(_) => {}
                Err(e) => eprintln!("capia-server: audit write failed: {}", e.message),
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn guarded<F>(
        &self,
        _ctx: &CallCtx,
        def: &'static OpDef,
        params: &Value,
        key: Option<&str>,
        token_id: Option<&str>,
        audit: &mut AuditRow,
        run: F,
    ) -> ApiResult<Reply>
    where
        F: FnOnce(&Self, &'static OpDef, Value) -> ApiResult<Value>,
    {
        // idempotência por (token, chave): replay devolve o mesmo resultado semântico
        let hash = sha256_hex(format!("{}\n{}", def.name, params).as_bytes());
        if let (Some(key), Some(tid)) = (key, token_id) {
            match self.db.idem_begin(
                tid,
                key,
                def.name,
                &hash,
                now_ms(),
                u64::try_from(self.cfg.idempotency_stale.as_millis()).unwrap_or(u64::MAX),
            )? {
                IdemBegin::New => {}
                IdemBegin::Replay { status, body } => {
                    let mut body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                    if ONE_TIME_SECRET_OPS.contains(&def.name) {
                        body["secret_unavailable_on_replay"] = json!(true);
                    }
                    return Ok(Reply {
                        status,
                        body,
                        replayed: true,
                    });
                }
                IdemBegin::Mismatch => {
                    return Err(ApiErr::new(
                        422,
                        "IDEMPOTENCY_KEY_REUSED",
                        "this Idempotency-Key was used with a different request",
                    ));
                }
                IdemBegin::InProgress => {
                    return Err(ApiErr::conflict(
                        "IDEMPOTENCY_IN_PROGRESS",
                        "a request with this Idempotency-Key is still running",
                    )
                    .with_retry(1));
                }
                IdemBegin::Indeterminate => {
                    return Err(ApiErr::conflict(
                        "IDEMPOTENCY_INDETERMINATE",
                        "a previous request with this Idempotency-Key did not finish (the server \
                         stopped); verify the resource state and retry with a NEW key",
                    ));
                }
            }
        }
        let rev_before = if def.mutating {
            self.lock_session().revision()
        } else {
            None
        };
        audit.revision_before = rev_before;
        self.inflight.fetch_add(1, Ordering::SeqCst);
        let switching = matches!(
            def.name,
            "projects.create" | "projects.open" | "projects.close"
        );
        let out = if switching {
            let _w = self
                .project_switch
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            run(self, def, params.clone())
        } else if def.project {
            let _r = self
                .project_switch
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // reconfere sob o lock de leitura: o projeto pode ter trocado entre o gate e aqui
            if self.open_project_id().as_deref() == params["project_id"].as_str() {
                run(self, def, params.clone())
            } else {
                Err(ApiErr::conflict(
                    "PROJECT_NOT_OPEN",
                    "the project was closed or switched while the request was in flight",
                ))
            }
        } else {
            run(self, def, params.clone())
        };
        self.inflight.fetch_sub(1, Ordering::SeqCst);
        if def.mutating {
            audit.revision_after = self.lock_session().revision();
        }
        match out {
            Ok(body) => {
                let reply = Reply {
                    status: def.status,
                    body,
                    replayed: false,
                };
                if let (Some(key), Some(tid)) = (key, token_id) {
                    let mut stored = reply.body.clone();
                    if ONE_TIME_SECRET_OPS.contains(&def.name) {
                        strip_secret(&mut stored);
                    }
                    let _ = self
                        .db
                        .idem_finish(tid, key, reply.status, &stored.to_string());
                }
                Ok(reply)
            }
            Err(e) => {
                if let (Some(key), Some(tid)) = (key, token_id) {
                    // erro determinístico do pedido (4xx) fica guardado; transitório libera a chave
                    if (400..500).contains(&e.status) && e.status != 409 && e.status != 429 {
                        let _ = self.db.idem_finish(
                            tid,
                            key,
                            e.status,
                            &e.body("replayed").to_string(),
                        );
                    } else {
                        let _ = self.db.idem_release(tid, key);
                    }
                }
                Err(e)
            }
        }
    }

    // ---- utilidades dos handlers ------------------------------------------------------------

    pub fn session_call(&self, method: &str, params: Value) -> ApiResult<Value> {
        let reply = self.lock_session().call(method, params)?;
        match reply {
            capia_editor_api::Reply::Json(v) => Ok(v),
            capia_editor_api::Reply::Binary { .. } => {
                Err(ApiErr::internal("unexpected binary reply from the engine"))
            }
        }
    }

    pub fn ai_call(&self, method: &str, params: Value) -> ApiResult<Value> {
        self.ai
            .call_json(method, params)
            .map_err(|e| ApiErr::from_json(&e))
    }

    /// Registra um evento do servidor e acorda o despachante.
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &self,
        event_id: &str,
        kind: &str,
        project_id: Option<&str>,
        run_id: Option<&str>,
        export_id: Option<&str>,
        data: &Value,
    ) {
        match self.db.event_append(
            event_id,
            kind,
            now_ms(),
            project_id,
            run_id,
            export_id,
            data,
            None,
        ) {
            Ok(Some(_)) => self.wake.notify(),
            Ok(None) => {}
            Err(e) => eprintln!("capia-server: event write failed: {}", e.message),
        }
    }
}

fn strip_secret(v: &mut Value) {
    if let Some(o) = v.as_object_mut() {
        o.remove("secret");
    }
}

/// Classe → para o limite de concorrência de uploads etc.
pub fn is_upload(c: Class) -> bool {
    c == Class::Upload
}
