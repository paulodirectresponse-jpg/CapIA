//! Entrega **endurecida** de webhooks (Fase 6, PHASE6_API_MCP_WEBHOOKS §24..§29): o outro único
//! ponto de rede de saída além dos providers e do `SafeFetcher`. O destino é configurado pelo
//! usuário, mas ainda assim passa pela política de URL: `https` (ou `http` em loopback — o receptor
//! local é o caso de uso principal), sem credencial na URL, DNS filtrado (sem rede privada/
//! link-local/metadata), **nenhum redirect seguido**, timeouts curtos e o corpo da resposta nunca é
//! lido além do status (um endpoint lento ou hostil não segura memória nem a fila).

use crate::error::{ErrorCode, ProviderError};
use crate::http::{GuardedResolver, UrlPolicy};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct WebhookPolicy {
    /// Receptores em `127.0.0.1`/`localhost` (padrão: sim — produto local-first).
    pub allow_loopback: bool,
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
}

impl Default for WebhookPolicy {
    fn default() -> Self {
        Self {
            allow_loopback: true,
            connect_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(15),
        }
    }
}

/// Resultado de uma tentativa (nunca um erro de transporte "cru": mensagem curta, sem URL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

impl Delivery {
    /// 2xx = entregue.
    pub fn delivered(&self) -> bool {
        self.error.is_none() && self.status.is_some_and(|s| (200..300).contains(&s))
    }

    /// Erro do receptor que repetir não resolve (4xx exceto 408/425/429).
    pub fn permanent(&self) -> bool {
        self.status
            .is_some_and(|s| (400..500).contains(&s) && !matches!(s, 408 | 425 | 429))
    }
}

pub struct WebhookClient {
    client: reqwest::Client,
    policy: WebhookPolicy,
}

impl core::fmt::Debug for WebhookClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WebhookClient").finish_non_exhaustive()
    }
}

impl WebhookClient {
    pub fn new(policy: WebhookPolicy) -> Result<Self, ProviderError> {
        let url_policy = UrlPolicy {
            allow_loopback: policy.allow_loopback,
        };
        let client = reqwest::Client::builder()
            .connect_timeout(policy.connect_timeout)
            .timeout(policy.total_timeout)
            // redirect nunca: um 3xx não pode levar o POST (e a assinatura) para outro host
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(GuardedResolver { policy: url_policy }))
            .user_agent(concat!("CapIA-Webhook/", env!("CARGO_PKG_VERSION")))
            .https_only(!policy.allow_loopback)
            .build()
            .map_err(|e| {
                ProviderError::new(ErrorCode::ProviderUnavailable, format!("http client: {e}"))
            })?;
        Ok(Self { client, policy })
    }

    /// Valida o destino **antes** de qualquer rede (registro e cada entrega).
    pub fn check_url(&self, raw: &str) -> Result<url::Url, ProviderError> {
        UrlPolicy {
            allow_loopback: self.policy.allow_loopback,
        }
        .check(raw)
    }

    /// Um POST. Nunca devolve `Err`: a falha de transporte vira `Delivery.error`.
    pub async fn post(
        &self,
        raw_url: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Delivery {
        let t0 = Instant::now();
        let elapsed = || u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        let url = match self.check_url(raw_url) {
            Ok(u) => u,
            Err(e) => {
                return Delivery {
                    status: None,
                    latency_ms: 0,
                    error: Some(e.message),
                };
            }
        };
        let mut req = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        match req.body(body).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                // o corpo é descartado sem ler: só o status importa
                drop(resp);
                let error = if (300..400).contains(&status) {
                    Some("the endpoint answered a redirect; redirects are never followed".into())
                } else {
                    None
                };
                Delivery {
                    status: Some(status),
                    latency_ms: elapsed(),
                    error,
                }
            }
            Err(e) => Delivery {
                status: None,
                latency_ms: elapsed(),
                error: Some(if e.is_timeout() {
                    "the endpoint timed out".to_owned()
                } else if e.is_connect() {
                    "could not connect to the endpoint".to_owned()
                } else {
                    format!("delivery failed: {}", e.without_url())
                }),
            },
        }
    }
}
