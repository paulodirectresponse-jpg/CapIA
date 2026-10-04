use core::fmt;
use serde::{Deserialize, Serialize};

/// Códigos estruturados (ADR-047). Nenhum erro de processo/JSON bruto escapa deste crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MediaErrorCode {
    MediaBackendNotFound,
    MediaBackendFailed,
    MediaProbeFailed,
    MediaProbeTimeout,
    MediaProbeOutputTooLarge,
    MediaMetadataInvalid,
    MediaUnsupportedFormat,
    MediaInvalidPath,
    MediaIo,
    MediaCancelled,
    MediaIndexInvalid,
    MediaFrameNotFound,
    MediaEncoderUnavailable,
    /// Encoder fora da política (GPL/não aprovado): nunca é aceito nem substituído em silêncio.
    MediaEncoderProhibited,
    MediaLimitExceeded,
    MediaDecodeFailed,
    MediaEncodeFailed,
}

impl MediaErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MediaBackendNotFound => "MEDIA_BACKEND_NOT_FOUND",
            Self::MediaBackendFailed => "MEDIA_BACKEND_FAILED",
            Self::MediaProbeFailed => "MEDIA_PROBE_FAILED",
            Self::MediaProbeTimeout => "MEDIA_PROBE_TIMEOUT",
            Self::MediaProbeOutputTooLarge => "MEDIA_PROBE_OUTPUT_TOO_LARGE",
            Self::MediaMetadataInvalid => "MEDIA_METADATA_INVALID",
            Self::MediaUnsupportedFormat => "MEDIA_UNSUPPORTED_FORMAT",
            Self::MediaInvalidPath => "MEDIA_INVALID_PATH",
            Self::MediaIo => "MEDIA_IO_ERROR",
            Self::MediaCancelled => "MEDIA_CANCELLED",
            Self::MediaIndexInvalid => "MEDIA_INDEX_INVALID",
            Self::MediaFrameNotFound => "MEDIA_FRAME_NOT_FOUND",
            Self::MediaEncoderUnavailable => "MEDIA_ENCODER_UNAVAILABLE",
            Self::MediaEncoderProhibited => "MEDIA_ENCODER_PROHIBITED",
            Self::MediaLimitExceeded => "MEDIA_LIMIT_EXCEEDED",
            Self::MediaDecodeFailed => "MEDIA_DECODE_FAILED",
            Self::MediaEncodeFailed => "MEDIA_ENCODE_FAILED",
        }
    }
}

impl fmt::Display for MediaErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaError {
    pub code: MediaErrorCode,
    pub message: String,
}

impl MediaError {
    pub fn new(code: MediaErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for MediaError {}
