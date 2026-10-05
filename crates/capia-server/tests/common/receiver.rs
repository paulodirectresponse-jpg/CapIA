//! Receptor de webhook **de teste** (std::net): grava o que recebe, responde de forma programável
//! (status, atraso, redirect) e verifica assinaturas. Escuta só em 127.0.0.1, vive dentro do
//! processo de teste e NÃO é um serviço de eco público.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used, clippy::expect_used)]

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Resposta decidida pelo script para uma requisição.
#[derive(Clone, Debug)]
pub struct Action {
    pub status: u16,
    pub delay: Duration,
    pub location: Option<String>,
}

impl Action {
    pub fn status(status: u16) -> Self {
        Self {
            status,
            delay: Duration::ZERO,
            location: None,
        }
    }

    pub fn slow(status: u16, delay: Duration) -> Self {
        Self {
            status,
            delay,
            location: None,
        }
    }

    pub fn redirect(to: &str) -> Self {
        Self {
            status: 302,
            delay: Duration::ZERO,
            location: Some(to.to_owned()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    /// Ordem de chegada (0, 1, …).
    pub n: usize,
    pub received_at: Instant,
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn timestamp(&self) -> u64 {
        self.header("x-capia-timestamp").unwrap().parse().unwrap()
    }

    pub fn event_id(&self) -> &str {
        self.header("x-capia-event-id").unwrap()
    }

    /// Verificação INDEPENDENTE do servidor: HMAC-SHA256 reimplementado aqui (RFC 2104) sobre
    /// `"<timestamp>.<corpo cru>"`, comparado com o cabeçalho.
    pub fn signature_ok_independent(&self, secret: &str) -> bool {
        let Some(sig) = self.header("x-capia-signature") else {
            return false;
        };
        let mut msg = self.timestamp().to_string().into_bytes();
        msg.push(b'.');
        msg.extend_from_slice(&self.body);
        sig == format!("v1={}", hmac_sha256_hex(secret.as_bytes(), &msg))
    }

    /// Verificação pela API pública `capia_server::webhooks::verify` (a que os receptores usam).
    pub fn verify(
        &self,
        secret: &str,
        now: u64,
    ) -> Result<(), capia_server::webhooks::VerifyError> {
        capia_server::webhooks::verify(
            secret,
            self.timestamp(),
            &self.body,
            self.header("x-capia-signature").unwrap_or(""),
            now,
            capia_server::webhooks::REPLAY_TOLERANCE_SECS,
        )
    }
}

pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

type Script = Box<dyn Fn(&Request) -> Action + Send + Sync>;

struct Shared {
    log: Mutex<Vec<Request>>,
    script: Mutex<Script>,
    stop: AtomicBool,
}

pub struct Receiver {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
}

impl Receiver {
    /// Responde 200 até alguém chamar [`Receiver::script`].
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            log: Mutex::new(Vec::new()),
            script: Mutex::new(Box::new(|_| Action::status(200))),
            stop: AtomicBool::new(false),
        });
        let sh = Arc::clone(&shared);
        std::thread::spawn(move || {
            while !sh.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((s, _)) => {
                        let sh = Arc::clone(&sh);
                        // uma thread por conexão: um atraso programado nunca trava as outras
                        std::thread::spawn(move || serve(&sh, s));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self { addr, shared }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/hook", self.addr.port())
    }

    /// Define como responder (recebe a requisição já gravada, com `n` = índice de chegada).
    pub fn script(&self, f: impl Fn(&Request) -> Action + Send + Sync + 'static) {
        *self.shared.script.lock().unwrap() = Box::new(f);
    }

    pub fn requests(&self) -> Vec<Request> {
        self.shared.log.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.shared.log.lock().unwrap().len()
    }

    /// Espera até `n` requisições (ou o prazo) e devolve tudo o que chegou.
    pub fn wait_for(&self, n: usize, timeout: Duration) -> Vec<Request> {
        let t0 = Instant::now();
        while self.count() < n && t0.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.requests()
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(sh: &Shared, mut s: TcpStream) {
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (method, path) = (
        first.next().unwrap_or_default().to_owned(),
        first.next().unwrap_or_default().to_owned(),
    );
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    let req = {
        let mut log = sh.log.lock().unwrap();
        let r = Request {
            n: log.len(),
            received_at: Instant::now(),
            method,
            path,
            headers,
            body,
        };
        log.push(r.clone());
        r
    };
    let action = (sh.script.lock().unwrap())(&req);
    if !action.delay.is_zero() {
        std::thread::sleep(action.delay);
    }
    let mut resp = format!(
        "HTTP/1.1 {} X\r\nContent-Length: 0\r\nConnection: close\r\n",
        action.status
    );
    if let Some(l) = &action.location {
        resp.push_str(&format!("Location: {l}\r\n"));
    }
    resp.push_str("\r\n");
    let _ = s.write_all(resp.as_bytes());
    let _ = s.flush();
}
