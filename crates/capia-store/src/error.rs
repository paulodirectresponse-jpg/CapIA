//! Erros estruturados do store. Erro cru do SQLite nunca sobe: vira um código estável + `cause`.

use capia_commands::journal::JournalError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StoreErrorCode {
    /// O caminho não existe.
    ProjectNotFound,
    /// `create` em caminho que já existe.
    ProjectAlreadyExists,
    /// Arquivo vazio, não-SQLite ou SQLite sem a assinatura do CapIA.
    NotACapiaProject,
    /// Arquivo CapIA com conteúdo inconsistente/corrompido.
    ProjectCorrupted,
    /// Schema mais novo que este software (o arquivo **não** é tocado).
    UnsupportedSchemaVersion,
    MigrationFailed,
    StoreIoError,
    /// Outro escritor detém o lock além do `busy_timeout`.
    StoreBusy,
    /// O banco avançou além do que este engine conhece (escritor obsoleto): reabra o projeto.
    StoreConflict,
    TransactionFailed,
    InvalidArgument,
}

impl StoreErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectNotFound => "PROJECT_NOT_FOUND",
            Self::ProjectAlreadyExists => "PROJECT_ALREADY_EXISTS",
            Self::NotACapiaProject => "NOT_A_CAPIA_PROJECT",
            Self::ProjectCorrupted => "PROJECT_CORRUPTED",
            Self::UnsupportedSchemaVersion => "UNSUPPORTED_SCHEMA_VERSION",
            Self::MigrationFailed => "MIGRATION_FAILED",
            Self::StoreIoError => "STORE_IO_ERROR",
            Self::StoreBusy => "STORE_BUSY",
            Self::StoreConflict => "STORE_CONFLICT",
            Self::TransactionFailed => "TRANSACTION_FAILED",
            Self::InvalidArgument => "INVALID_ARGUMENT",
        }
    }
}

impl core::fmt::Display for StoreErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoreError {
    pub code: StoreErrorCode,
    pub message: String,
    /// Causa interna (texto do SQLite/IO) para diagnóstico — não para exibir ao usuário final.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

pub type StoreResult<T> = Result<T, StoreError>;

impl StoreError {
    pub fn new(code: StoreErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            cause: None,
            details: None,
        }
    }

    pub fn with_cause(mut self, cause: impl core::fmt::Display) -> Self {
        self.cause = Some(cause.to_string());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn corrupted(message: impl Into<String>) -> Self {
        Self::new(StoreErrorCode::ProjectCorrupted, message)
    }
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        use rusqlite::ffi::ErrorCode as Sql;
        let code = match &e {
            rusqlite::Error::SqliteFailure(f, _) => match f.code {
                Sql::DatabaseBusy | Sql::DatabaseLocked => StoreErrorCode::StoreBusy,
                Sql::NotADatabase => StoreErrorCode::NotACapiaProject,
                Sql::DatabaseCorrupt => StoreErrorCode::ProjectCorrupted,
                Sql::CannotOpen | Sql::SystemIoFailure | Sql::DiskFull | Sql::ReadOnly => {
                    StoreErrorCode::StoreIoError
                }
                _ => StoreErrorCode::TransactionFailed,
            },
            _ => StoreErrorCode::TransactionFailed,
        };
        let message = match code {
            StoreErrorCode::StoreBusy => "the project is locked by another writer",
            StoreErrorCode::NotACapiaProject => "the file is not a CapIA project",
            StoreErrorCode::ProjectCorrupted => "the project file is corrupted",
            StoreErrorCode::StoreIoError => "the project file could not be read or written",
            _ => "the storage transaction failed",
        };
        Self::new(code, message).with_cause(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            StoreErrorCode::ProjectNotFound
        } else {
            StoreErrorCode::StoreIoError
        };
        Self::new(code, "file system error").with_cause(e)
    }
}

impl From<StoreError> for JournalError {
    fn from(e: StoreError) -> Self {
        JournalError {
            kind: e.code.as_str().to_owned(),
            message: e.message,
            cause: e.cause,
        }
    }
}
