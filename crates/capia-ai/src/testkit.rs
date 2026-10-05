//! Servidor HTTP falso (loopback) para testes de contrato e de segurança dos providers.
//! Registra todo pedido (método, caminho, headers, corpo) e responde por um *handler*.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct MockRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl MockRequest {
    pub fn header(&self, k: &str) -> Option<&str> {
        self.headers
            .get(&k.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    /// Todo o pedido como texto (para busca do canário).
    pub fn raw_text(&self) -> String {
        let mut s = format!("{} {}\n", self.method, self.path);
        for (k, v) in &self.headers {
            s.push_str(&format!("{k}: {v}\n"));
        }
        s.push_str(&String::from_utf8_lossy(&self.body));
        s
    }
}

#[derive(Debug)]
pub enum MockResponse {
    Json(u16, String),
    /// Eventos SSE `(event, data)`, com atraso entre eles; `hang` ⇒ não fecha a conexão ao final.
    Sse {
        events: Vec<(Option<String>, String)>,
        delay_ms: u64,
        hang: bool,
    },
    Redirect(String),
    /// Corpo bruto sem terminar (`hang`) ou com `n` bytes de `x`.
    Huge(usize),
    /// Aceita e nunca responde.
    Hang,
    RetryAfter(u16, u64),
}

type Handler = dyn Fn(&MockRequest) -> MockResponse + Send + Sync;

pub struct MockServer {
    pub addr: std::net::SocketAddr,
    pub requests: Arc<Mutex<Vec<MockRequest>>>,
    /// Conexões em que o cliente fechou/abortou enquanto o servidor ainda segurava a resposta.
    pub client_aborts: Arc<std::sync::atomic::AtomicUsize>,
    _task: tokio::task::JoinHandle<()>,
}

impl core::fmt::Debug for MockServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MockServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl MockServer {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// Mesmo servidor, mas pelo nome `localhost` (host diferente de `127.0.0.1` para testes de redirect).
    pub fn url_localhost(&self) -> String {
        format!("http://localhost:{}", self.addr.port())
    }

    pub fn seen(&self) -> Vec<MockRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let aborts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler: Arc<Handler> = Arc::new(handler);
        let reqs = requests.clone();
        let aborts_t = aborts.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                let reqs = reqs.clone();
                let aborts = aborts_t.clone();
                tokio::spawn(async move {
                    let Some(req) = read_request(&mut sock).await else {
                        return;
                    };
                    if let Ok(mut v) = reqs.lock() {
                        v.push(req.clone());
                    }
                    match handler(&req) {
                        MockResponse::Json(status, body) => {
                            let head = format!(
                                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = sock.write_all(head.as_bytes()).await;
                            let _ = sock.write_all(body.as_bytes()).await;
                        }
                        MockResponse::RetryAfter(status, secs) => {
                            let body = "{\"error\":\"slow\"}";
                            let head = format!(
                                "HTTP/1.1 {status} X\r\nRetry-After: {secs}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = sock.write_all(head.as_bytes()).await;
                            let _ = sock.write_all(body.as_bytes()).await;
                        }
                        MockResponse::Redirect(loc) => {
                            let head = format!(
                                "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            );
                            let _ = sock.write_all(head.as_bytes()).await;
                        }
                        MockResponse::Sse {
                            events,
                            delay_ms,
                            hang,
                        } => {
                            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
                            let _ = sock.write_all(head.as_bytes()).await;
                            for (ev, data) in events {
                                let mut s = String::new();
                                if let Some(e) = ev {
                                    s.push_str(&format!("event: {e}\n"));
                                }
                                for line in data.split('\n') {
                                    s.push_str(&format!("data: {line}\n"));
                                }
                                s.push('\n');
                                if sock.write_all(s.as_bytes()).await.is_err() {
                                    return;
                                }
                                let _ = sock.flush().await;
                                if delay_ms > 0 {
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                }
                            }
                            if hang {
                                // espera o cliente fechar a conexão (EOF) — prova de *abort* real
                                let mut b = [0u8; 16];
                                if let Ok(Ok(0) | Err(_)) =
                                    tokio::time::timeout(Duration::from_secs(30), sock.read(&mut b))
                                        .await
                                {
                                    aborts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                        }
                        MockResponse::Huge(n) => {
                            let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
                            let _ = sock.write_all(head.as_bytes()).await;
                            let chunk = vec![b'x'; 64 * 1024];
                            let mut sent = 0;
                            while sent < n {
                                if sock.write_all(&chunk).await.is_err() {
                                    return;
                                }
                                sent += chunk.len();
                            }
                        }
                        MockResponse::Hang => {
                            let mut b = [0u8; 16];
                            if let Ok(Ok(0) | Err(_)) =
                                tokio::time::timeout(Duration::from_secs(30), sock.read(&mut b))
                                    .await
                            {
                                aborts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            }
                        }
                    }
                });
            }
        });
        Self {
            addr,
            requests,
            client_aborts: aborts,
            _task: task,
        }
    }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<MockRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 1 << 20 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split(' ');
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = BTreeMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len.max(body.len().min(len)));
    Some(MockRequest {
        method,
        path,
        headers,
        body,
    })
}

/// Resposta de **texto** em streaming no protocolo nativo de cada família de provider (para testes
/// de integração que trocam o Brain entre fabricantes sem mudar o código que consome o provider).
pub fn text_reply(kind: crate::registry::ProviderKind, text: &str) -> MockResponse {
    use crate::registry::ProviderKind as K;
    use serde_json::json;
    let sse = |events: Vec<(Option<&str>, serde_json::Value)>, done: bool| {
        let mut ev: Vec<(Option<String>, String)> = events
            .into_iter()
            .map(|(e, v)| (e.map(str::to_owned), v.to_string()))
            .collect();
        if done {
            ev.push((None, "[DONE]".into()));
        }
        MockResponse::Sse {
            events: ev,
            delay_ms: 0,
            hang: false,
        }
    };
    match kind {
        K::Anthropic => sse(
            vec![
                (
                    Some("message_start"),
                    json!({"type": "message_start", "message": {"usage": {"input_tokens": 11, "output_tokens": 1}}}),
                ),
                (
                    Some("content_block_start"),
                    json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                ),
                (
                    Some("content_block_delta"),
                    json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
                ),
                (
                    Some("content_block_stop"),
                    json!({"type": "content_block_stop", "index": 0}),
                ),
                (
                    Some("message_delta"),
                    json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 2}}),
                ),
                (Some("message_stop"), json!({"type": "message_stop"})),
            ],
            false,
        ),
        K::Google => sse(
            vec![(
                None,
                json!({"candidates": [{"content": {"role": "model", "parts": [{"text": text}]}, "finishReason": "STOP"}],
                       "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 2}}),
            )],
            false,
        ),
        _ => sse(
            vec![
                (None, json!({"choices": [{"delta": {"content": text}}]})),
                (
                    None,
                    json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
                ),
                (
                    None,
                    json!({"choices": [], "usage": {"prompt_tokens": 11, "completion_tokens": 2}}),
                ),
            ],
            true,
        ),
    }
}
