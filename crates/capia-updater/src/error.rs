//! Erros do updater (estruturados; nunca carregam segredo).

use crate::verify::VerifyError;
use std::fmt;

#[derive(Debug)]
pub enum UpdateError {
    Invalid(String),
    SignatureMissing,
    Signature(VerifyError),
    /// A política recusou (canal, downgrade, versão mínima, versão rejeitada...).
    Rejected(String),
    /// Estado inválido para a operação pedida.
    WrongState(String),
    HashMismatch(String),
    Io(String),
    Download(String),
    Switch(String),
    /// A operação foi adiada (Run de IA ativa / escrita crítica).
    Deferred(String),
    /// Simula a morte do processo (só em testes de falha).
    Killed,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "invalid: {m}"),
            Self::SignatureMissing => f.write_str("manifest is not signed"),
            Self::Signature(e) => write!(f, "signature: {e}"),
            Self::Rejected(m) => write!(f, "rejected: {m}"),
            Self::WrongState(m) => write!(f, "wrong state: {m}"),
            Self::HashMismatch(m) => write!(f, "hash mismatch: {m}"),
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Download(m) => write!(f, "download: {m}"),
            Self::Switch(m) => write!(f, "switch: {m}"),
            Self::Deferred(m) => write!(f, "deferred: {m}"),
            Self::Killed => f.write_str("process killed (fault injection)"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<VerifyError> for UpdateError {
    fn from(e: VerifyError) -> Self {
        Self::Signature(e)
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
