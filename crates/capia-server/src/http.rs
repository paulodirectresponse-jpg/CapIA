//! HTTP/1.1 mínimo e **endurecido** sobre `std::net` (sem dependência de framework): limites de
//! linha/cabeçalhos/corpo, prazo total para o cabeçalho (slowloris), `Content-Length` obrigatório
//! (sem `Transfer-Encoding`: nada de request smuggling), pool de workers limitado com fila finita
//! (backpressure 503), validação de `Host` (anti DNS-rebinding) e CORS fechado por padrão.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub const MAX_LINE: usize = 8 * 1024;
pub const MAX_HEAD: usize = 32 * 1024;
pub const MAX_HEADERS: usize = 64;
pub const HEAD_DEADLINE: Duration = Duration::from_secs(10);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15);
const READ_SLICE: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Nomes em minúsculas.
    pub headers: Vec<(String, String)>,
    pub content_length: u64,
    pub keep_alive: bool,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, v: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![(
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            )],
            body: v.to_string().into_bytes(),
        }
    }

    pub fn with(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_owned(), v.to_owned()));
        self
    }

    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// Erro de cabeçalho: resposta imediata e a conexão fecha.
#[derive(Debug)]
pub struct HeadError {
    pub status: u16,
    pub code: &'static str,
    pub message: &'static str,
}

fn he(status: u16, code: &'static str, message: &'static str) -> HeadError {
    HeadError {
        status,
        code,
        message,
    }
}

/// `TcpStream` com **prazo absoluto** por requisição: cada `read` usa `min(restante, 5 s)`.
#[derive(Debug)]
pub struct DeadlineStream {
    pub inner: TcpStream,
    pub deadline: Instant,
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let now = Instant::now();
        let Some(left) = self
            .deadline
            .checked_duration_since(now)
            .filter(|d| !d.is_zero())
        else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "deadline exceeded",
            ));
        };
        self.inner.set_read_timeout(Some(left.min(READ_SLICE)))?;
        self.inner.read(buf)
    }
}

pub type Reader = BufReader<DeadlineStream>;

pub fn new_reader(stream: TcpStream) -> Reader {
    BufReader::with_capacity(
        16 * 1024,
        DeadlineStream {
            inner: stream,
            deadline: Instant::now() + IDLE_TIMEOUT,
        },
    )
}

fn read_line(r: &mut Reader, budget: &mut usize) -> Result<Option<String>, HeadError> {
    let mut line = Vec::new();
    let n = r
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut line)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                he(408, "REQUEST_TIMEOUT", "the request head took too long")
            }
            _ => he(
                400,
                "BAD_REQUEST",
                "connection error while reading the request",
            ),
        })?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() > MAX_LINE {
        return Err(he(431, "HEADER_TOO_LARGE", "a header line is too long"));
    }
    *budget = budget
        .checked_sub(line.len())
        .ok_or_else(|| he(431, "HEADER_TOO_LARGE", "the request head is too large"))?;
    if !line.ends_with(b"\n") {
        return Err(he(400, "BAD_REQUEST", "truncated request head"));
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    if line.iter().any(|b| *b < 0x20 && *b != b'\t') || line.contains(&0x7f) {
        return Err(he(
            400,
            "BAD_REQUEST",
            "control characters in the request head",
        ));
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| he(400, "BAD_REQUEST", "the request head is not valid UTF-8"))
}

pub fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let h = b.get(i + 1..i + 3)?;
                let v = u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
                out.push(v);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Lê a linha de requisição + cabeçalhos. `Ok(None)` = o cliente fechou a conexão ociosa.
pub fn read_request(r: &mut Reader) -> Result<Option<Request>, HeadError> {
    r.get_mut().deadline = Instant::now() + IDLE_TIMEOUT;
    let mut budget = MAX_HEAD;
    // `BufRead::fill_buf` bloqueia até o primeiro byte (ocioso); depois vale o prazo do cabeçalho
    match r.fill_buf() {
        Ok([]) => return Ok(None),
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            return Ok(None);
        }
        Err(_) => return Ok(None),
    }
    r.get_mut().deadline = Instant::now() + HEAD_DEADLINE;
    let Some(first) = read_line(r, &mut budget)? else {
        return Ok(None);
    };
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(he(400, "BAD_REQUEST", "malformed request line"));
    };
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") {
        return Err(he(
            505,
            "HTTP_VERSION",
            "only HTTP/1.0 and HTTP/1.1 are supported",
        ));
    }
    if !method.bytes().all(|b| b.is_ascii_uppercase()) || method.len() > 8 {
        return Err(he(400, "BAD_REQUEST", "malformed method"));
    }
    if !target.starts_with('/') || target.len() > 4096 {
        return Err(he(
            400,
            "BAD_REQUEST",
            "the request target must be an absolute path",
        ));
    }
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(raw_path)
        .filter(|p| !p.contains('\0'))
        .ok_or_else(|| he(400, "BAD_REQUEST", "invalid percent-encoding in the path"))?;
    let mut query = Vec::new();
    for pair in raw_query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (Some(k), Some(v)) = (percent_decode(k), percent_decode(v)) else {
            return Err(he(
                400,
                "BAD_REQUEST",
                "invalid percent-encoding in the query",
            ));
        };
        query.push((k, v));
        if query.len() > 32 {
            return Err(he(400, "BAD_REQUEST", "too many query parameters"));
        }
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let Some(line) = read_line(r, &mut budget)? else {
            return Err(he(400, "BAD_REQUEST", "truncated request head"));
        };
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(he(431, "HEADER_TOO_LARGE", "too many headers"));
        }
        let Some((k, v)) = line.split_once(':') else {
            return Err(he(400, "BAD_REQUEST", "malformed header"));
        };
        if k.is_empty()
            || !k
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(he(400, "BAD_REQUEST", "malformed header name"));
        }
        headers.push((k.to_ascii_lowercase(), v.trim().to_owned()));
    }
    let count = |n: &str| headers.iter().filter(|(k, _)| k == n).count();
    if headers.iter().any(|(k, _)| k == "transfer-encoding") {
        return Err(he(
            501,
            "TRANSFER_ENCODING_UNSUPPORTED",
            "Transfer-Encoding is not supported: send Content-Length",
        ));
    }
    if count("content-length") > 1 || count("host") > 1 {
        return Err(he(
            400,
            "BAD_REQUEST",
            "duplicate Content-Length/Host header",
        ));
    }
    let content_length = match headers.iter().find(|(k, _)| k == "content-length") {
        None => 0,
        Some((_, v)) => v
            .parse::<u64>()
            .ok()
            .filter(|_| v.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| he(400, "BAD_REQUEST", "invalid Content-Length"))?,
    };
    let connection = headers
        .iter()
        .find(|(k, _)| k == "connection")
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_default();
    let keep_alive = if version == "HTTP/1.1" {
        connection != "close"
    } else {
        connection == "keep-alive"
    };
    Ok(Some(Request {
        method: method.to_owned(),
        path,
        query,
        headers,
        content_length,
        keep_alive,
    }))
}

/// Lê o corpo (já validado contra o teto) com prazo próprio.
pub fn read_body(r: &mut Reader, len: usize, deadline: Duration) -> Result<Vec<u8>, HeadError> {
    r.get_mut().deadline = Instant::now() + deadline;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            he(408, "REQUEST_TIMEOUT", "the request body took too long")
        }
        _ => he(400, "BAD_REQUEST", "the request body was truncated"),
    })?;
    Ok(body)
}

/// Descarta `len` bytes de corpo não lido (mantém a conexão utilizável) com teto de tempo.
pub fn drain(r: &mut Reader, len: u64) -> bool {
    if len > 256 * 1024 {
        return false;
    }
    r.get_mut().deadline = Instant::now() + Duration::from_secs(2);
    std::io::copy(&mut r.by_ref().take(len), &mut std::io::sink()).is_ok_and(|n| n == len)
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        507 => "Insufficient Storage",
        _ => "Status",
    }
}

fn head(
    resp_status: u16,
    headers: &[(String, String)],
    len: Option<usize>,
    keep_alive: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(format!("HTTP/1.1 {resp_status} {}\r\n", reason(resp_status)).as_bytes());
    for (k, v) in headers {
        // nenhum valor de cabeçalho carrega CR/LF (sem response splitting)
        let v = v.replace(['\r', '\n'], " ");
        out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
    }
    out.extend_from_slice(b"Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n");
    if let Some(n) = len {
        out.extend_from_slice(format!("Content-Length: {n}\r\n").as_bytes());
    }
    out.extend_from_slice(if keep_alive {
        b"Connection: keep-alive\r\n\r\n"
    } else {
        b"Connection: close\r\n\r\n"
    });
    out
}

/// Cabeçalho + corpo numa única escrita (um segmento TCP quando cabe).
pub fn write_response(
    stream: &mut TcpStream,
    resp: &Response,
    keep_alive: bool,
) -> std::io::Result<()> {
    let mut out = head(
        resp.status,
        &resp.headers,
        Some(resp.body.len()),
        keep_alive,
    );
    out.extend_from_slice(&resp.body);
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    stream.write_all(&out)?;
    stream.flush()
}

/// Início de um fluxo SSE (sem `Content-Length`; a conexão fecha ao terminar).
pub fn write_stream_head(
    stream: &mut TcpStream,
    headers: &[(String, String)],
) -> std::io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    // sem Content-Length e fechando a conexão: o fim do corpo é o fim da conexão
    let h = head(200, headers, None, false);
    stream.write_all(&h)?;
    stream.flush()
}

/// Profundidade máxima de aninhamento JSON, medida **antes** do parse (rápido e sem recursão).
pub fn json_depth_exceeds(body: &[u8], max: usize) -> bool {
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for &b in body {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > max {
                    return true;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_rejects_bad_sequences() {
        assert_eq!(percent_decode("a%20b+c").as_deref(), Some("a b c"));
        assert_eq!(percent_decode("%2e%2e").as_deref(), Some(".."));
        assert!(percent_decode("%zz").is_none());
        assert!(percent_decode("%4").is_none());
        assert!(percent_decode("%ff").is_none(), "invalid UTF-8");
    }

    #[test]
    fn json_depth_is_counted_outside_strings() {
        assert!(!json_depth_exceeds(br#"{"a":[1,{"b":"[[[[["}]}"#, 4));
        assert!(json_depth_exceeds(&b"[".repeat(40), 32));
        assert!(!json_depth_exceeds(&b"[".repeat(32), 32));
        assert!(!json_depth_exceeds(br#"{"k":"\"[[[[[[[[["}"#, 3));
    }
}
