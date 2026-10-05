//! Suporte do app (Fase 6, Track C): build info, logs com rotação limitada, diagnóstico redigido com
//! preview, preferências de privacidade e crash report **opt-in**.
//!
//! Regras: sem código de rede (o destino de upload é externo, via [`CrashSink`]); todo texto que sai do
//! processo passa por `capia_secrets::redact_global`; nenhum conteúdo de projeto/mídia/prompt.

mod buildinfo;
mod crash;
mod diag;
mod error;
mod logs;
mod settings;
mod util;

pub use buildinfo::{APP_VERSION, BuildInfo, SystemInfo};
pub use crash::{
    CrashRecord, CrashReporter, CrashSink, CrashStatus, CrashStore, DEFAULT_MAX_RECORDS,
    FlushReport, UnconfiguredSink,
};
pub use diag::{
    DiagnosticBuilder, DiagnosticPreview, ERRORS_STEM, NEVER_INCLUDED, PreviewEntry,
    StructuredErrorLog, SupportDirs,
};
pub use error::SupportError;
pub use logs::{LogPolicy, MAX_LINE_BYTES, RotatingLog};
pub use settings::{NAMESPACE, SettingsStore, SupportSettings};
pub use util::{now_ms, sanitize, scrub_paths};
