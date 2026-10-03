//! Executor de jobs do CapIA (ADR-052). Genérico: não conhece projeto, mídia nem SQLite — quem
//! persiste implementa [`JobSink`]. Garantias:
//!
//! * **limitado**: N workers fixos e uma fila **limitada** por categoria (nada de thread por job);
//! * **prioridade sem starvation**: `interactive` > `normal` > `background` por *créditos* (6/3/1 por
//!   ciclo), então o background sempre avança;
//! * **cancelamento cooperativo real**: [`CancelToken`] chega ao job; jobs de mídia o repassam ao
//!   processo filho (que é morto);
//! * **deduplicação** por chave: o mesmo trabalho ativo não roda duas vezes;
//! * um *panic* num job vira falha estruturada, nunca derruba o worker.
//!
//! Este crate faz IO apenas de relógio/threads e **não** compila para WASM.

mod executor;
mod types;

pub use executor::{Executor, ExecutorConfig, JobHandle, Submitted};
pub use types::{
    CODE_CANCELLED, CODE_PANICKED, CancelToken, JobCtx, JobError, JobId, JobKind, JobSink,
    JobSnapshot, JobSpec, JobState, Priority, Progress, SubmitError,
};
