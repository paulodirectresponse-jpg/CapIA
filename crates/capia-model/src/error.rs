use core::fmt;
use serde::{Deserialize, Serialize};

/// Códigos de erro estáveis do Command Engine (docs/COMMAND_SYSTEM.md §3). Servem a UI, à IA
/// (autocorreção) e à API. Serializam como `SCREAMING_SNAKE_CASE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // Schema
    InvalidArgument,
    OutOfRange,
    UnsupportedCommand,
    // Precondição
    NotFound,
    TrackLocked,
    NotFrameAligned,
    InsufficientHandles,
    WrongTrackKind,
    NotOnBoundary,
    InvalidSplitPoint,
    OutOfClipRange,
    KeyframeTimeConflict,
    UnresolvedRef,
    // Invariante
    Overlap,
    GapInMagneticTrack,
    NestedCycle,
    NestedDepth,
    DanglingReference,
    LimitExceeded,
    InvariantViolation,
    // Escopo de edição (ripple)
    RippleConflict,
    // Permissão / conflito
    PermissionDenied,
    Conflict,
    // Idempotência / plano
    OperationIdReused,
    OperationIdConflict,
    PlanTokenInvalid,
    PlanExpired,
    PlanConsumed,
    PlanStateChanged,
    PreviewRequired,
    // Histórico
    NothingToUndo,
    NothingToRedo,
    // Consistência interna (bug): op primitiva não bate com o estado.
    OpMismatch,
    // Camada de persistência recusou/falhou (ADR-043): nada mudou em memória.
    PersistenceFailed,
    // Entidade ainda referenciada (ex.: sequence usada como nested).
    InUse,
}

impl ErrorCode {
    /// Nome estável (igual ao serializado).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::UnsupportedCommand => "UNSUPPORTED_COMMAND",
            Self::NotFound => "NOT_FOUND",
            Self::TrackLocked => "TRACK_LOCKED",
            Self::NotFrameAligned => "NOT_FRAME_ALIGNED",
            Self::InsufficientHandles => "INSUFFICIENT_HANDLES",
            Self::WrongTrackKind => "WRONG_TRACK_KIND",
            Self::NotOnBoundary => "NOT_ON_BOUNDARY",
            Self::InvalidSplitPoint => "INVALID_SPLIT_POINT",
            Self::OutOfClipRange => "OUT_OF_CLIP_RANGE",
            Self::KeyframeTimeConflict => "KEYFRAME_TIME_CONFLICT",
            Self::UnresolvedRef => "UNRESOLVED_REF",
            Self::Overlap => "OVERLAP",
            Self::GapInMagneticTrack => "GAP_IN_MAGNETIC_TRACK",
            Self::NestedCycle => "NESTED_CYCLE",
            Self::NestedDepth => "NESTED_DEPTH",
            Self::DanglingReference => "DANGLING_REFERENCE",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::InvariantViolation => "INVARIANT_VIOLATION",
            Self::RippleConflict => "RIPPLE_CONFLICT",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::Conflict => "CONFLICT",
            Self::OperationIdReused => "OPERATION_ID_REUSED",
            Self::OperationIdConflict => "OPERATION_ID_CONFLICT",
            Self::PlanTokenInvalid => "PLAN_TOKEN_INVALID",
            Self::PlanExpired => "PLAN_EXPIRED",
            Self::PlanConsumed => "PLAN_CONSUMED",
            Self::PlanStateChanged => "PLAN_STATE_CHANGED",
            Self::PreviewRequired => "PREVIEW_REQUIRED",
            Self::NothingToUndo => "NOTHING_TO_UNDO",
            Self::NothingToRedo => "NOTHING_TO_REDO",
            Self::OpMismatch => "OP_MISMATCH",
            Self::PersistenceFailed => "PERSISTENCE_FAILED",
            Self::InUse => "IN_USE",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn as_str_matches_the_serialized_name() {
        for code in [
            ErrorCode::InvalidArgument,
            ErrorCode::OutOfRange,
            ErrorCode::NotOnBoundary,
            ErrorCode::KeyframeTimeConflict,
            ErrorCode::OperationIdReused,
            ErrorCode::PlanStateChanged,
            ErrorCode::OpMismatch,
            ErrorCode::PersistenceFailed,
            ErrorCode::InUse,
        ] {
            assert_eq!(
                serde_json::to_string(&code).unwrap(),
                format!("\"{}\"", code.as_str())
            );
        }
    }
}
