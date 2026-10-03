use capia_media::{MediaError, MediaErrorCode};
use core::fmt;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetErrorCode {
    AssetFileNotFound,
    AssetNotRegularFile,
    AssetEmptyFile,
    AssetPathInvalid,
    AssetIo,
    AssetChangedDuringImport,
    /// O arquivo mudou enquanto um job (hash/índice/…) o processava: resultado descartado.
    AssetChangedDuringProcessing,
    AssetCancelled,
    AssetHashMismatch,
    AssetOffline,
    /// Falha do backend de mídia, com o código original (`MEDIA_*`).
    Media(MediaErrorCode),
}

impl AssetErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AssetFileNotFound => "ASSET_FILE_NOT_FOUND",
            Self::AssetNotRegularFile => "ASSET_NOT_REGULAR_FILE",
            Self::AssetEmptyFile => "ASSET_EMPTY_FILE",
            Self::AssetPathInvalid => "ASSET_PATH_INVALID",
            Self::AssetIo => "ASSET_IO_ERROR",
            Self::AssetChangedDuringImport => "ASSET_CHANGED_DURING_IMPORT",
            Self::AssetChangedDuringProcessing => "ASSET_CHANGED_DURING_PROCESSING",
            Self::AssetCancelled => "ASSET_CANCELLED",
            Self::AssetHashMismatch => "ASSET_HASH_MISMATCH",
            Self::AssetOffline => "ASSET_OFFLINE",
            Self::Media(m) => m.as_str(),
        }
    }
}

impl fmt::Display for AssetErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erro estruturado de asset. `details` carrega dados para automação (ex.: hashes no mismatch).
#[derive(Clone, Debug, PartialEq)]
pub struct AssetError {
    pub code: AssetErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

impl AssetError {
    pub fn new(code: AssetErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for AssetError {}

impl From<MediaError> for AssetError {
    fn from(e: MediaError) -> Self {
        if e.code == MediaErrorCode::MediaCancelled {
            return Self::new(AssetErrorCode::AssetCancelled, e.message);
        }
        Self::new(AssetErrorCode::Media(e.code), e.message)
    }
}
