//! Servidor local de **desenvolvimento e E2E** (nunca distribuído): `POST /api/<método>` com corpo
//! JSON chama `capia_editor_api::Session::call`; quadros/miniaturas voltam como bytes com os
//! metadados em `X-Capia-Meta`. Qualquer outro `GET` serve arquivos do front compilado.
//!
//! Segurança: escuta só em `127.0.0.1`; com `CAPIA_DEVSERVER_TOKEN` exige o cabeçalho
//! `x-capia-token`. O produto usa o shell Tauri (IPC tipado), não este servidor.
//!
//! Uso: `capia-devserver [--port 5199] [--static apps/desktop/dist]`.

use capia_editor_api::{Reply, Session, SessionConfig};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

fn header(name: &str, value: &str) -> Header {
    // nomes/valores são ASCII controlados por nós: falha aqui seria bug de programação
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .unwrap_or_else(|()| Header::from_bytes(&b"X-Error"[..], &b"bad-header"[..]).unwrap_or_else(|()| unreachable!()))
}

fn json_response(status: u16, v: &Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(v.to_string())
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("Cache-Control", "no-store"))
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

fn handle(session: &Mutex<Session>, root: &Path, token: Option<&str>, mut req: Request) {
    let url = req.url().to_owned();
    if let Some(method) = url.strip_prefix("/api/") {
        if let Some(t) = token {
            let ok = req
                .headers()
                .iter()
                .any(|h| h.field.equiv("x-capia-token") && h.value.as_str() == t);
            if !ok {
                let _ = req.respond(json_response(401, &json!({"code":"UNAUTHORIZED","message":"missing or wrong token"})));
                return;
            }
        }
        if *req.method() != Method::Post {
            let _ = req.respond(json_response(405, &json!({"code":"METHOD_NOT_ALLOWED","message":"use POST"})));
            return;
        }
        let mut body = String::new();
        if req.as_reader().take(64 << 20).read_to_string(&mut body).is_err() {
            let _ = req.respond(json_response(400, &json!({"code":"BAD_BODY","message":"unreadable body"})));
            return;
        }
        let params: Value = if body.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(e) => {
                    let _ = req.respond(json_response(400, &json!({"code":"BAD_JSON","message":e.to_string()})));
                    return;
                }
            }
        };
        let result = match session.lock() {
            Ok(mut s) => s.call(method, params),
            Err(_) => {
                let _ = req.respond(json_response(500, &json!({"code":"POISONED","message":"session lock poisoned"})));
                return;
            }
        };
        let _ = match result {
            Ok(Reply::Json(v)) => req.respond(json_response(200, &v)),
            Ok(Reply::Binary { mime, bytes, meta }) => req.respond(
                Response::from_data(bytes)
                    .with_header(header("Content-Type", mime))
                    .with_header(header("X-Capia-Meta", &meta.to_string()))
                    .with_header(header("Access-Control-Expose-Headers", "X-Capia-Meta"))
                    .with_header(header("Cache-Control", "no-store")),
            ),
            Err(e) => req.respond(json_response(422, &e.to_json())),
        };
        return;
    }
    if *req.method() != Method::Get {
        let _ = req.respond(Response::from_string("method not allowed").with_status_code(405));
        return;
    }
    let file = static_path(root, &url).filter(|p| p.is_file()).or_else(|| {
        // rota do SPA: qualquer caminho sem extensão cai no index.html
        (!url.contains('.')).then(|| root.join("index.html"))
    });
    match file.and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))) {
        Some((p, bytes)) => {
            let _ = req.respond(Response::from_data(bytes).with_header(header("Content-Type", mime_of(&p))));
        }
        None => {
            let _ = req.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}

fn main() {
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
    let token = std::env::var("CAPIA_DEVSERVER_TOKEN").ok().filter(|t| !t.is_empty());
    let server = match Server::http(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot listen on 127.0.0.1:{port}: {e}");
            std::process::exit(1);
        }
    };
    println!("capia-devserver listening on http://127.0.0.1:{port} (static: {})", root.display());
    let session = Mutex::new(Session::new(SessionConfig::default()));
    for req in server.incoming_requests() {
        handle(&session, &root, token.as_deref(), req);
    }
}
