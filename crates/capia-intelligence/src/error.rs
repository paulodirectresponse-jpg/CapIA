//! Erro estruturado da camada de inteligência. Códigos estáveis para a UI/CLI; mensagens já
//! redigidas (nunca carregam segredo) e sem caminhos de mídia do usuário além do necessário.

use capia_ai::ProviderError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelError {
    pub code: String,
    pub message: String,
}

impl IntelError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: capia_secrets::redact_registered_global(&message.into()),
        }
    }

    pub fn cancelled() -> Self {
        Self::new("CANCELLED", "the task was cancelled")
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == "CANCELLED"
    }
}

impl core::fmt::Display for IntelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for IntelError {}

impl From<ProviderError> for IntelError {
    fn from(e: ProviderError) -> Self {
        Self::new(e.code.as_str(), e.message)
    }
}

impl From<capia_media::MediaError> for IntelError {
    fn from(e: capia_media::MediaError) -> Self {
        Self::new(format!("{:?}", e.code).to_uppercase(), e.message)
    }
}

impl From<capia_store::StoreError> for IntelError {
    fn from(e: capia_store::StoreError) -> Self {
        Self::new("STORE_ERROR", e.to_string())
    }
}

pub type IntelResult<T> = Result<T, IntelError>;
