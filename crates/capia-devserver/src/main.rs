//! Servidor local de **desenvolvimento e E2E** (nunca distribuído): `POST /api/<método>` com corpo
//! JSON chama `capia_editor_api::Session`; quadros/miniaturas voltam como bytes com os metadados em
//! `X-Capia-Meta`. Qualquer outro `GET` serve arquivos do front compilado.
//!
//! Segurança: escuta só em `127.0.0.1`; com `CAPIA_DEVSERVER_TOKEN` exige o cabeçalho
//! `x-capia-token`. O produto usa o shell Tauri (IPC tipado), não este servidor.
//!
//! HTTP/1.1 mínimo sobre `std::net` (uma thread por conexão, keep-alive, `TCP_NODELAY`, resposta
//! numa única escrita): com um servidor que escreve cabeçalho e corpo em segmentos separados, o
//! Nagle + ACK atrasado somavam ~40 ms a respostas pequenas em conexões reutilizadas e
//! contaminavam as medições de latência do E2E.
//!
//! Uso: `capia-devserver [--port 5199] [--static apps/desktop/dist]`.

use capia_editor_api::{Outcome, Reply, Session, SessionConfig};
use capia_intelligence::{IntelligenceService, SessionEngine};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_BODY: usize = 64 << 20;
const MAX_HEADER_LINE: usize = 16 << 10;

struct Req {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    keep_alive: bool,
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![
                ("Content-Type".into(), content_type.into()),
                ("Cache-Control".into(), "no-store".into()),
            ],
            body,
        }
    }

    fn json(status: u16, v: &Value) -> Self {
        Self::new(status, "application/json", v.to_string().into_bytes())
    }

    fn text(status: u16, s: &str) -> Self {
        Self::new(status, "text/plain; charset=utf-8", s.as_bytes().to_vec())
    }

    fn with(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        422 => "Unprocessable Entity",
        _ => "Internal Server Error",
    }
}

/// Cabeçalho + corpo numa única escrita (um segmento TCP quando cabe).
fn write_response(stream: &mut TcpStream, resp: &Resp, keep_alive: bool) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(resp.body.len() + 256);
    out.extend_from_slice(
        format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status)).as_bytes(),
    );
    for (k, v) in &resp.headers {
        // valores vêm de constantes/JSON nossos; CR/LF nunca entram num cabeçalho
        let v = v.replace(['\r', '\n'], " ");
        out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n", resp.body.len()).as_bytes());
    out.extend_from_slice(if keep_alive {
        b"Connection: keep-alive\r\n\r\n"
    } else {
        b"Connection: close\r\n\r\n"
    });
    out.extend_from_slice(&resp.body);
    stream.write_all(&out)?;
    stream.flush()
}

fn read_req(reader: &mut BufReader<TcpStream>) -> Option<Req> {
    let mut line = String::new();
    if reader
        .by_ref()
        .take(MAX_HEADER_LINE as u64)
        .read_line(&mut line)
        .ok()?
        == 0
    {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let url = parts.next()?.to_owned();
    let version = parts.next().unwrap_or("HTTP/1.0").to_owned();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    let mut connection = String::new();
    loop {
        let mut h = String::new();
        if reader
            .by_ref()
            .take(MAX_HEADER_LINE as u64)
            .read_line(&mut h)
            .ok()?
            == 0
        {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
        match k.as_str() {
            "content-length" => content_length = v.parse().ok()?,
            "connection" => connection = v.to_ascii_lowercase(),
            _ => {}
        }
        headers.push((k, v));
    }
    if content_length > MAX_BODY {
        return None;
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;
    let keep_alive = if version == "HTTP/1.1" {
        connection != "close"
    } else {
        connection == "keep-alive"
    };
    Some(Req {
        method,
        url,
        headers,
        body,
        keep_alive,
    })
}

fn mime_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Resolve `url` dentro de `root` recusando qualquer `..`/absoluto (sem path traversal).
fn static_path(root: &Path, url: &str) -> Option<PathBuf> {
    let rel = url.split('?').next().unwrap_or("").trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let mut out = root.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => out.push(p),
            _ => return None,
        }
    }
    Some(out)
}

/// Estado do servidor: a sessão do editor e o serviço `ai.*` (ligados ao MESMO projeto).
struct App {
    session: Arc<Mutex<Session>>,
    ai: IntelligenceService,
}

fn api(app: &App, token: Option<&str>, method: &str, req: &Req) -> Resp {
    let session = &app.session;
    if let Some(t) = token
        && !req
            .headers
            .iter()
            .any(|(k, v)| k == "x-capia-token" && v == t)
    {
        return Resp::json(
            401,
            &json!({"code":"UNAUTHORIZED","message":"missing or wrong token"}),
        );
    }
    if req.method != "POST" {
        return Resp::json(
            405,
            &json!({"code":"METHOD_NOT_ALLOWED","message":"use POST"}),
        );
    }
    let params: Value = if req.body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(e) => {
                return Resp::json(400, &json!({"code":"BAD_JSON","message":e.to_string()}));
            }
        }
    };
    // `ai.*` vai ao serviço de inteligência (credenciais write-only; nada de segredo na resposta)
    if IntelligenceService::handles(method) {
        return match app.ai.call_json(method, params) {
            Ok(v) => Resp::json(200, &v),
            Err(e) => Resp::json(422, &e),
        };
    }
    let t_start = Instant::now();
    // fase 1 sob o lock; o trabalho pesado (quadros) roda fora dele: comandos não esperam render
    let begun = match session.lock() {
        Ok(mut s) => s.begin(method, params),
        Err(_) => {
            return Resp::json(
                500,
                &json!({"code":"POISONED","message":"session lock poisoned"}),
            );
        }
    };
    let locked_ms = t_start.elapsed().as_secs_f64() * 1000.0;
    let result = begun.and_then(|o| match o {
        Outcome::Done(r) => Ok(r),
        Outcome::Later(job) => job.run(),
    });
    let total_ms = t_start.elapsed().as_secs_f64() * 1000.0;
    // `lock+begin` = tempo sob o lock da sessão; `total` inclui o trabalho fora dele (quadros)
    let timing = format!("lock+begin;dur={locked_ms:.2}, total;dur={total_ms:.2}");
    match result {
        Ok(Reply::Json(mut v)) => {
            if method == "events.poll" {
                app.ai.merge_events(&mut v);
            }
            Resp::json(200, &v).with("Server-Timing", &timing)
        }
        Ok(Reply::Binary { mime, bytes, meta }) => Resp::new(200, mime, bytes)
            .with("X-Capia-Meta", &meta.to_string())
            .with(
                "Access-Control-Expose-Headers",
                "X-Capia-Meta, Server-Timing",
            )
            .with("Server-Timing", &timing),
        Err(e) => Resp::json(422, &e.to_json()),
    }
}

fn route(app: &App, root: &Path, token: Option<&str>, req: &Req) -> Resp {
    if let Some(method) = req.url.strip_prefix("/api/") {
        return api(app, token, method, req);
    }
    if req.method != "GET" {
        return Resp::text(405, "method not allowed");
    }
    let file = static_path(root, &req.url)
        .filter(|p| p.is_file())
        // rota do SPA: qualquer caminho sem extensão cai no index.html
        .or_else(|| (!req.url.contains('.')).then(|| root.join("index.html")));
    match file.and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))) {
        Some((p, bytes)) => Resp::new(200, mime_of(&p), bytes),
        None => Resp::text(404, "not found"),
    }
}

fn serve(stream: TcpStream, app: &App, root: &Path, token: Option<&str>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut write_half = write_half;
    let mut reader = BufReader::new(stream);
    while let Some(req) = read_req(&mut reader) {
        let resp = route(app, root, token, &req);
        if write_response(&mut write_half, &resp, req.keep_alive).is_err() || !req.keep_alive {
            return;
        }
    }
}

fn main() {
    // pânico nunca imprime segredo (mensagem passa pelo redator do processo)
    capia_secrets::install_redacting_panic_hook();
    let mut port = 5199u16;
    let mut root = PathBuf::from("apps/desktop/dist");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--static" => root = args.next().map(PathBuf::from).unwrap_or(root),
            other => {
                eprintln!("unknown argument `{other}`");
                std::process::exit(2);
            }
        }
    }
    let token = std::env::var("CAPIA_DEVSERVER_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on 127.0.0.1:{port}: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "capia-devserver listening on http://127.0.0.1:{port} (static: {})",
        root.display()
    );
    let session = Arc::new(Mutex::new(Session::new(SessionConfig::default())));
    // devserver = dev/E2E: cofre em memória (nada vai ao Credential Manager) e registry sem AppDb
    let ai = IntelligenceService::new(
        Arc::new(SessionEngine::new(Arc::clone(&session))),
        capia_intelligence::service::ServiceConfig {
            appdb_path: std::env::var_os("CAPIA_AI_APPDB").map(PathBuf::from),
            secrets: Arc::new(capia_secrets::MemoryStore::new()),
        },
    )
    .expect("failed to start the intelligence service");
    if let Some(p) = std::env::var_os("CAPIA_AI_REPLAY_SCRIPT") {
        // roteiros Replay para E2E sem rede/credenciais (nunca no produto)
        let scripts: Value = std::fs::read_to_string(&p)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| {
                eprintln!("CAPIA_AI_REPLAY_SCRIPT is not readable JSON");
                std::process::exit(2);
            });
        ai.load_replay_scripts(&scripts).expect("replay scripts");
    }
    if std::env::var_os("CAPIA_AI_DEMO_BRAIN").is_some() {
        // cérebro determinístico para E2E de UI (nunca no produto)
        ai.install_demo_autonomy();
    }
    let app = Arc::new(App { session, ai });
    let root = Arc::new(root);
    let token = Arc::new(token);
    // uma thread por conexão (127.0.0.1, uso de dev/E2E): um quadro em render não bloqueia o resto
    for stream in listener.incoming().flatten() {
        let (app, root, token) = (Arc::clone(&app), Arc::clone(&root), Arc::clone(&token));
        std::thread::spawn(move || {
            serve(stream, &app, &root, token.as_deref());
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn static_paths_never_escape_the_root() {
        let root = Path::new("/srv/dist");
        assert!(static_path(root, "/../etc/passwd").is_none());
        assert!(static_path(root, "/a/../../b").is_none());
        assert_eq!(
            static_path(root, "/assets/app.js?x=1"),
            Some(PathBuf::from("/srv/dist/assets/app.js"))
        );
    }

    #[test]
    fn headers_cannot_be_split_by_injected_newlines() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let mut c = TcpStream::connect(addr).unwrap();
        let (mut s, _) = l.accept().unwrap();
        let r = Resp::text(200, "ok").with("X-Test", "a\r\nInjected: 1");
        write_response(&mut s, &r, false).unwrap();
        drop(s);
        let mut got = String::new();
        c.read_to_string(&mut got).unwrap();
        assert!(!got.contains("\r\nInjected:"), "{got}");
    }
}
