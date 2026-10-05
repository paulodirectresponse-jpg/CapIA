//! Utilitários de ataque (Track D-2): cliente HTTP *cru* (cabeçalhos exatamente como dados),
//! PRNG determinístico, varredura de árvores de arquivos, contagem de threads/fds e o detector de
//! vazamento em corpo de resposta. Sem dependências novas. Incluído por `#[path]` nos testes que
//! o usam (`mod common; #[path = "common/attack.rs"] mod attack;`).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use crate::common::{Resp, TestServer};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// xorshift64*: determinístico, sem dependência. Mesma semente ⇒ mesma sequência.
#[derive(Clone, Debug)]
pub struct Prng(pub u64);

impl Prng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next_u64() >> 24) as u8).collect()
    }
}

/// Envia `bytes`, lê até o servidor fechar ou `limit` estourar. `None` = o servidor nunca
/// respondeu nem fechou dentro do prazo (hang).
pub fn exchange(addr: SocketAddr, bytes: &[u8], limit: Duration) -> Option<Vec<u8>> {
    let mut s = TcpStream::connect(addr).ok()?;
    s.set_read_timeout(Some(limit)).ok()?;
    s.set_write_timeout(Some(limit)).ok()?;
    let _ = s.write_all(bytes);
    let _ = s.flush();
    let mut out = Vec::new();
    let t0 = Instant::now();
    let mut buf = [0u8; 8192];
    loop {
        match s.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > 4 << 20 {
                    return Some(out);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // sem fechar e sem dados novos dentro do prazo: se já veio uma resposta
                // completa com keep-alive isso é normal; sem nada, é hang
                return if out.is_empty() { None } else { Some(out) };
            }
            Err(_) => return Some(out),
        }
        if t0.elapsed() > limit * 2 {
            return Some(out);
        }
    }
}

pub fn try_parse(out: &[u8]) -> Option<Resp> {
    let split = out.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    let mut lines = head.lines();
    let status: u16 = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Some(Resp {
        status,
        headers,
        body: out[split + 4..].to_vec(),
    })
}

/// Monta uma requisição com os cabeçalhos **exatamente** como dados (nada de Host automático).
pub fn build(method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut h = format!("{method} {target} HTTP/1.1\r\n");
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("\r\n");
    let mut b = h.into_bytes();
    b.extend_from_slice(body);
    b
}

/// Requisição crua com Host correto + `Connection: close` + os extras; devolve a resposta
/// parseada (ou `None` se a conexão fechou sem resposta).
pub fn send(
    s: &TestServer,
    method: &str,
    target: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: &[u8],
) -> Option<Resp> {
    let host = format!("127.0.0.1:{}", s.addr.port());
    let auth = token.map(|t| format!("Bearer {t}"));
    let len = body.len().to_string();
    let mut hs: Vec<(&str, &str)> = vec![("Host", &host), ("Connection", "close")];
    if let Some(a) = &auth {
        hs.push(("Authorization", a));
    }
    if !body.is_empty() {
        hs.push(("Content-Length", &len));
    }
    hs.extend_from_slice(extra);
    try_parse(&exchange(
        s.addr,
        &build(method, target, &hs, body),
        Duration::from_secs(15),
    )?)
}

/// `GET /v1/health` + uma chamada autenticada válida: o servidor continua saudável.
pub fn healthy(s: &TestServer) -> bool {
    let h = send(s, "GET", "/v1/health", None, &[], b"");
    let a = send(s, "GET", "/v1/server", Some(&s.admin), &[], b"");
    matches!((h, a), (Some(h), Some(a)) if h.status == 200 && a.status == 200)
}

pub fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        out.push(p.clone());
        if is_dir {
            out.extend(walk(&p));
        }
    }
    out
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Arquivos sob `dir` (inclui WAL/SHM do SQLite) cujo conteúdo contém `needle`.
pub fn files_containing(dir: &Path, needle: &[u8]) -> Vec<PathBuf> {
    walk(dir)
        .into_iter()
        .filter(|p| p.is_file())
        .filter(|p| std::fs::read(p).is_ok_and(|b| contains(&b, needle)))
        .collect()
}

/// Mesma coisa para os **nomes** de arquivo/diretório sob `dir`.
pub fn names_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    walk(dir)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains(needle))
        .collect()
}

pub fn threads() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(0, Iterator::count)
}

pub fn fds() -> usize {
    std::fs::read_dir("/proc/self/fd").map_or(0, Iterator::count)
}

/// Fragmentos que NUNCA podem aparecer num corpo de erro (caminhos, SQL, pilha, segredos).
const LEAK_MARKERS: &[&str] = &[
    "/tmp/",
    "/home/",
    "/var/",
    "/proc/",
    "/etc/",
    "C:\\",
    "capia-srv-",
    "rusqlite",
    "sqlite",
    "SQLITE",
    "SELECT ",
    "INSERT ",
    "UPDATE ",
    "panicked",
    "stack backtrace",
    "RUST_BACKTRACE",
    "thread '",
    "src/",
    ".rs:",
    "secret_hash",
    "capia_",
];

/// `Some(motivo)` se o texto vaza caminho, SQL, pilha ou o diretório de dados.
pub fn leak_in(text: &str, data_dir: &Path) -> Option<String> {
    let dd = data_dir.to_string_lossy();
    if text.contains(dd.as_ref()) {
        return Some(format!("data dir `{dd}`"));
    }
    LEAK_MARKERS
        .iter()
        .find(|m| text.contains(*m))
        .map(|m| format!("marker `{m}`"))
}

/// Corpo + cabeçalhos de uma resposta como texto único (para varredura).
pub fn whole(r: &Resp) -> String {
    let mut t = String::new();
    for (k, v) in &r.headers {
        t.push_str(k);
        t.push_str(": ");
        t.push_str(v);
        t.push('\n');
    }
    t.push_str(&String::from_utf8_lossy(&r.body));
    t
}

/// Um PNG mínimo válido (assinatura + IHDR + IDAT + IEND) — passa no *sniff* de imagem.
pub fn tiny_png() -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend_from_slice(&[0, 0, 0, 13]);
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    v.extend_from_slice(&[0x90, 0x77, 0x53, 0xDE]);
    v.extend_from_slice(&[0, 0, 0, 0]);
    v.extend_from_slice(b"IEND");
    v.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
    v
}

/// Envia um upload em streaming pelo endpoint real.
pub fn upload(
    s: &TestServer,
    token: &str,
    filename_header: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) -> Option<Resp> {
    let mut hs: Vec<(&str, &str)> = vec![("X-Capia-Filename", filename_header)];
    hs.extend_from_slice(extra);
    send(s, "POST", "/v1/uploads", Some(token), &hs, body)
}

/// Como `exchange`, mas fecha a metade de escrita depois de enviar (cliente que "morreu" no meio).
pub fn exchange_half_close(addr: SocketAddr, bytes: &[u8], limit: Duration) -> Option<Vec<u8>> {
    let mut s = TcpStream::connect(addr).ok()?;
    s.set_read_timeout(Some(limit)).ok()?;
    let _ = s.write_all(bytes);
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    Some(out)
}

/// O processo de teste roda como root? (chmod não restringe root.)
pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|u| u == "0"))
        })
        .unwrap_or(false)
}

/// Espera `cond` ficar verdadeira (até `max`); devolve o valor final.
pub fn wait_until(max: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < max {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    cond()
}

/// Nome de dispositivo reservado do Windows (com ou sem extensão) ou com ponto/espaço final.
pub fn windows_unsafe_name(n: &str) -> bool {
    let stem = n.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let dev = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    dev || n.ends_with('.') || n.ends_with(' ')
}

pub fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}
