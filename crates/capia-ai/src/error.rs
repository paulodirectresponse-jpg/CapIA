//! Taxonomia de erros de provider (AI_PROVIDERS §4/§24). A mensagem é **sempre** redigida na
//! construção: nenhum erro carrega uma credencial.

use capia_secrets::redact;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AuthFailed,
    RateLimited,
    ProviderTimeout,
    ProviderUnavailable,
    InvalidProviderResponse,
    UnsupportedCapability,
    ModelNotFound,
    StructuredOutputInvalid,
    ToolCallInvalid,
    PrivacyPolicyBlocked,
    BudgetExceeded,
    Cancelled,
    // classificação adicional (retry/fallback)
    InvalidRequest,
    ContextTooLong,
    ContentFiltered,
    NoCapableModel,
    NotConfigured,
    NotAllowed,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthFailed => "AUTH_FAILED",
            Self::RateLimited => "RATE_LIMITED",
            Self::ProviderTimeout => "PROVIDER_TIMEOUT",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::InvalidProviderResponse => "INVALID_PROVIDER_RESPONSE",
            Self::UnsupportedCapability => "UNSUPPORTED_CAPABILITY",
            Self::ModelNotFound => "MODEL_NOT_FOUND",
            Self::StructuredOutputInvalid => "STRUCTURED_OUTPUT_INVALID",
            Self::ToolCallInvalid => "TOOL_CALL_INVALID",
            Self::PrivacyPolicyBlocked => "PRIVACY_POLICY_BLOCKED",
            Self::BudgetExceeded => "BUDGET_EXCEEDED",
            Self::Cancelled => "CANCELLED",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::ContextTooLong => "CONTEXT_TOO_LONG",
            Self::ContentFiltered => "CONTENT_FILTERED",
            Self::NoCapableModel => "NO_CAPABLE_MODEL",
            Self::NotConfigured => "NOT_CONFIGURED",
            Self::NotAllowed => "NOT_ALLOWED",
        }
    }

    /// Só transitórios têm retry (nunca auth/inválido/capability).
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ProviderTimeout | Self::ProviderUnavailable
        )
    }

    /// Vale tentar o próximo modelo do fallback? (auth/permissão de um endpoint não condena outro.)
    pub fn fallback_eligible(self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::ProviderTimeout
                | Self::ProviderUnavailable
                | Self::AuthFailed
                | Self::ModelNotFound
                | Self::InvalidProviderResponse
                | Self::StructuredOutputInvalid
                | Self::ToolCallInvalid
                | Self::ContextTooLong
                | Self::NotConfigured
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderError {
    pub code: ErrorCode,
    pub message: String,
    pub status: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub provider: Option<String>,
}

impl ProviderError {
    pub fn new(code: ErrorCode, message: impl AsRef<str>) -> Self {
        Self {
            code,
            message: redact(message.as_ref()),
            status: None,
            retry_after_ms: None,
            provider: None,
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }

    pub fn with_provider(mut self, p: impl Into<String>) -> Self {
        self.provider = Some(p.into());
        self
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCode::Cancelled, "cancelled")
    }

    pub fn unsupported(what: &str) -> Self {
        Self::new(
            ErrorCode::UnsupportedCapability,
            format!("capability not supported: {what}"),
        )
    }

    pub fn retryable(&self) -> bool {
        self.code.retryable()
    }

    /// Classifica um status HTTP (corpo opcional já limitado) em um código canônico.
    pub fn from_http(status: u16, body: &str) -> Self {
        let lower = body.to_ascii_lowercase();
        let code = match status {
            401 | 403 => ErrorCode::AuthFailed,
            404 => ErrorCode::ModelNotFound,
            408 | 504 => ErrorCode::ProviderTimeout,
            413 => ErrorCode::ContextTooLong,
            429 => ErrorCode::RateLimited,
            400 | 422
                if lower.contains("context")
                    && (lower.contains("length")
                        || lower.contains("too long")
                        || lower.contains("maximum")) =>
            {
                ErrorCode::ContextTooLong
            }
            400 | 422 => ErrorCode::InvalidRequest,
            500..=599 => ErrorCode::ProviderUnavailable,
            _ => ErrorCode::InvalidProviderResponse,
        };
        // só um trecho curto do corpo, já redigido, para diagnóstico
        let snippet: String = body.chars().take(300).collect();
        Self::new(code, format!("HTTP {status}: {snippet}")).with_status(status)
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn http_classification() {
        assert_eq!(
            ProviderError::from_http(401, "").code,
            ErrorCode::AuthFailed
        );
        assert_eq!(
            ProviderError::from_http(429, "").code,
            ErrorCode::RateLimited
        );
        assert_eq!(
            ProviderError::from_http(503, "").code,
            ErrorCode::ProviderUnavailable
        );
        assert_eq!(
            ProviderError::from_http(400, "x").code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            ProviderError::from_http(400, "maximum context length exceeded").code,
            ErrorCode::ContextTooLong
        );
        assert!(ErrorCode::RateLimited.retryable());
        assert!(!ErrorCode::AuthFailed.retryable());
    }

    #[test]
    fn messages_are_redacted_at_construction() {
        let e = ProviderError::new(
            ErrorCode::AuthFailed,
            "bad Authorization: Bearer abcdefghijklmnop for ?key=AIzaXXXXXXXXXXXXXXXXXXXXXXXX",
        );
        assert!(!e.message.contains("abcdefghijklmnop"));
        assert!(!e.message.contains("AIzaXXXX"));
    }
}
