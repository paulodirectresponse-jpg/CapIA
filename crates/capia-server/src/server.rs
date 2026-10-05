//! Servidor REST `/v1`: aceita conexões (loopback por padrão), roteia para o catálogo e despacha
//! pelo MESMO `Core::call` do MCP. Ciclo de vida: `start` → `shutdown` (gracioso: recusa novas
//! escritas, pausa Runs, espera o que está em voo, persiste e fecha o projeto).

use crate::auth::{Principal, authenticate, now_ms};
use crate::catalog::{self, OpDef, Surface};
use crate::core::{CallCtx, Core, Reply, new_request_id};
use crate::error::ApiErr;
use crate::http::{self, Request, Response};
use crate::mac::random_hex;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Server {
    core: Arc<Core>,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl Server {
    pub fn start(cfg: crate::config::ServerConfig) -> Result<Self, String> {
        let core = Core::open(cfg)?;
        let listener = TcpListener::bind((core.cfg.bind, core.cfg.port))
            .map_err(|e| format!("cannot listen on {}:{}: {e}", core.cfg.bind, core.cfg.port))?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        // publica a porta (sem segredo) para os clientes locais
        let info = json!({"addr": addr.to_string(), "port": addr.port(), "pid": std::process::id(),
                          "version": crate::core::SERVER_VERSION, "api_version": crate::core::API_VERSION});
        let _ = std::fs::write(core.cfg.data_dir.join("server.json"), info.to_string());
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = sync_channel::<TcpStream>(core.cfg.queue);
        let rx = Arc::new(Mutex::new(rx));
        let mut threads = Vec::new();
        for i in 0..core.cfg.workers {
            let (core, rx, stop) = (Arc::clone(&core), Arc::clone(&rx), Arc::clone(&stop));
            threads.push(
                std::thread::Builder::new()
                    .name(format!("capia-http-{i}"))
                    .spawn(move || worker(&core, &rx, &stop, addr.port()))
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let stop = Arc::clone(&stop);
            threads.push(
                std::thread::Builder::new()
                    .name("capia-accept".into())
                    .spawn(move || accept_loop(&listener, &tx, &stop))
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let core = Arc::clone(&core);
            threads.push(
                std::thread::Builder::new()
                    .name("capia-pump".into())
                    .spawn(move || crate::pump::run_pump(&core))
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let core = Arc::clone(&core);
            threads.push(
                std::thread::Builder::new()
                    .name("capia-webhooks".into())
                    .spawn(move || crate::webhooks::run_dispatcher(&core))
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(Self {
            core,
            addr,
            stop,
            threads,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// Shutdown gracioso: (1) recusa novas escritas, (2) pausa as Runs em execução (estado
    /// persistido, retomáveis), (3) espera o que está em voo, (4) para as threads, (5) fecha o projeto.
    pub fn shutdown(mut self) {
        self.core.shutting_down.store(true, Ordering::SeqCst);
        self.core.wake.notify();
        if let Ok(v) = self.core.ai_call("ai.run.list", json!({"limit": 200})) {
            for r in v["runs"].as_array().into_iter().flatten() {
                if r["status"] == "running"
                    && let Some(id) = r["id"].as_str()
                {
                    let _ = self.core.ai_call("ai.run.pause", json!({"run_id": id}));
                }
            }
        }
        let t0 = std::time::Instant::now();
        while self.core.inflight.load(Ordering::SeqCst) > 0
            && t0.elapsed() < Duration::from_secs(20)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        let _ = self.core.session_call("project.close", json!({}));
        let _ = std::fs::remove_file(self.core.cfg.data_dir.join("server.json"));
    }
}

fn accept_loop(l: &TcpListener, tx: &SyncSender<TcpStream>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match l.accept() {
            Ok((s, _)) => {
                let _ = s.set_nonblocking(false);
                let _ = s.set_nodelay(true);
                match tx.try_send(s) {
                    Ok(()) => {}
                    Err(TrySendError::Full(mut s)) => {
                        // backpressure: fila cheia ⇒ 503 imediato, sem criar mais trabalho
                        let body = ApiErr::unavailable(
                            "OVERLOADED",
                            "the server is at capacity; retry shortly",
                        )
                        .with_retry(1)
                        .body("overloaded");
                        let r = Response::json(503, &body).with("Retry-After", "1");
                        let _ = http::write_response(&mut s, &r, false);
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn worker(core: &Arc<Core>, rx: &Arc<Mutex<Receiver<TcpStream>>>, stop: &AtomicBool, port: u16) {
    while !stop.load(Ordering::SeqCst) {
        let next = {
            let g = rx.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            g.recv_timeout(Duration::from_millis(100))
        };
        if let Ok(stream) = next {
            serve_connection(core, stream, stop, port);
        }
    }
}

fn host_allowed(core: &Core, host: &str, port: u16) -> bool {
    let h = host.to_ascii_lowercase();
    h == format!("127.0.0.1:{port}")
        || h == format!("localhost:{port}")
        || h == format!("[::1]:{port}")
        || core
            .cfg
            .allowed_hosts
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&h))
}

fn serve_connection(core: &Arc<Core>, stream: TcpStream, stop: &AtomicBool, port: u16) {
    let Ok(mut write_half) = stream.try_clone() else {
        return;
    };
    let mut reader = http::new_reader(stream);
    for _ in 0..1000 {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let req = match http::read_request(&mut reader) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                let body = ApiErr::new(e.status, e.code, e.message).body("head-error");
                let _ =
                    http::write_response(&mut write_half, &Response::json(e.status, &body), false);
                return;
            }
        };
        let keep = req.keep_alive;
        let outcome = handle(core, &req, &mut reader, &mut write_half, port);
        match outcome {
            Outcome::Respond { resp, keep_open } => {
                if http::write_response(&mut write_half, &resp, keep && keep_open).is_err()
                    || !(keep && keep_open)
                {
                    return;
                }
            }
            Outcome::Streamed => return,
        }
    }
}

enum Outcome {
    /// `keep_open = false` quando o corpo da requisição ficou sem ler (a conexão precisa fechar).
    Respond {
        resp: Response,
        keep_open: bool,
    },
    Streamed,
}

fn err_resp(e: &ApiErr, req_id: &str) -> Response {
    let mut r = Response::json(e.status, &e.body(req_id)).with("X-Request-Id", req_id);
    if let Some(s) = e.retry_after {
        r = r.with("Retry-After", &s.to_string());
    }
    if e.status == 401 {
        r = r.with("WWW-Authenticate", "Bearer realm=\"capia\"");
    }
    r
}

fn respond(resp: Response, keep_open: bool) -> Outcome {
    Outcome::Respond { resp, keep_open }
}

fn fail(e: &ApiErr, req_id: &str, keep_open: bool) -> Outcome {
    respond(err_resp(e, req_id), keep_open)
}

/// Coage os valores de query (strings) para os tipos do schema (integer/boolean).
fn coerce(schema: &Value, k: &str, v: &str) -> Value {
    match schema["properties"][k]["type"].as_str() {
        Some("integer") => v.parse::<i64>().map_or_else(|_| json!(v), |n| json!(n)),
        Some("boolean") => match v {
            "true" => json!(true),
            "false" => json!(false),
            _ => json!(v),
        },
        _ => json!(v),
    }
}

struct Matched {
    def: &'static OpDef,
    path_params: Vec<(String, String)>,
}

fn match_route(method: &str, path: &str) -> Result<Matched, Option<Vec<&'static str>>> {
    let segs: Vec<&str> = path.split('/').collect();
    let mut allowed: Vec<&'static str> = Vec::new();
    for def in catalog::ops()
        .iter()
        .filter(|d| d.surface == Surface::Both || d.name == "uploads.create")
    {
        let tpl: Vec<&str> = def.path.split('/').collect();
        if tpl.len() != segs.len() {
            continue;
        }
        let mut params = Vec::new();
        let mut ok = true;
        for (t, s) in tpl.iter().zip(&segs) {
            if let Some(name) = t.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                if s.is_empty() || s.contains('\0') {
                    ok = false;
                    break;
                }
                params.push((name.to_owned(), (*s).to_owned()));
            } else if t != s {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        if def.method == method {
            return Ok(Matched {
                def,
                path_params: params,
            });
        }
        allowed.push(def.method);
    }
    Err((!allowed.is_empty()).then_some(allowed))
}

fn cors_headers(resp: Response, origin: Option<&str>) -> Response {
    match origin {
        Some(o) => resp
            .with("Access-Control-Allow-Origin", o)
            .with("Vary", "Origin")
            .with(
                "Access-Control-Expose-Headers",
                "X-Request-Id, Retry-After, Idempotent-Replay",
            ),
        None => resp,
    }
}

#[allow(clippy::too_many_lines)]
fn handle(
    core: &Arc<Core>,
    req: &Request,
    reader: &mut http::Reader,
    out: &mut TcpStream,
    port: u16,
) -> Outcome {
    let req_id = req
        .header("x-request-id")
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map_or_else(new_request_id, str::to_owned);
    let body_len = req.content_length;
    let unread = body_len > 0;

    // 1) Host: só o próprio servidor (anti DNS-rebinding)
    match req.header("host") {
        Some(h) if host_allowed(core, h, port) => {}
        _ => {
            return fail(
                &ApiErr::new(
                    421,
                    "HOST_NOT_ALLOWED",
                    "the Host header is not one of this server's addresses",
                ),
                &req_id,
                false,
            );
        }
    }
    // 2) CORS: fechado por padrão. Qualquer `Origin` não listado é negado (CSRF de navegador)
    let origin = req.header("origin");
    let origin_ok = origin.filter(|o| core.cfg.cors_origins.iter().any(|a| a == o));
    if origin.is_some() && origin_ok.is_none() {
        return fail(
            &ApiErr::forbidden(
                "CORS_DENIED",
                "cross-origin requests are not allowed for this origin",
            ),
            &req_id,
            false,
        );
    }
    if req.method == "OPTIONS" {
        let r = cors_headers(Response::empty(204), origin_ok)
            .with("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE")
            .with(
                "Access-Control-Allow-Headers",
                "authorization, content-type, idempotency-key, x-capia-filename, x-capia-sha256, x-request-id",
            )
            .with("Access-Control-Max-Age", "600");
        return respond(r, !unread);
    }
    // 3) rotas fora do catálogo
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/openapi.json") => {
            return respond(
                cors_headers(Response::json(200, &crate::openapi::document()), origin_ok),
                !unread,
            );
        }
        ("GET", "/v1/events/stream") => {
            return stream_events(core, req, out, &req_id);
        }
        ("POST", "/mcp") => {
            return handle_mcp(core, req, reader, &req_id);
        }
        _ => {}
    }
    // 4) autenticação (tudo, exceto o health, exige bearer)
    let matched = match match_route(&req.method, &req.path) {
        Ok(m) => m,
        Err(Some(allowed)) => {
            let e = ApiErr::new(
                405,
                "METHOD_NOT_ALLOWED",
                "this route does not support this method",
            );
            return respond(
                err_resp(&e, &req_id).with("Allow", &allowed.join(", ")),
                !unread,
            );
        }
        Err(None) => return fail(&ApiErr::not_found("no such route"), &req_id, !unread),
    };
    let def = matched.def;
    let principal = match bearer(req) {
        Some(tok) => match authenticate(&core.db, tok) {
            Ok(p) => Some(p),
            Err(e) => return fail(&e, &req_id, !unread),
        },
        None => None,
    };
    if def.scope.is_some() && principal.is_none() {
        return fail(
            &ApiErr::unauthorized("missing, invalid, expired or revoked token"),
            &req_id,
            !unread,
        );
    }
    let idem = match req.header("idempotency-key") {
        Some(k)
            if k.is_empty() || k.len() > 128 || !k.bytes().all(|b| (0x21..0x7f).contains(&b)) =>
        {
            return fail(
                &ApiErr::bad_request("invalid Idempotency-Key header"),
                &req_id,
                !unread,
            );
        }
        Some(k) => Some(k.to_owned()),
        None => None,
    };
    let ctx = CallCtx {
        principal,
        surface: "rest",
        request_id: req_id.clone(),
        idempotency_key: idem,
    };
    // 5) upload em streaming: o corpo NÃO é lido para a memória
    if def.name == "uploads.create" {
        return upload(core, req, reader, &ctx, &req_id, origin_ok);
    }
    // 6) corpo JSON com teto, profundidade limitada e Content-Type exigido
    let mut params = Map::new();
    if body_len > 0 {
        if body_len > core.cfg.max_json_bytes as u64 {
            return fail(
                &ApiErr::new(
                    413,
                    "PAYLOAD_TOO_LARGE",
                    format!(
                        "JSON bodies are limited to {} bytes",
                        core.cfg.max_json_bytes
                    ),
                ),
                &req_id,
                false,
            );
        }
        if !req
            .header("content-type")
            .is_some_and(|c| c.to_ascii_lowercase().starts_with("application/json"))
        {
            return fail(
                &ApiErr::new(
                    415,
                    "UNSUPPORTED_MEDIA_TYPE",
                    "send Content-Type: application/json",
                ),
                &req_id,
                http::drain(reader, body_len),
            );
        }
        let body = match http::read_body(
            reader,
            usize::try_from(body_len).unwrap_or(0),
            Duration::from_secs(10),
        ) {
            Ok(b) => b,
            Err(e) => return fail(&ApiErr::new(e.status, e.code, e.message), &req_id, false),
        };
        if http::json_depth_exceeds(&body, core.cfg.max_json_depth) {
            return fail(
                &ApiErr::bad_request("the JSON is nested too deeply")
                    .with_details(json!({"max_depth": core.cfg.max_json_depth})),
                &req_id,
                true,
            );
        }
        match serde_json::from_slice::<Value>(&body) {
            Ok(Value::Object(m)) => params = m,
            Ok(_) => {
                return fail(
                    &ApiErr::bad_request("the JSON body must be an object"),
                    &req_id,
                    true,
                );
            }
            Err(e) => {
                return fail(
                    &ApiErr::new(400, "BAD_JSON", format!("malformed JSON: {e}")),
                    &req_id,
                    true,
                );
            }
        }
    }
    for (k, v) in &req.query {
        params.insert(k.clone(), coerce(&def.schema, k, v));
    }
    // os parâmetros de caminho valem sobre o corpo (o recurso é o da URL)
    for (k, v) in &matched.path_params {
        params.insert(k.clone(), coerce(&def.schema, k, v));
    }
    let result = core.call(&ctx, def.name, Value::Object(params));
    match result {
        Ok(Reply {
            status,
            body,
            replayed,
        }) => {
            let mut r = cors_headers(Response::json(status, &body), origin_ok)
                .with("X-Request-Id", &req_id);
            if replayed {
                r = r.with("Idempotent-Replay", "true");
            }
            respond(r, true)
        }
        Err(e) => respond(cors_headers(err_resp(&e, &req_id), origin_ok), true),
    }
}

fn bearer(req: &Request) -> Option<&str> {
    let h = req.header("authorization")?;
    let (scheme, tok) = h.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(tok.trim())
}

fn upload(
    core: &Arc<Core>,
    req: &Request,
    reader: &mut http::Reader,
    ctx: &CallCtx,
    req_id: &str,
    origin_ok: Option<&str>,
) -> Outcome {
    // sem Content-Length não há como impor teto antes de ler: exigido
    if req.content_length == 0 {
        return fail(
            &ApiErr::new(
                411,
                "LENGTH_REQUIRED",
                "uploads need a non-zero Content-Length",
            ),
            req_id,
            true,
        );
    }
    let Some(filename) = req
        .header("x-capia-filename")
        .and_then(http::percent_decode)
        .filter(|f| !f.is_empty() && f.len() <= 255)
    else {
        return fail(
            &ApiErr::bad_request(
                "send the original file name in the X-Capia-Filename header (percent-encoded)",
            ),
            req_id,
            false,
        );
    };
    let expected = req.header("x-capia-sha256").map(str::to_ascii_lowercase);
    // o gate (auth/scope/rate) roda ANTES de ler um byte do corpo; se recusar, a conexão fecha
    let mut limited = reader.by_ref().take(req.content_length);
    // o prazo de leitura do corpo é o do upload (o tempo máximo é aplicado também dentro do loop)
    limited.get_mut().get_mut().deadline = std::time::Instant::now() + core.cfg.upload_timeout;
    match core.upload_stream(
        ctx,
        &filename,
        expected.as_deref(),
        req.content_length,
        &mut limited,
    ) {
        Ok(Reply {
            status,
            body,
            replayed,
        }) => {
            // o corpo pode ter sido lido só em parte (erro no meio): conexão limpa só se zerou
            let clean = limited.limit() == 0;
            let mut r =
                cors_headers(Response::json(status, &body), origin_ok).with("X-Request-Id", req_id);
            if replayed {
                r = r.with("Idempotent-Replay", "true");
            }
            respond(r, clean)
        }
        Err(e) => respond(cors_headers(err_resp(&e, req_id), origin_ok), false),
    }
}

fn handle_mcp(core: &Arc<Core>, req: &Request, reader: &mut http::Reader, req_id: &str) -> Outcome {
    let unread = req.content_length > 0;
    let principal: Option<Principal> = match bearer(req) {
        Some(t) => match authenticate(&core.db, t) {
            Ok(p) => Some(p),
            Err(e) => return fail(&e, req_id, !unread),
        },
        None => {
            return fail(
                &ApiErr::unauthorized("missing, invalid, expired or revoked token"),
                req_id,
                !unread,
            );
        }
    };
    if req.content_length == 0 || req.content_length > core.cfg.max_json_bytes as u64 {
        return fail(
            &ApiErr::new(413, "PAYLOAD_TOO_LARGE", "invalid MCP request size"),
            req_id,
            false,
        );
    }
    let body = match http::read_body(
        reader,
        usize::try_from(req.content_length).unwrap_or(0),
        Duration::from_secs(10),
    ) {
        Ok(b) => b,
        Err(e) => return fail(&ApiErr::new(e.status, e.code, e.message), req_id, false),
    };
    if http::json_depth_exceeds(&body, core.cfg.max_json_depth) {
        let v = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "JSON nested too deeply"}});
        return respond(Response::json(200, &v), true);
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let v = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}});
            return respond(Response::json(200, &v), true);
        }
    };
    match crate::mcp::handle_rpc(core, principal.as_ref(), &msg) {
        Some(v) => respond(Response::json(200, &v).with("X-Request-Id", req_id), true),
        None => respond(Response::empty(202), true),
    }
}

struct SseGuard<'a>(&'a Core);
impl Drop for SseGuard<'_> {
    fn drop(&mut self) {
        self.0.sse_active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn stream_events(core: &Arc<Core>, req: &Request, out: &mut TcpStream, req_id: &str) -> Outcome {
    use std::io::Write as _;
    let principal = match bearer(req).map(|t| authenticate(&core.db, t)) {
        Some(Ok(p)) => p,
        Some(Err(e)) => return fail(&e, req_id, false),
        None => {
            return fail(
                &ApiErr::unauthorized("missing, invalid, expired or revoked token"),
                req_id,
                false,
            );
        }
    };
    if !principal.has(crate::scope::Scope::ProjectRead) {
        return fail(
            &ApiErr::forbidden(
                "INSUFFICIENT_SCOPE",
                "the event stream needs the `project:read` scope",
            ),
            req_id,
            false,
        );
    }
    if core.sse_active.fetch_add(1, Ordering::SeqCst) >= core.cfg.max_sse {
        core.sse_active.fetch_sub(1, Ordering::SeqCst);
        return fail(
            &ApiErr::new(429, "TOO_MANY_STREAMS", "too many open event streams").with_retry(5),
            req_id,
            false,
        );
    }
    let _g = SseGuard(core);
    let mut last: i64 = req
        .header("last-event-id")
        .and_then(|v| v.parse().ok())
        .or_else(|| {
            req.query
                .iter()
                .find(|(k, _)| k == "after")
                .and_then(|(_, v)| v.parse().ok())
        })
        .unwrap_or_else(|| core.db.last_event_seq().unwrap_or(0));
    let headers = vec![
        ("Content-Type".to_owned(), "text/event-stream".to_owned()),
        ("X-Request-Id".to_owned(), req_id.to_owned()),
    ];
    if http::write_stream_head(out, &headers).is_err() {
        return Outcome::Streamed;
    }
    let mut idle = 0u32;
    loop {
        if core.shutting_down.load(Ordering::SeqCst) {
            let _ = out.write_all(b"event: shutdown\ndata: {}\n\n");
            break;
        }
        match core.db.events_after(last, 100) {
            Ok(rows) if !rows.is_empty() => {
                idle = 0;
                for e in rows {
                    last = e.seq;
                    let data = serde_json::to_string(&e).unwrap_or_default();
                    let frame = format!("id: {}\nevent: {}\ndata: {data}\n\n", e.seq, e.kind);
                    if out.write_all(frame.as_bytes()).is_err() {
                        return Outcome::Streamed;
                    }
                }
                if out.flush().is_err() {
                    return Outcome::Streamed;
                }
            }
            Ok(_) => {
                idle += 1;
                // comentário de batimento a cada ~15 s: detecta cliente que sumiu
                if idle >= 60 {
                    idle = 0;
                    if out
                        .write_all(b": ping\n\n")
                        .and_then(|()| out.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Outcome::Streamed
}

/// Lê o arquivo-guia de descoberta (`server.json`) de um diretório de dados (para clientes locais).
pub fn read_server_info(data_dir: &std::path::Path) -> Option<Value> {
    let mut s = String::new();
    std::fs::File::open(data_dir.join("server.json"))
        .ok()?
        .read_to_string(&mut s)
        .ok()?;
    serde_json::from_str(&s).ok()
}

/// Valores únicos de `Idempotency-Key` aleatórios (utilitário de clientes/testes).
pub fn new_idempotency_key() -> String {
    random_hex(12).unwrap_or_else(|_| now_ms().to_string())
}

/// Conjunto de scopes distintos usados pelo catálogo (para documentação).
pub fn catalog_scopes() -> BTreeSet<&'static str> {
    catalog::ops()
        .iter()
        .filter_map(|o| o.scope.map(crate::scope::Scope::as_str))
        .collect()
}
