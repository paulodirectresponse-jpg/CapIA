//! Configuração do servidor. Os padrões são os **seguros**: loopback, CORS fechado, limites finitos.

use capia_ai::webhook::WebhookPolicy;
use capia_editor_api::SessionConfig;
use capia_secrets::SecretStore;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct ServerConfig {
    /// Raiz de dados do servidor: `server.db`, `app.db`, `projects/`, `uploads/`, `exports/`.
    pub data_dir: PathBuf,
    pub bind: IpAddr,
    /// `0` = porta aleatória (publicada em `server.json`).
    pub port: u16,
    /// Bind fora do loopback exige as DUAS confirmações (nunca token em texto claro por padrão).
    pub allow_remote: bool,
    pub remote_tls_terminated_by_proxy: bool,
    pub workers: usize,
    /// Conexões aguardando um worker; além disso o servidor responde 503 (backpressure).
    pub queue: usize,
    pub max_json_bytes: usize,
    pub max_json_depth: usize,
    pub max_upload_bytes: u64,
    pub upload_quota_bytes: u64,
    pub upload_timeout: Duration,
    pub max_concurrent_uploads: usize,
    pub max_sse: usize,
    /// Origens CORS permitidas (vazio = CORS negado: qualquer requisição com `Origin` leva 403).
    pub cors_origins: Vec<String>,
    /// Hosts extras aceitos no cabeçalho `Host` (além de loopback:porta).
    pub allowed_hosts: Vec<String>,
    /// Multiplica capacidade/reposição dos limites (testes de carga/abuso usam < 1 ou > 1).
    pub rate_scale: f64,
    pub webhook: WebhookPolicy,
    pub webhook_max_attempts: u32,
    pub webhook_backoff_base: Duration,
    pub webhook_backoff_cap: Duration,
    /// `pending` de idempotência mais velho que isto veio de um processo que caiu.
    pub idempotency_stale: Duration,
    pub session: SessionConfig,
    pub secrets: Arc<dyn SecretStore>,
    /// Cérebro Replay determinístico (só com a feature `testkit`; nunca no produto).
    pub demo_brain: bool,
}

impl core::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("data_dir", &self.data_dir)
            .field("bind", &self.bind)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl ServerConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            allow_remote: false,
            remote_tls_terminated_by_proxy: false,
            workers: 8,
            queue: 64,
            max_json_bytes: 1 << 20,
            max_json_depth: 32,
            max_upload_bytes: 4 << 30,
            upload_quota_bytes: 16 << 30,
            upload_timeout: Duration::from_secs(30 * 60),
            max_concurrent_uploads: 4,
            max_sse: 4,
            cors_origins: Vec::new(),
            allowed_hosts: Vec::new(),
            rate_scale: 1.0,
            webhook: WebhookPolicy::default(),
            webhook_max_attempts: 8,
            webhook_backoff_base: Duration::from_secs(5),
            webhook_backoff_cap: Duration::from_secs(3600),
            idempotency_stale: Duration::from_secs(300),
            session: SessionConfig::default(),
            // sem cofre do SO (Linux/CI) o segredo de webhook fica só em memória: nunca em arquivo
            secrets: capia_secrets::platform_store()
                .unwrap_or_else(|_| Arc::new(capia_secrets::MemoryStore::new())),
            demo_brain: false,
        }
    }

    pub fn is_loopback(&self) -> bool {
        self.bind.is_loopback()
    }

    /// Recusa configurações perigosas **antes** de abrir o socket.
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_loopback() && !(self.allow_remote && self.remote_tls_terminated_by_proxy) {
            return Err(format!(
                "refusing to bind {}: a non-loopback address needs --allow-remote AND \
                 --remote-tls-terminated-by-proxy (tokens must never travel in clear text; \
                 put a TLS reverse proxy in front, see docs/api/security.md)",
                self.bind
            ));
        }
        if self.workers == 0 || self.workers > 256 {
            return Err("workers must be between 1 and 256".into());
        }
        if self.queue == 0 {
            return Err("queue must be at least 1".into());
        }
        if self.max_json_bytes == 0 || self.max_json_bytes > 64 << 20 {
            return Err("max_json_bytes must be between 1 and 64 MiB".into());
        }
        for o in &self.cors_origins {
            if o == "*" {
                return Err("CORS origin `*` is not allowed: list explicit origins".into());
            }
        }
        Ok(())
    }
}
