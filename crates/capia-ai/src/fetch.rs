//! Download **endurecido** para o Asset Gateway (PHASE5_MEMORY_GATEWAY §13..§15, §30): único ponto
//! de rede do produto fora dos providers de IA. O modelo **nunca** recebe `http.get`: ele pede
//! `gateway.search`/`gateway.fetch`, o adapter decide a URL e este módulo a baixa com:
//!
//! * esquema `https` (ou `http` em loopback só com opt-in explícito de teste), sem credencial na URL;
//! * host **dentro da lista permitida** do adapter — a cada redirect (qualquer outro host: pára);
//! * DNS filtrado (sem IP privado/link-local/metadata), nunca `file://`;
//! * tipo de conteúdo na lista permitida, teto de bytes **durante** o streaming, timeouts;
//! * hash SHA-256 em streaming, escrita em `*.part` e *rename* atômico só ao fim;
//! * cancelamento real (abort do fluxo) e nenhuma credencial seguindo redirect para outro host.

use crate::cancel::CancelToken;
use crate::error::{ErrorCode, ProviderError};
use crate::http::UrlPolicy;
use futures_util::StreamExt as _;
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct FetchPolicy {
    /// Hosts aceitos: `exemplo.com` (exato) ou `*.exemplo.com` (qualquer subdomínio).
    pub allowed_hosts: Vec<String>,
    /// Só para testes/loopback explícito (nunca padrão).
    pub allow_loopback: bool,
    pub max_bytes: u64,
    /// Prefixos de `Content-Type` aceitos (ex.: `video/`, `image/`, `audio/`).
    pub allowed_types: Vec<String>,
    pub max_redirects: usize,
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
            allow_loopback: false,
            max_bytes: 512 * 1024 * 1024,
            allowed_types: vec!["video/".into(), "image/".into(), "audio/".into()],
            max_redirects: 3,
            connect_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(600),
        }
    }
}

impl FetchPolicy {
    pub fn host_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allowed_hosts.iter().any(|p| {
            let p = p.to_ascii_lowercase();
            match p.strip_prefix("*.") {
                Some(suffix) => host.ends_with(&format!(".{suffix}")),
                None => host == p,
            }
        })
    }

    fn url_policy(&self) -> UrlPolicy {
        UrlPolicy {
            allow_loopback: self.allow_loopback,
        }
    }

    fn type_allowed(&self, ct: Option<&str>) -> bool {
        let Some(ct) = ct else { return false };
        let ct = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        self.allowed_types
            .iter()
            .any(|p| ct.starts_with(&p.to_ascii_lowercase()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub path: PathBuf,
    pub bytes: u64,
    /// `sha256:<hex>` do conteúdo baixado.
    pub sha256: String,
    pub content_type: Option<String>,
    pub final_url: String,
}

#[derive(Clone, Copy, Debug)]
struct GuardedResolver {
    policy: UrlPolicy,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
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
            let it: reqwest::dns::Addrs = Box::new(allowed.into_iter());
            Ok(it)
        })
    }
}

pub struct SafeFetcher {
    client: reqwest::Client,
    policy: FetchPolicy,
}

impl core::fmt::Debug for SafeFetcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SafeFetcher")
            .field("hosts", &self.policy.allowed_hosts)
            .finish_non_exhaustive()
    }
}

fn err(code: ErrorCode, msg: impl AsRef<str>) -> ProviderError {
    ProviderError::new(code, msg)
}

impl SafeFetcher {
    pub fn new(policy: FetchPolicy) -> Result<Self, ProviderError> {
        if policy.allowed_hosts.is_empty() {
            return Err(err(
                ErrorCode::NotAllowed,
                "the fetcher has no allowed host",
            ));
        }
        let allowed = Arc::new(policy.clone());
        let allowed2 = allowed.clone();
        let redirect = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= allowed2.max_redirects.min(3) {
                return attempt.error("too many redirects");
            }
            let ok = attempt
                .url()
                .host_str()
                .is_some_and(|h| allowed2.host_allowed(h))
                && allowed2.url_policy().check(attempt.url().as_str()).is_ok();
            if ok { attempt.follow() } else { attempt.stop() }
        });
        let client = reqwest::Client::builder()
            .connect_timeout(policy.connect_timeout)
            .timeout(policy.total_timeout)
            .redirect(redirect)
            .dns_resolver(Arc::new(GuardedResolver {
                policy: allowed.url_policy(),
            }))
            .user_agent("CapIA/0 (+local-first video editor)")
            .https_only(!policy.allow_loopback)
            .build()
            .map_err(|e| err(ErrorCode::ProviderUnavailable, format!("http client: {e}")))?;
        Ok(Self { client, policy })
    }

    pub fn policy(&self) -> &FetchPolicy {
        &self.policy
    }

    /// Valida a URL **antes** de qualquer rede: esquema, credencial, host permitido.
    pub fn check_url(&self, raw: &str) -> Result<url::Url, ProviderError> {
        let url = self.policy.url_policy().check(raw)?;
        let host = url.host_str().unwrap_or_default();
        if !self.policy.host_allowed(host) {
            return Err(err(
                ErrorCode::NotAllowed,
                format!("host `{host}` is not allowed by this source"),
            ));
        }
        Ok(url)
    }

    /// Baixa para `dest` (staging). O arquivo final só aparece (por *rename*) depois de tamanho,
    /// tipo e hash conferidos; qualquer falha remove o `.part`.
    pub async fn download(
        &self,
        raw_url: &str,
        dest: &Path,
        cancel: &CancelToken,
    ) -> Result<Fetched, ProviderError> {
        let url = self.check_url(raw_url)?;
        let part = dest.with_extension("part");
        let res = self.download_inner(url, dest, &part, cancel).await;
        if res.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        res
    }

    async fn download_inner(
        &self,
        url: url::Url,
        dest: &Path,
        part: &Path,
        cancel: &CancelToken,
    ) -> Result<Fetched, ProviderError> {
        let send = self.client.get(url.clone()).send();
        let resp = tokio::select! {
            () = cancel.cancelled() => return Err(ProviderError::cancelled()),
            r = send => r.map_err(|e| err(ErrorCode::ProviderUnavailable, format!("download failed: {}", e.without_url())))?,
        };
        let final_url = resp.url().clone();
        if !final_url
            .host_str()
            .is_some_and(|h| self.policy.host_allowed(h))
        {
            return Err(err(
                ErrorCode::NotAllowed,
                "the download redirected to a host that is not allowed",
            ));
        }
        if resp.status().is_redirection() {
            return Err(err(
                ErrorCode::NotAllowed,
                "redirect to a host that is not allowed was refused",
            ));
        }
        if !resp.status().is_success() {
            return Err(err(
                ErrorCode::ProviderUnavailable,
                format!("the source answered HTTP {}", resp.status().as_u16()),
            ));
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if !self.policy.type_allowed(ct.as_deref()) {
            return Err(err(
                ErrorCode::InvalidRequest,
                format!(
                    "content type `{}` is not allowed",
                    ct.as_deref().unwrap_or("(none)")
                ),
            ));
        }
        if let Some(len) = resp.content_length()
            && len > self.policy.max_bytes
        {
            return Err(err(
                ErrorCode::InvalidRequest,
                format!("the file is larger than {} bytes", self.policy.max_bytes),
            ));
        }
        if let Some(dir) = part.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| err(ErrorCode::ProviderUnavailable, format!("staging: {e}")))?;
        }
        let mut file = std::fs::File::create(part)
            .map_err(|e| err(ErrorCode::ProviderUnavailable, format!("staging: {e}")))?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        loop {
            let next = tokio::select! {
                () = cancel.cancelled() => return Err(ProviderError::cancelled()),
                n = tokio::time::timeout(Duration::from_secs(45), stream.next()) => n,
            };
            let chunk = match next {
                Err(_) => return Err(err(ErrorCode::ProviderUnavailable, "the download stalled")),
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    return Err(err(
                        ErrorCode::ProviderUnavailable,
                        format!("download interrupted: {}", e.without_url()),
                    ));
                }
                Ok(Some(Ok(c))) => c,
            };
            total += chunk.len() as u64;
            if total > self.policy.max_bytes {
                return Err(err(
                    ErrorCode::InvalidRequest,
                    format!("the file exceeds {} bytes", self.policy.max_bytes),
                ));
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .map_err(|e| err(ErrorCode::ProviderUnavailable, format!("staging: {e}")))?;
        }
        file.flush().ok();
        file.sync_all().ok();
        drop(file);
        if total == 0 {
            return Err(err(
                ErrorCode::InvalidRequest,
                "the source returned an empty file",
            ));
        }
        std::fs::rename(part, dest)
            .map_err(|e| err(ErrorCode::ProviderUnavailable, format!("staging: {e}")))?;
        let digest = hasher.finalize();
        let mut hex = String::from("sha256:");
        for b in digest {
            hex.push_str(&format!("{b:02x}"));
        }
        Ok(Fetched {
            path: dest.to_path_buf(),
            bytes: total,
            sha256: hex,
            content_type: ct,
            final_url: final_url.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn host_matching_is_exact_or_subdomain_only() {
        let p = FetchPolicy {
            allowed_hosts: vec!["cdn.example.com".into(), "*.stock.test".into()],
            ..FetchPolicy::default()
        };
        assert!(p.host_allowed("cdn.example.com"));
        assert!(p.host_allowed("CDN.Example.com"));
        assert!(p.host_allowed("a.stock.test"));
        assert!(p.host_allowed("a.b.stock.test"));
        assert!(!p.host_allowed("stock.test"));
        assert!(!p.host_allowed("evil-cdn.example.com"));
        assert!(!p.host_allowed("cdn.example.com.evil.test"));
    }

    #[test]
    fn an_empty_allow_list_cannot_build_a_fetcher_and_urls_are_checked_first() {
        assert!(SafeFetcher::new(FetchPolicy::default()).is_err());
        let f = SafeFetcher::new(FetchPolicy {
            allowed_hosts: vec!["cdn.example.com".into()],
            ..FetchPolicy::default()
        })
        .unwrap();
        for bad in [
            "file:///etc/passwd",
            "http://cdn.example.com/a.mp4",
            "https://user:pw@cdn.example.com/a.mp4",
            "https://other.example.com/a.mp4",
            "https://127.0.0.1/a.mp4",
            "https://169.254.169.254/latest/meta-data",
            "ftp://cdn.example.com/a.mp4",
        ] {
            assert!(f.check_url(bad).is_err(), "{bad}");
        }
        assert!(f.check_url("https://cdn.example.com/a.mp4").is_ok());
    }

    #[test]
    fn content_types_are_matched_by_prefix_and_missing_is_refused() {
        let p = FetchPolicy::default();
        assert!(p.type_allowed(Some("video/mp4")));
        assert!(p.type_allowed(Some("Image/PNG; charset=binary")));
        assert!(!p.type_allowed(Some("text/html")));
        assert!(!p.type_allowed(Some("application/octet-stream")));
        assert!(!p.type_allowed(None));
    }
}
