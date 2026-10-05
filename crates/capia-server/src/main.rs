//! `capia-server`: host local da Engine API (REST `/v1`, MCP, webhooks).
//!
//! ```text
//! capia-server serve      --data-dir D [--port N] [--bootstrap] [--stop-on-stdin-eof]
//! capia-server stop       --data-dir D                       # shutdown gracioso
//! capia-server token      create|list|revoke --data-dir D …  # offline: o segredo sai UMA vez
//! capia-server mcp-stdio  --data-dir D --token-env VAR
//! capia-server openapi | catalog                             # JSON no stdout
//! ```
//!
//! Loopback por padrão. Bind remoto exige `--allow-remote --remote-tls-terminated-by-proxy`.

use capia_server::auth;
use capia_server::config::ServerConfig;
use capia_server::{Server, openapi};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  capia-server serve --data-dir D [--port N] [--bind IP] [--allow-remote --remote-tls-terminated-by-proxy] \\\n      [--cors-origin URL]... [--allowed-host HOST:PORT]... [--bootstrap] [--stop-on-stdin-eof]\n  capia-server stop --data-dir D\n  capia-server token create --data-dir D --name NAME --scopes a,b[,..] [--expires-in SECONDS]\n  capia-server token list|revoke --data-dir D [--id ID]\n  capia-server mcp-stdio --data-dir D --token-env VAR\n  capia-server openapi | catalog"
    );
    ExitCode::from(2)
}

struct Args(Vec<String>);

impl Args {
    fn value(&self, flag: &str) -> Option<String> {
        self.0
            .iter()
            .position(|a| a == flag)
            .and_then(|i| self.0.get(i + 1).cloned())
    }

    fn values(&self, flag: &str) -> Vec<String> {
        self.0
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == flag)
            .filter_map(|(i, _)| self.0.get(i + 1).cloned())
            .collect()
    }

    fn has(&self, flag: &str) -> bool {
        self.0.iter().any(|a| a == flag)
    }
}

fn open_db(dir: &PathBuf) -> Result<capia_store::ServerDb, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    capia_store::ServerDb::open(&dir.join("server.db"), Duration::from_secs(5))
        .map_err(|e| e.message)
}

fn main() -> ExitCode {
    // pânico nunca imprime segredo (mensagem passa pelo redator do processo)
    capia_secrets::install_redacting_panic_hook();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = raw.first().cloned() else {
        return usage();
    };
    let args = Args(raw[1..].to_vec());
    match cmd.as_str() {
        "openapi" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&openapi::document()).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        "catalog" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&openapi::catalog_json()).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        "token" => token_cmd(&raw[1..], &args),
        "stop" => {
            let Some(dir) = args.value("--data-dir") else {
                return usage();
            };
            match std::fs::write(PathBuf::from(dir).join("shutdown"), b"1") {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("cannot request shutdown: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "serve" => serve(&args),
        "mcp-stdio" => mcp_stdio(&args),
        _ => usage(),
    }
}

fn token_cmd(raw: &[String], args: &Args) -> ExitCode {
    let (Some(sub), Some(dir)) = (raw.first(), args.value("--data-dir")) else {
        return usage();
    };
    let db = match open_db(&PathBuf::from(dir)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match sub.as_str() {
        "create" => {
            let (Some(name), Some(scopes)) = (args.value("--name"), args.value("--scopes")) else {
                return usage();
            };
            let scopes: Vec<String> = scopes.split(',').map(|s| s.trim().to_owned()).collect();
            let ttl = args.value("--expires-in").and_then(|s| s.parse().ok());
            match auth::create_token(&db, &name, &scopes, ttl) {
                Ok((row, secret)) => {
                    // o segredo é mostrado AQUI, uma única vez; o banco guarda só o hash
                    println!(
                        "{}",
                        serde_json::json!({"token": auth::public_view(&row), "secret": secret})
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        "list" => {
            let rows = db.token_list().unwrap_or_default();
            let v: Vec<_> = rows.iter().map(auth::public_view).collect();
            println!("{}", serde_json::json!({"tokens": v}));
            ExitCode::SUCCESS
        }
        "revoke" => {
            let Some(id) = args.value("--id") else {
                return usage();
            };
            match db.token_revoke(&id, auth::now_ms()) {
                Ok(b) => {
                    println!("{}", serde_json::json!({"revoked": b}));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e.message);
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}

fn serve(args: &Args) -> ExitCode {
    let Some(dir) = args.value("--data-dir") else {
        return usage();
    };
    let mut cfg = ServerConfig::new(&dir);
    if let Some(p) = args.value("--port") {
        match p.parse() {
            Ok(p) => cfg.port = p,
            Err(_) => return usage(),
        }
    }
    if let Some(b) = args.value("--bind") {
        match b.parse() {
            Ok(b) => cfg.bind = b,
            Err(_) => return usage(),
        }
    }
    cfg.allow_remote = args.has("--allow-remote");
    cfg.remote_tls_terminated_by_proxy = args.has("--remote-tls-terminated-by-proxy");
    cfg.cors_origins = args.values("--cors-origin");
    cfg.allowed_hosts = args.values("--allowed-host");
    #[cfg(feature = "testkit")]
    {
        // cérebro Replay SÓ em build de teste (a feature `testkit` nunca liga no produto)
        cfg.demo_brain = std::env::var_os("CAPIA_AI_DEMO_BRAIN").is_some();
    }
    if let Err(e) = cfg.validate() {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    if !cfg.is_loopback() {
        eprintln!(
            "WARNING: listening on {} (not loopback). Put a TLS reverse proxy in front.",
            cfg.bind
        );
    }
    if capia_secrets::platform_store().is_err() {
        eprintln!(
            "note: no OS credential vault on this platform; webhook secrets live in memory only (re-create or rotate them after a restart)"
        );
    }
    let data_dir = PathBuf::from(&dir);
    if args.has("--bootstrap") {
        match open_db(&data_dir) {
            Ok(db) if db.token_count_active(auth::now_ms()).unwrap_or(1) == 0 => {
                let all: Vec<String> = capia_server::scope::ALL_SCOPES
                    .iter()
                    .map(|s| s.as_str().to_owned())
                    .collect();
                if let Ok((_, secret)) = auth::create_token(&db, "bootstrap", &all, None) {
                    println!("CAPIA_BOOTSTRAP_TOKEN={secret}");
                }
            }
            _ => {}
        }
    }
    let server = match Server::start(cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "capia-server listening on http://{} (data: {dir})",
        server.addr()
    );
    let stdin_eof = args.has("--stop-on-stdin-eof");
    if stdin_eof {
        let flag = data_dir.join("shutdown");
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            while std::io::Read::read(&mut std::io::stdin(), &mut buf).is_ok_and(|n| n > 0) {}
            let _ = std::fs::write(flag, b"1");
        });
    }
    let marker = data_dir.join("shutdown");
    let _ = std::fs::remove_file(&marker);
    while !marker.exists() {
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::remove_file(&marker);
    server.shutdown();
    ExitCode::SUCCESS
}

/// MCP por stdio: o mesmo `Core` do `serve`, sem abrir socket (pump + webhooks rodam em threads).
fn mcp_stdio(args: &Args) -> ExitCode {
    let (Some(dir), Some(token_env)) = (args.value("--data-dir"), args.value("--token-env")) else {
        return usage();
    };
    #[allow(unused_mut)]
    let mut cfg = ServerConfig::new(&dir);
    #[cfg(feature = "testkit")]
    {
        cfg.demo_brain = std::env::var_os("CAPIA_AI_DEMO_BRAIN").is_some();
    }
    let host = match capia_server::mcp::Headless::start(cfg) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let code = capia_server::mcp::serve_stdio(std::sync::Arc::clone(host.core()), &token_env);
    host.shutdown();
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
