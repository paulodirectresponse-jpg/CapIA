//! Utilitários dos testes do servidor: diretório temporário, servidor em loopback com token admin e
//! um cliente HTTP mínimo sobre `std::net` (sem dependência extra).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use capia_server::config::ServerConfig;
use capia_server::{Server, auth};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "capia-srv-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    }

    pub fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_owned()
    }
}

/// Envia bytes crus e lê até o servidor fechar (`Connection: close`).
pub fn raw(addr: SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let _ = s.write_all(bytes);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    out
}

pub fn parse(out: &[u8]) -> Resp {
    let split = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no head in {:?}", String::from_utf8_lossy(out)));
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Resp {
        status,
        headers,
        body: out[split + 4..].to_vec(),
    }
}

pub fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Resp {
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
        addr.port()
    );
    if let Some(t) = token {
        head.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        head.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    if let Some(b) = body {
        bytes.extend_from_slice(b);
    }
    parse(&raw(addr, &bytes))
}

pub struct TestServer {
    pub server: Option<Server>,
    pub dir: TempDir,
    pub addr: SocketAddr,
    pub admin: String,
}

pub fn start(tag: &str, tweak: impl FnOnce(&mut ServerConfig)) -> TestServer {
    let dir = TempDir::new(tag);
    start_in(dir, tweak)
}

pub fn start_in(dir: TempDir, tweak: impl FnOnce(&mut ServerConfig)) -> TestServer {
    let mut cfg = ServerConfig::new(dir.path());
    cfg.webhook_backoff_base = Duration::from_millis(50);
    cfg.webhook_backoff_cap = Duration::from_millis(400);
    cfg.secrets = std::sync::Arc::new(capia_secrets::MemoryStore::new());
    tweak(&mut cfg);
    let server = Server::start(cfg).unwrap_or_else(|e| panic!("server did not start: {e}"));
    let all: Vec<String> = capia_server::scope::ALL_SCOPES
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    let (_, admin) = auth::create_token(&server.core().db, "admin", &all, None).unwrap();
    TestServer {
        addr: server.addr(),
        server: Some(server),
        dir,
        admin,
    }
}

impl TestServer {
    pub fn call(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        self.call_as(&self.admin, method, path, body)
    }

    pub fn call_as(&self, token: &str, method: &str, path: &str, body: Option<Value>) -> Resp {
        let bytes = body.map(|b| b.to_string().into_bytes());
        request(
            self.addr,
            method,
            path,
            Some(token),
            &[("Content-Type", "application/json")],
            bytes.as_deref(),
        )
    }

    pub fn call_idem(&self, key: &str, method: &str, path: &str, body: Value) -> Resp {
        let bytes = body.to_string().into_bytes();
        request(
            self.addr,
            method,
            path,
            Some(&self.admin),
            &[
                ("Content-Type", "application/json"),
                ("Idempotency-Key", key),
            ],
            Some(&bytes),
        )
    }

    /// Cria um token com os scopes dados (via DB, como o CLI offline).
    pub fn token(&self, name: &str, scopes: &[&str]) -> String {
        let s: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
        auth::create_token(&self.core().db, name, &s, None)
            .unwrap()
            .1
    }

    pub fn core(&self) -> &std::sync::Arc<capia_server::Core> {
        self.server.as_ref().unwrap().core()
    }

    pub fn create_project(&self, name: &str) -> String {
        let r = self.call("POST", "/v1/projects", Some(json!({"name": name})));
        assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
        r.json()["project"]["id"].as_str().unwrap().to_owned()
    }

    pub fn shutdown(&mut self) {
        if let Some(s) = self.server.take() {
            s.shutdown();
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Transação mínima válida: cria uma sequence (1080x1920@30).
pub fn create_sequence_cmds(op: &str, seq: &str) -> Value {
    json!([{"operation_id": op, "type": "create_sequence", "id": seq, "name": "Main", "frame_rate": "30", "width": 1080, "height": 1920}])
}
