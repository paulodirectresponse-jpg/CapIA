//! Porta de persistência do engine (ADR-043). O `Engine` é puro: quem quiser durabilidade injeta
//! um [`Journal`]. A regra é **persistir antes de publicar**: o engine só altera seu estado em
//! memória depois que `Journal::append` devolve `Ok`; se falhar, nada muda (`PERSISTENCE_FAILED`).

use crate::engine::{AppliedOperation, AuditEvent, CommitResult, HistoryEntry};
use capia_model::Document;
use serde::{Deserialize, Serialize};

/// Falha da camada de persistência, sem acoplar o engine a nenhum backend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalError {
    /// Código estável do backend (ex.: `STORE_BUSY`, `STORE_CONFLICT`, `TRANSACTION_FAILED`).
    pub kind: String,
    pub message: String,
    /// Causa interna para diagnóstico (nunca para a UI final).
    pub cause: Option<String>,
}

impl JournalError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            cause: None,
        }
    }
}

impl core::fmt::Display for JournalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl core::error::Error for JournalError {}

/// O que o engine está prestes a publicar. Tudo o que muda junto é entregue junto, para o backend
/// gravar numa **única** transação (documento ⇔ histórico ⇔ log de operações nunca divergem).
#[derive(Debug)]
pub enum JournalRecord<'a> {
    /// Transação de comandos confirmada: entrada de histórico completa (ops + inversas), resultado
    /// original (para replay idempotente) e os `operation_id` registrados.
    Commit {
        entry: &'a HistoryEntry,
        result: &'a CommitResult,
        applied: &'a [(String, AppliedOperation)],
        event: &'a AuditEvent,
    },
    /// Undo da última entrada aplicada (aplica as `inverse_ops` já gravadas).
    Undo { event: &'a AuditEvent },
    /// Redo da primeira entrada desfeita.
    Redo { event: &'a AuditEvent },
}

impl JournalRecord<'_> {
    pub fn event(&self) -> &AuditEvent {
        match self {
            Self::Commit { event, .. } | Self::Undo { event } | Self::Redo { event } => event,
        }
    }
}

/// Backend durável. `doc_after` é o documento **depois** do registro (para snapshots).
/// Implementações devem ser atômicas: ou gravam tudo ou nada, e conferir que o *head* do banco é
/// a revisão anterior à do evento (`event.revision - 1`), recusando escritor obsoleto.
pub trait Journal: Send {
    fn append(
        &mut self,
        record: &JournalRecord<'_>,
        doc_after: &Document,
    ) -> Result<(), JournalError>;
}
