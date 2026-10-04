//! Command Engine do CapIA (docs/COMMAND_SYSTEM.md).
//!
//! **Única porta de escrita** do documento. Comandos estruturados e versionados ([`Command`]) são
//! expandidos em ops primitivas invertíveis, validados (invariantes de `capia-model`) e commitados
//! em transações atômicas por [`Engine`], com:
//!
//! * `operation_id` idempotente (ADR-029) — replays nunca duplicam edições;
//! * `preview → apply_plan` com token HMAC (ADR-030) — `Agent`/`Api` só escrevem por aí;
//! * undo/redo por `inverse_ops` gravadas, histórico e auditoria;
//! * erros estruturados e acionáveis ([`CommandError`]).
//!
//! Funções puras de UX/engine: [`resolve_placement`], [`resolve_snap`], [`threshold_ticks`],
//! [`resolve_group_move`], [`eval_property`]. Puro: sem IO, relógio ou aleatoriedade; compila para
//! WASM (ADR-016).

mod command;
mod ctx;
mod engine;
mod error;
mod exec;
pub mod group_move;
pub mod hash;
pub mod journal;
pub mod placement;
mod query;
pub mod snap;

pub use command::{
    COMMAND_SCHEMA_VERSION, ClipMove, Command, CommandEnvelope, Edge, NewClip, RippleScope,
    Transaction, VariantSpec, VariantSwap,
};
pub use ctx::CommandOutput;
pub use engine::{
    Actor, ActorKind, AppliedOperation, AuditEvent, AuditKind, CommandSummary, CommitResult,
    Engine, EngineConfig, EngineState, HistoryEntry, PreviewResult,
};
pub use error::{CommandError, Result};
pub use group_move::{GroupMove, GroupMoveRequest, GroupSnap, resolve_group_move};
pub use hash::document_digest;
pub use placement::{InsertSide, Placement, PlacementStrategy, resolve_placement};
pub use query::eval_property;
pub use snap::{
    SnapRequest, SnapResult, SnapTarget, SnapTargetKind, resolve_point_snap, resolve_snap,
    threshold_ticks,
};
