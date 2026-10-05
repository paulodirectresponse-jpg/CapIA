//! Erros do crate de suporte (sempre estruturados e sem segredo).

use std::fmt;

#[derive(Debug)]
pub enum SupportError {
    Io(String),
    Store(String),
    Invalid(String),
    /// Nenhum destino de envio configurado (pendência externa): nada é enviado.
    SinkNotConfigured,
}

impl fmt::Display for SupportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Store(m) => write!(f, "store: {m}"),
            Self::Invalid(m) => write!(f, "invalid: {m}"),
            Self::SinkNotConfigured => f.write_str("crash upload endpoint is not configured"),
        }
    }
}

impl std::error::Error for SupportError {}

impl From<std::io::Error> for SupportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(capia_secrets::redact_global(&e.to_string()))
    }
}

impl From<capia_store::StoreError> for SupportError {
    fn from(e: capia_store::StoreError) -> Self {
        Self::Store(capia_secrets::redact_global(&e.to_string()))
    }
}

impl From<zip::result::ZipError> for SupportError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Io(e.to_string())
    }
}
