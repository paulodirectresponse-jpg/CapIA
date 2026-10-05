//! Cliente HTTP endurecido para providers (SECURITY §3, PHASE4_PROVIDERS_SECURITY §13/§14/§22/§23):
//! esquema/host validados (SSRF), resolução de DNS filtrada, redirects só para o **mesmo** host
//! (a credencial nunca segue para outro), TLS normal, timeouts separados, tetos de corpo/evento,
//! concorrência/rate-limit por provider e *abort* real ao cancelar.

use crate::cancel::CancelToken;
use crate::error::{ErrorCode, ProviderError};
use crate::registry::{ProviderConfig, ProviderKind};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use reqwest::{RequestBuilder, Response};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use url::{Host, Url};

pub const MAX_REDIRECTS: usize = 3;

/// Política de destino de uma URL de provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UrlPolicy {
    pub allow_loopback: bool,
}

impl UrlPolicy {
    pub fn for_kind(kind: ProviderKind) -> Self {
        Self {
            // loopback só quando o provider é configurado como local
            allow_loopback: kind.is_local(),
        }
    }

    /// Como `for_kind`, mais o opt-in explícito `allow_loopback` do provider.
    pub fn for_config(cfg: &ProviderConfig) -> Self {
        Self {
            allow_loopback: cfg.kind.is_local() || cfg.allow_loopback,
        }
    }

    /// Valida esquema/host/credenciais embutidas e devolve a URL parseada.
    pub fn check(&self, raw: &str) -> Result<Url, ProviderError> {
        let bad = |m: &str| {
            ProviderError::new(
                ErrorCode::InvalidRequest,
                format!("invalid provider URL: {m}"),
            )
        };
        if raw.len() > 2048 {
            return Err(bad("too long"));
        }
        let url = Url::parse(raw).map_err(|_| bad("not a valid URL"))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(bad("credentials in URL are not allowed"));
        }
        let host = url.host().ok_or_else(|| bad("missing host"))?;
        let loopback = is_loopback_host(&host);
        match url.scheme() {
            "https" => {}
            "http" if loopback && self.allow_loopback => {}
            "http" => {
                return Err(bad(
                    "plain http is only allowed for a local provider on loopback",
                ));
            }
            _ => {
                return Err(bad(
                    "scheme must be https (or http on loopback for local providers)",
                ));
            }
        }
        if loopback && !self.allow_loopback {
            return Err(bad("loopback hosts are only allowed for local providers"));
        }
        match host {
            Host::Ipv4(ip) => self.check_ip(IpAddr::V4(ip)).map_err(|m| bad(&m))?,
            Host::Ipv6(ip) => self.check_ip(IpAddr::V6(ip)).map_err(|m| bad(&m))?,
            Host::Domain(d) => {
                let d = d.to_ascii_lowercase();
                let internal = d.ends_with(".local")
                    || d.ends_with(".internal")
                    || d.ends_with(".localdomain");
                if internal {
                    return Err(bad("internal host names are not allowed"));
                }
            }
        }
        Ok(url)
    }

    /// Endereço permitido? (nunca link-local/metadata/não especificado/multicast; privado só nunca.)
    pub fn check_ip(&self, ip: IpAddr) -> Result<(), String> {
        if ip.is_loopback() {
            return if self.allow_loopback {
                Ok(())
            } else {
                Err("loopback address not allowed".into())
            };
        }
        if is_blocked_ip(ip) {
            return Err(
                "address is in a blocked range (private, link-local, metadata or reserved)".into(),
            );
        }
        Ok(())
    }
}

fn is_loopback_host(h: &Host<&str>) -> bool {
    match h {
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
        Host::Domain(d) => {
            let d = d.to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
    }
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(m) = v6.to_ipv4_mapped() {
                return blocked_v4(m);
            }
            blocked_v6(v6)
        }
    }
}

fn blocked_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_private()
        || ip.is_link_local() // 169.254/16 inclui o endpoint de metadata de nuvem
        || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT 100.64/10
        || o[0] == 0
        || o[0] >= 240
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // benchmarking
}

fn blocked_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    ip.is_unspecified()
        || ip.is_multicast()
        || (s[0] & 0xffc0) == 0xfe80 // link-local
        || (s[0] & 0xfe00) == 0xfc00 // ULA
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentação
}

/// Resolver que **filtra** o que o DNS devolve (anti DNS-rebinding para hosts públicos).
#[derive(Clone, Copy, Debug)]
pub(crate) struct GuardedResolver {
    pub(crate) policy: UrlPolicy,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.policy;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let allowed: Vec<SocketAddr> =
                addrs.filter(|a| policy.check_ip(a.ip()).is_ok()).collect();
            if allowed.is_empty() {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "host resolves only to blocked addresses",
                ));
            }
            let it: Addrs = Box::new(allowed.into_iter());
            Ok(it)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HttpLimits {
    pub connect: Duration,
    pub first_byte: Duration,
    pub total: Duration,
    pub idle: Duration,
    pub max_body: usize,
    pub max_event: usize,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            first_byte: Duration::from_secs(60),
            total: Duration::from_secs(300),
            idle: Duration::from_secs(45),
            max_body: 32 * 1024 * 1024,
            max_event: 1024 * 1024,
        }
    }
}

struct RateLimiter {
    interval: Duration,
    next: Mutex<Instant>,
}

impl RateLimiter {
    async fn acquire(&self, cancel: &CancelToken) -> Result<(), ProviderError> {
        let wait = {
            let mut next = self.next.lock().await;
            let now = Instant::now();
            let at = (*next).max(now);
            *next = at + self.interval;
            at.saturating_duration_since(now)
        };
        if wait.is_zero() {
            return Ok(());
        }
        tokio::select! {
            () = cancel.cancelled() => Err(ProviderError::cancelled()),
            () = tokio::time::sleep(wait) => Ok(()),
        }
    }
}

/// Cliente de um provider configurado.
pub struct HttpClient {
    client: reqwest::Client,
    base: Url,
    pub limits: HttpLimits,
    sem: Arc<Semaphore>,
    rate: Option<RateLimiter>,
    label: String,
}

impl core::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpClient")
            .field("provider", &self.label)
            .field("host", &self.base.host_str())
            .finish_non_exhaustive()
    }
}

impl HttpClient {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, ProviderError> {
        let raw = cfg.effective_base_url().ok_or_else(|| {
            ProviderError::new(ErrorCode::NotConfigured, "provider has no base URL")
        })?;
        let policy = UrlPolicy::for_config(cfg);
        let base = policy.check(&raw)?;
        // vínculo credencial↔host: mudar a base_url para outro host exige reinserir a chave
        if cfg.credential_ref.is_some() {
            let host = base.host_str().unwrap_or_default();
            match &cfg.bound_host {
                Some(b) if b.eq_ignore_ascii_case(host) => {}
                _ => {
                    return Err(ProviderError::new(
                        ErrorCode::NotAllowed,
                        "the credential is bound to a different host: re-enter the key after changing the base URL",
                    ));
                }
            }
        }
        let origin_host = base.host_str().unwrap_or_default().to_ascii_lowercase();
        let redirect = Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            let same_host = attempt
                .url()
                .host_str()
                .is_some_and(|h| h.eq_ignore_ascii_case(&origin_host));
            let scheme_ok = attempt.url().scheme() == "https"
                || (attempt.url().scheme() == "http" && policy.allow_loopback);
            if same_host && scheme_ok {
                attempt.follow()
            } else {
                // não segue: a credencial nunca vai para outro host
                attempt.stop()
            }
        });
        let d = HttpLimits::default();
        let secs = |v: Option<u32>, def: Duration| {
            v.map_or(def, |s| Duration::from_secs(u64::from(s.clamp(1, 3600))))
        };
        let total = Duration::from_secs(u64::from(cfg.timeout_s));
        let limits = HttpLimits {
            total,
            connect: secs(cfg.connect_timeout_s, d.connect).min(total),
            first_byte: secs(cfg.first_byte_timeout_s, d.first_byte).min(total),
            idle: secs(cfg.idle_timeout_s, d.idle).min(total),
            ..d
        };
        let client = reqwest::Client::builder()
            .connect_timeout(limits.connect)
            .redirect(redirect)
            .dns_resolver(Arc::new(GuardedResolver { policy }))
            .user_agent("CapIA/0 (+local-first video editor)")
            .https_only(!policy.allow_loopback)
            .build()
            .map_err(|e| {
                ProviderError::new(ErrorCode::ProviderUnavailable, format!("http client: {e}"))
            })?;
        let rate = cfg
            .rate_limit
            .as_ref()
            .filter(|r| r.requests_per_minute > 0)
            .map(|r| RateLimiter {
                interval: Duration::from_millis(60_000 / u64::from(r.requests_per_minute)),
                next: Mutex::new(Instant::now()),
            });
        Ok(Self {
            client,
            base,
            limits,
            sem: Arc::new(Semaphore::new(cfg.max_concurrency.max(1) as usize)),
            rate,
            label: cfg.id.clone(),
        })
    }

    /// `base` + `path` (path relativo; sem permitir sair do host).
    pub fn url(&self, path: &str) -> Result<Url, ProviderError> {
        let mut s = self.base.as_str().trim_end_matches('/').to_owned();
        s.push('/');
        s.push_str(path.trim_start_matches('/'));
        let u = Url::parse(&s)
            .map_err(|_| ProviderError::new(ErrorCode::InvalidRequest, "bad path"))?;
        if u.host_str() != self.base.host_str() || u.scheme() != self.base.scheme() {
            return Err(ProviderError::new(
                ErrorCode::NotAllowed,
                "path escapes the provider host",
            ));
        }
        Ok(u)
    }

    pub fn post(&self, path: &str) -> Result<RequestBuilder, ProviderError> {
        Ok(self.client.post(self.url(path)?))
    }

    pub fn get(&self, path: &str) -> Result<RequestBuilder, ProviderError> {
        Ok(self.client.get(self.url(path)?))
    }

    /// Envia respeitando concorrência, rate limit, *first byte timeout* e cancelamento.
    /// Devolve a resposta **bem-sucedida**; status de erro viram [`ProviderError`] classificados.
    pub async fn send(
        &self,
        req: RequestBuilder,
        cancel: &CancelToken,
    ) -> Result<SentResponse, ProviderError> {
        if cancel.is_cancelled() {
            return Err(ProviderError::cancelled());
        }
        let permit = tokio::select! {
            () = cancel.cancelled() => return Err(ProviderError::cancelled()),
            p = self.sem.clone().acquire_owned() => p.map_err(|_| ProviderError::new(ErrorCode::ProviderUnavailable, "client closed"))?,
        };
        if let Some(r) = &self.rate {
            r.acquire(cancel).await?;
        }
        let started = Instant::now();
        let resp = tokio::select! {
            () = cancel.cancelled() => return Err(ProviderError::cancelled()),
            r = tokio::time::timeout(self.limits.first_byte, req.send()) => match r {
                Err(_) => return Err(ProviderError::new(ErrorCode::ProviderTimeout, "no response before the first-byte timeout").with_provider(&self.label)),
                Ok(Err(e)) => return Err(map_reqwest(&e).with_provider(&self.label)),
                Ok(Ok(resp)) => resp,
            },
        };
        let status = resp.status();
        if status.is_redirection() {
            return Err(ProviderError::new(
                ErrorCode::ProviderUnavailable,
                "redirect to a different host was blocked (credentials are never forwarded)",
            )
            .with_status(status.as_u16())
            .with_provider(&self.label));
        }
        if !status.is_success() {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|s| s.saturating_mul(1000));
            let body = read_limited(resp, 64 * 1024, &self.limits, started, cancel)
                .await
                .unwrap_or_default();
            let text = String::from_utf8_lossy(&body).into_owned();
            let mut e = ProviderError::from_http(status.as_u16(), &text).with_provider(&self.label);
            if let Some(ms) = retry_after {
                e = e.with_retry_after_ms(ms.min(120_000));
            }
            return Err(e);
        }
        Ok(SentResponse {
            resp,
            started,
            limits: self.limits,
            _permit: permit,
        })
    }
}

/// Resposta com a vaga de concorrência presa até ser consumida/descartada.
#[derive(Debug)]
pub struct SentResponse {
    resp: Response,
    started: Instant,
    limits: HttpLimits,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl SentResponse {
    pub async fn bytes(self, cancel: &CancelToken) -> Result<Vec<u8>, ProviderError> {
        read_limited(
            self.resp,
            self.limits.max_body,
            &self.limits,
            self.started,
            cancel,
        )
        .await
    }

    pub async fn json<T: serde::de::DeserializeOwned>(
        self,
        cancel: &CancelToken,
    ) -> Result<T, ProviderError> {
        let b = self.bytes(cancel).await?;
        serde_json::from_slice(&b).map_err(|e| {
            ProviderError::new(
                ErrorCode::InvalidProviderResponse,
                format!("invalid JSON: {e}"),
            )
        })
    }

    /// A resposta é `text/event-stream`?
    pub fn is_event_stream(&self) -> bool {
        self.resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"))
    }

    pub fn sse(self, cancel: CancelToken) -> SseReader {
        SseReader {
            resp: self.resp,
            buf: Vec::new(),
            started: self.started,
            limits: self.limits,
            cancel,
            done: false,
            _permit: self._permit,
        }
    }
}

fn map_reqwest(e: &reqwest::Error) -> ProviderError {
    let msg = capia_secrets::redact(&e.to_string());
    if e.is_timeout() {
        ProviderError::new(ErrorCode::ProviderTimeout, msg)
    } else if e.is_connect() {
        ProviderError::new(
            ErrorCode::ProviderUnavailable,
            format!("connection failed: {msg}"),
        )
    } else if e.is_redirect() {
        ProviderError::new(
            ErrorCode::ProviderUnavailable,
            format!("redirect refused: {msg}"),
        )
    } else {
        ProviderError::new(ErrorCode::ProviderUnavailable, msg)
    }
}

async fn next_chunk(
    resp: &mut Response,
    limits: &HttpLimits,
    started: Instant,
    cancel: &CancelToken,
) -> Result<Option<bytes::Bytes>, ProviderError> {
    let remaining = limits.total.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(ProviderError::new(
            ErrorCode::ProviderTimeout,
            "total timeout exceeded",
        ));
    }
    let wait = remaining.min(limits.idle);
    tokio::select! {
        () = cancel.cancelled() => Err(ProviderError::cancelled()),
        r = tokio::time::timeout(wait, resp.chunk()) => match r {
            Err(_) => Err(ProviderError::new(
                ErrorCode::ProviderTimeout,
                if wait == limits.idle { "stream idle timeout" } else { "total timeout exceeded" },
            )),
            Ok(Err(e)) => Err(map_reqwest(&e)),
            Ok(Ok(c)) => Ok(c),
        },
    }
}

async fn read_limited(
    mut resp: Response,
    max: usize,
    limits: &HttpLimits,
    started: Instant,
    cancel: &CancelToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut out = Vec::new();
    while let Some(c) = next_chunk(&mut resp, limits, started, cancel).await? {
        if out.len() + c.len() > max {
            return Err(ProviderError::new(
                ErrorCode::InvalidProviderResponse,
                format!("response body exceeds the {max}-byte limit"),
            ));
        }
        out.extend_from_slice(&c);
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Leitor de Server-Sent Events com tetos e cancelamento.
#[derive(Debug)]
pub struct SseReader {
    resp: Response,
    buf: Vec<u8>,
    started: Instant,
    limits: HttpLimits,
    cancel: CancelToken,
    done: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl SseReader {
    /// Próximo evento, `None` no fim do stream.
    pub async fn next(&mut self) -> Result<Option<SseEvent>, ProviderError> {
        loop {
            if let Some(ev) = pop_event(&mut self.buf) {
                if ev.data.is_empty() && ev.event.is_none() {
                    continue; // comentário/keep-alive
                }
                return Ok(Some(ev));
            }
            if self.done {
                // sobra sem linha em branco final: despacha o que houver
                if !self.buf.is_empty() {
                    self.buf.extend_from_slice(b"\n\n");
                    if let Some(ev) = pop_event(&mut self.buf) {
                        self.buf.clear();
                        if !(ev.data.is_empty() && ev.event.is_none()) {
                            return Ok(Some(ev));
                        }
                    }
                }
                return Ok(None);
            }
            if self.buf.len() > self.limits.max_event {
                return Err(ProviderError::new(
                    ErrorCode::InvalidProviderResponse,
                    "stream event exceeds the size limit",
                ));
            }
            match next_chunk(&mut self.resp, &self.limits, self.started, &self.cancel).await? {
                Some(c) => self.buf.extend_from_slice(&c),
                None => self.done = true,
            }
        }
    }
}

/// Remove do buffer o primeiro evento completo (terminado por linha em branco), se houver.
fn pop_event(buf: &mut Vec<u8>) -> Option<SseEvent> {
    // normaliza CRLF/CR para LF só no trecho consumido
    let end = find_blank_line(buf)?;
    let raw: Vec<u8> = buf.drain(..end.consume).collect();
    let text = String::from_utf8_lossy(&raw[..end.content]).into_owned();
    let mut event = None;
    let mut data: Vec<&str> = Vec::new();
    for line in text.split(['\n', '\r']) {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (k, v) = match line.split_once(':') {
            Some((k, v)) => (k, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match k {
            "event" => event = Some(v.to_owned()),
            "data" => data.push(v),
            _ => {}
        }
    }
    Some(SseEvent {
        event,
        data: data.join("\n"),
    })
}

struct Blank {
    content: usize,
    consume: usize,
}

fn find_blank_line(buf: &[u8]) -> Option<Blank> {
    let mut i = 0;
    while i < buf.len() {
        // \n\n  |  \r\n\r\n  |  \r\r  |  \n\r\n
        if buf[i] == b'\n' {
            if buf.get(i + 1) == Some(&b'\n') {
                return Some(Blank {
                    content: i,
                    consume: i + 2,
                });
            }
            if buf.get(i + 1) == Some(&b'\r') && buf.get(i + 2) == Some(&b'\n') {
                return Some(Blank {
                    content: i,
                    consume: i + 3,
                });
            }
        } else if buf[i] == b'\r' {
            if buf.get(i + 1) == Some(&b'\n') {
                if buf.get(i + 2) == Some(&b'\r') && buf.get(i + 3) == Some(&b'\n') {
                    return Some(Blank {
                        content: i,
                        consume: i + 4,
                    });
                }
                if buf.get(i + 2) == Some(&b'\n') {
                    return Some(Blank {
                        content: i,
                        consume: i + 3,
                    });
                }
            } else if buf.get(i + 1) == Some(&b'\r') {
                return Some(Blank {
                    content: i,
                    consume: i + 2,
                });
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn pol(local: bool) -> UrlPolicy {
        UrlPolicy {
            allow_loopback: local,
        }
    }

    #[test]
    fn url_policy_rules() {
        assert!(pol(false).check("https://api.openai.com/v1").is_ok());
        assert!(pol(false).check("http://api.openai.com/v1").is_err());
        assert!(pol(false).check("https://127.0.0.1/").is_err());
        assert!(pol(false).check("https://localhost/").is_err());
        assert!(pol(true).check("http://127.0.0.1:11434/v1").is_ok());
        assert!(pol(true).check("http://localhost:11434").is_ok());
        assert!(pol(true).check("http://[::1]:8080").is_ok());
        assert!(pol(true).check("http://10.0.0.5/").is_err());
        assert!(pol(true).check("http://169.254.169.254/latest").is_err());
        assert!(pol(false).check("https://192.168.1.1/").is_err());
        assert!(pol(false).check("https://100.64.0.1/").is_err());
        assert!(pol(false).check("https://[fe80::1]/").is_err());
        assert!(pol(false).check("https://[::ffff:10.0.0.1]/").is_err());
        assert!(pol(false).check("ftp://example.com/").is_err());
        assert!(pol(false).check("file:///etc/passwd").is_err());
        assert!(pol(false).check("https://user:pw@example.com/").is_err());
        assert!(pol(false).check("https://metadata.internal/").is_err());
    }

    #[test]
    fn sse_parsing_variants() {
        let mut b = b"event: a\ndata: 1\ndata: 2\n\n: keep\n\ndata: [DONE]\r\n\r\n".to_vec();
        let e1 = pop_event(&mut b).unwrap();
        assert_eq!(e1.event.as_deref(), Some("a"));
        assert_eq!(e1.data, "1\n2");
        let e2 = pop_event(&mut b).unwrap(); // comentário
        assert!(e2.data.is_empty() && e2.event.is_none());
        let e3 = pop_event(&mut b).unwrap();
        assert_eq!(e3.data, "[DONE]");
        assert!(pop_event(&mut b).is_none());
        // incompleto não despacha
        let mut part = b"data: x".to_vec();
        assert!(pop_event(&mut part).is_none());
    }
}
