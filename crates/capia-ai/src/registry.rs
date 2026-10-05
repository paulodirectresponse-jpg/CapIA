//! Entidades persistidas (fora do `.capia`, no app DB): `ProviderConfig`, `ModelEndpoint`, `Pricing`.
//! Nada aqui guarda segredo: a credencial é só um `CredentialRef` (ponteiro para o cofre).

use crate::brain::BrainProfile;
use crate::capability::{Capabilities, Capability};
use crate::probe::ProbeResult;
use crate::types::GenParams;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// OpenAI, OpenRouter, Groq, vLLM, Azure-like (por base URL).
    OpenAiCompatible,
    Anthropic,
    Google,
    /// Ollama / LM Studio / llama.cpp server etc. (OpenAI-compatível em loopback).
    LocalOpenAiCompatible,
    /// whisper.cpp server (`/inference`) ou CLI configurado — só STT.
    WhisperLocal,
    /// Determinístico, sem rede (CI).
    Replay,
}

impl ProviderKind {
    /// Roda na máquina do usuário (não envia dados a terceiros).
    pub fn is_local(self) -> bool {
        matches!(
            self,
            Self::LocalOpenAiCompatible | Self::WhisperLocal | Self::Replay
        )
    }

    pub fn needs_credential(self) -> bool {
        matches!(
            self,
            Self::OpenAiCompatible | Self::Anthropic | Self::Google
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    pub requests_per_minute: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub base_url: Option<String>,
    /// `CredentialRef` como string (ponteiro; nunca o segredo).
    pub credential_ref: Option<String>,
    /// Host ao qual a credencial está vinculada (SECURITY §3).
    pub bound_host: Option<String>,
    /// Headers **não secretos**.
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    /// Timeout total da chamada (s).
    #[serde(default = "default_timeout")]
    pub timeout_s: u32,
    /// Conexão (s); padrão 10.
    #[serde(default)]
    pub connect_timeout_s: Option<u32>,
    /// Até o primeiro byte (s); padrão 60.
    #[serde(default)]
    pub first_byte_timeout_s: Option<u32>,
    /// Silêncio máximo entre chunks de um stream (s); padrão 45.
    #[serde(default)]
    pub idle_timeout_s: Option<u32>,
    #[serde(default = "default_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
    /// Aceita URL `http://` em loopback (proxy/gateway local de um provider de nuvem). Opt-in explícito.
    #[serde(default)]
    pub allow_loopback: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: u64,
}

fn default_timeout() -> u32 {
    120
}
fn default_concurrency() -> u32 {
    4
}

const FORBIDDEN_EXTRA_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
    "host",
    "content-length",
    "transfer-encoding",
];

impl ProviderConfig {
    pub fn new(id: impl Into<String>, kind: ProviderKind, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            display_name: display_name.into(),
            base_url: None,
            credential_ref: None,
            bound_host: None,
            extra_headers: BTreeMap::new(),
            timeout_s: default_timeout(),
            connect_timeout_s: None,
            first_byte_timeout_s: None,
            idle_timeout_s: None,
            max_concurrency: default_concurrency(),
            rate_limit: None,
            allow_loopback: false,
            enabled: false,
            created_at: 0,
        }
    }

    /// Valida o que dá para validar sem rede: id, URL (esquema/host), headers extras, limites.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err("provider id must be 1..64 chars [A-Za-z0-9_-]".into());
        }
        if self.timeout_s == 0 || self.timeout_s > 3600 {
            return Err("timeout_s must be in 1..=3600".into());
        }
        if self.max_concurrency == 0 || self.max_concurrency > 64 {
            return Err("max_concurrency must be in 1..=64".into());
        }
        for (k, v) in &self.extra_headers {
            let lk = k.to_ascii_lowercase();
            if FORBIDDEN_EXTRA_HEADERS.contains(&lk.as_str()) {
                return Err(format!(
                    "header `{k}` is reserved (secrets use the credential store)"
                ));
            }
            if k.is_empty()
                || !k
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return Err(format!("invalid header name `{k}`"));
            }
            if v.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) || v.len() > 512 {
                return Err(format!("invalid value for header `{k}`"));
            }
        }
        match (&self.base_url, self.kind) {
            (None, ProviderKind::Replay) => {}
            (
                None,
                ProviderKind::Anthropic | ProviderKind::Google | ProviderKind::OpenAiCompatible,
            ) => {}
            (None, _) => return Err("base_url is required for this provider".into()),
            (Some(u), _) => {
                crate::http::UrlPolicy::for_config(self)
                    .check(u)
                    .map_err(|e| e.message)?;
            }
        }
        Ok(())
    }

    /// URL base efetiva (padrão do fabricante quando não configurada).
    pub fn effective_base_url(&self) -> Option<String> {
        self.base_url.clone().or_else(|| match self.kind {
            ProviderKind::OpenAiCompatible => Some("https://api.openai.com/v1".into()),
            ProviderKind::Anthropic => Some("https://api.anthropic.com/v1".into()),
            ProviderKind::Google => Some("https://generativelanguage.googleapis.com/v1beta".into()),
            _ => None,
        })
    }
}

/// Preço **declarado** (nunca estimado "de cabeça"). Valores em micro-unidades da moeda
/// (ex.: US$ 3,00 por 1M tokens = `3_000_000`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pricing {
    pub currency: String,
    pub input_micros_per_mtok: u64,
    pub output_micros_per_mtok: u64,
    #[serde(default)]
    pub cached_input_micros_per_mtok: Option<u64>,
    #[serde(default)]
    pub audio_micros_per_second: Option<u64>,
    #[serde(default)]
    pub image_micros_each: Option<u64>,
    /// Quem informou (ex.: "usuário", "página de preços do fabricante").
    pub source: String,
    /// Data da informação (ISO-8601), obrigatória.
    pub effective_date: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    #[default]
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelEndpoint {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Capabilities,
    #[serde(default)]
    pub context_window: u32,
    #[serde(default)]
    pub max_output_tokens: u32,
    #[serde(default)]
    pub pricing: Option<Pricing>,
    #[serde(default)]
    pub default_params: GenParams,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_probe: Option<ProbeResult>,
    #[serde(default)]
    pub health: Health,
    /// Falhas transitórias consecutivas (uma falha isolada não remove o modelo).
    #[serde(default)]
    pub consecutive_failures: u32,
}

impl ModelEndpoint {
    pub fn new(
        id: impl Into<String>,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        let model_id = model_id.into();
        Self {
            id: id.into(),
            provider_id: provider_id.into(),
            display_name: model_id.clone(),
            model_id,
            capabilities: Capabilities::new(),
            context_window: 0,
            max_output_tokens: 0,
            pricing: None,
            default_params: GenParams::default(),
            enabled: true,
            last_probe: None,
            health: Health::Unknown,
            consecutive_failures: 0,
        }
    }

    pub fn has(&self, c: Capability) -> bool {
        self.capabilities.has(c)
    }

    /// Registra sucesso/falha de uma chamada. 3 falhas transitórias seguidas ⇒ `Degraded`; 6 ⇒ `Unavailable`.
    pub fn record_outcome(&mut self, ok: bool) {
        if ok {
            self.consecutive_failures = 0;
            self.health = Health::Healthy;
        } else {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            self.health = match self.consecutive_failures {
                0..=2 => self.health,
                3..=5 => Health::Degraded,
                _ => Health::Unavailable,
            };
        }
    }
}

/// Todo o estado de configuração de IA do usuário (um documento JSON no app DB).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Registry {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelEndpoint>,
    #[serde(default)]
    pub profiles: BTreeMap<String, BrainProfile>,
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Modo global: `false` ⇒ nenhuma chamada de IA acontece (AI Off).
    #[serde(default = "default_ai_enabled")]
    pub ai_enabled: bool,
}

fn default_ai_enabled() -> bool {
    true
}

impl Registry {
    pub fn new() -> Self {
        Self {
            ai_enabled: true,
            ..Self::default()
        }
    }

    pub fn upsert_provider(&mut self, p: ProviderConfig) -> Result<(), String> {
        p.validate()?;
        self.providers.insert(p.id.clone(), p);
        Ok(())
    }

    pub fn remove_provider(&mut self, id: &str) {
        self.providers.remove(id);
        self.models.retain(|_, m| m.provider_id != id);
    }

    pub fn upsert_model(&mut self, m: ModelEndpoint) -> Result<(), String> {
        if !self.providers.contains_key(&m.provider_id) {
            return Err(format!("unknown provider `{}`", m.provider_id));
        }
        self.models.insert(m.id.clone(), m);
        Ok(())
    }

    pub fn provider_of(&self, m: &ModelEndpoint) -> Option<&ProviderConfig> {
        self.providers.get(&m.provider_id)
    }

    /// Modelo habilitado cujo provider também está habilitado.
    pub fn usable(&self, m: &ModelEndpoint) -> bool {
        m.enabled && self.provider_of(m).is_some_and(|p| p.enabled)
    }

    pub fn active(&self) -> Option<&BrainProfile> {
        self.active_profile
            .as_ref()
            .and_then(|id| self.profiles.get(id))
    }

    /// Preserva evidência `Probed` ao reimportar uma lista de modelos do provider.
    pub fn merge_remote_models(&mut self, provider_id: &str, remote: &[(String, Option<u32>)]) {
        for (model_id, ctx) in remote {
            let id = format!("{provider_id}:{model_id}");
            self.models.entry(id.clone()).or_insert_with(|| {
                let mut m = ModelEndpoint::new(id, provider_id, model_id.clone());
                m.enabled = false;
                m.context_window = ctx.unwrap_or(0);
                m
            });
        }
    }
}

/// Sugestão editável de provider (base URL e família); **sem** preços.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderPreset {
    pub key: &'static str,
    pub display_name: &'static str,
    pub kind: ProviderKind,
    pub base_url: Option<&'static str>,
}

pub fn provider_presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            key: "openai",
            display_name: "OpenAI",
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("https://api.openai.com/v1"),
        },
        ProviderPreset {
            key: "openrouter",
            display_name: "OpenRouter",
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("https://openrouter.ai/api/v1"),
        },
        ProviderPreset {
            key: "groq",
            display_name: "Groq",
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("https://api.groq.com/openai/v1"),
        },
        ProviderPreset {
            key: "anthropic",
            display_name: "Anthropic",
            kind: ProviderKind::Anthropic,
            base_url: Some("https://api.anthropic.com/v1"),
        },
        ProviderPreset {
            key: "google",
            display_name: "Google Gemini",
            kind: ProviderKind::Google,
            base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
        },
        ProviderPreset {
            key: "ollama",
            display_name: "Ollama (local)",
            kind: ProviderKind::LocalOpenAiCompatible,
            base_url: Some("http://127.0.0.1:11434/v1"),
        },
        ProviderPreset {
            key: "lmstudio",
            display_name: "LM Studio (local)",
            kind: ProviderKind::LocalOpenAiCompatible,
            base_url: Some("http://127.0.0.1:1234/v1"),
        },
        ProviderPreset {
            key: "whisper-cpp",
            display_name: "whisper.cpp server (local)",
            kind: ProviderKind::WhisperLocal,
            base_url: Some("http://127.0.0.1:8080"),
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn validates_reserved_headers_and_urls() {
        let mut p = ProviderConfig::new("p1", ProviderKind::OpenAiCompatible, "x");
        p.base_url = Some("https://api.openai.com/v1".into());
        assert!(p.validate().is_ok());
        p.extra_headers.insert("Authorization".into(), "x".into());
        assert!(p.validate().is_err());
        p.extra_headers.clear();
        p.extra_headers.insert("X-Org".into(), "a\r\nb".into());
        assert!(p.validate().is_err());
        p.extra_headers.clear();
        p.base_url = Some("file:///etc/passwd".into());
        assert!(p.validate().is_err());
        p.base_url = Some("http://example.com/v1".into());
        assert!(p.validate().is_err(), "http só em loopback");
        p.base_url = Some("https://user:pw@example.com/v1".into());
        assert!(p.validate().is_err(), "userinfo proibido");
    }

    #[test]
    fn local_provider_allows_loopback_http() {
        let mut p = ProviderConfig::new("loc", ProviderKind::LocalOpenAiCompatible, "ollama");
        p.base_url = Some("http://127.0.0.1:11434/v1".into());
        assert!(p.validate().is_ok());
        p.base_url = Some("http://169.254.169.254/".into());
        assert!(
            p.validate().is_err(),
            "metadata de nuvem bloqueada mesmo em local"
        );
    }

    #[test]
    fn health_degrades_gradually() {
        let mut m = ModelEndpoint::new("a", "p", "m");
        m.record_outcome(false);
        m.record_outcome(false);
        assert_eq!(m.health, Health::Unknown);
        m.record_outcome(false);
        assert_eq!(m.health, Health::Degraded);
        for _ in 0..3 {
            m.record_outcome(false);
        }
        assert_eq!(m.health, Health::Unavailable);
        m.record_outcome(true);
        assert_eq!(m.health, Health::Healthy);
    }

    #[test]
    fn remove_provider_drops_models() {
        let mut r = Registry::new();
        r.upsert_provider(ProviderConfig::new("p", ProviderKind::Replay, "r"))
            .unwrap();
        r.upsert_model(ModelEndpoint::new("p:m", "p", "m")).unwrap();
        assert!(r.upsert_model(ModelEndpoint::new("q:m", "q", "m")).is_err());
        r.remove_provider("p");
        assert!(r.models.is_empty());
    }
}
