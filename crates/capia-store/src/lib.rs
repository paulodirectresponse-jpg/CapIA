//! Persistência do CapIA: o arquivo `.capia` (SQLite, ADR-042/043/044).
//!
//! O store **não edita** nada: recebe do `Engine` (via [`capia_commands::journal::Journal`]) o que
//! mudou e grava numa única transação SQLite — entrada de histórico completa, evento, registro de
//! `operation_id`, resultado original e, de tempos em tempos, um snapshot do documento. Abrir o
//! projeto = último snapshot + replay dos eventos + verificações de consistência.
//!
//! Este crate faz IO e **não** compila para WASM; o núcleo puro (`capia-time/model/commands`) segue
//! livre de SQLite (verificado por `tools/check-architecture.mjs`).

mod ai;
mod appdb;
mod autonomy;
mod catalog;
mod error;
mod failpoints;
mod jobs;
mod load;
mod schema;
mod serverdb;
mod store;

pub use ai::{AiStore, MAX_RECORD_JSON, RecordRow, UsageRow, UsageSummary};
pub use appdb::{APP_APPLICATION_ID, APP_MIGRATIONS, APP_SCHEMA_VERSION, AppDb};
pub use autonomy::{
    AutonomyStore, Claim, EffectRow, EventRow, LedgerRow, MAX_AUTONOMY_JSON, MemoryLogRow,
    MemoryRow, ProvenanceRow, RunRow, RunUpdate, StageRow,
};
pub use catalog::{Catalog, CatalogEvent, CatalogEventKind, CatalogOp, PendingCatalog};
pub use error::{StoreError, StoreErrorCode, StoreResult};
pub use jobs::{JobStore, Recovery, TicketRow, TicketState};
pub use load::StoreStats;
pub use schema::{APPLICATION_ID, CURRENT_SCHEMA_VERSION, MIGRATIONS, Migration};
pub use serverdb::{
    AuditRow, DeliveryRow, ExportRow, IdemBegin, ProjectRow, SERVER_APPLICATION_ID,
    SERVER_MIGRATIONS, SERVER_SCHEMA_VERSION, ServerDb, ServerEventRow, TokenRow, WebhookRow,
};
pub use store::{
    ProjectInfo, ProjectStore, SequenceInfo, StoreOptions, StoredOperation, Synchronous,
    ValidationReport, random_plan_key,
};
