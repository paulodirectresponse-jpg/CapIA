use capia_model::{EntityRef, ErrorCode, OpError, Violation};
use capia_time::TimeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Erro estruturado e acionável (docs/COMMAND_SYSTEM.md §3): a IA lê `code`, `entities` e `hint`
/// para se autocorrigir; a UI mostra `message`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
    /// Índice do comando na transação que falhou (quando aplicável).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<Value>,
}

pub type Result<T> = core::result::Result<T, CommandError>;

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            command_index: None,
            entities: Vec::new(),
            hint: None,
        }
    }

    pub fn with_entities(mut self, entities: impl IntoIterator<Item = EntityRef>) -> Self {
        self.entities = entities.into_iter().collect();
        self
    }

    pub fn with_hint(mut self, hint: Value) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn at(mut self, index: usize) -> Self {
        self.command_index = Some(index);
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn not_found(what: &str, id: impl core::fmt::Display) -> Self {
        Self::new(ErrorCode::NotFound, format!("{what} {id} does not exist"))
    }

    /// Violação de invariante vira erro de comando (usa o código da violação).
    pub fn from_violation(v: &Violation) -> Self {
        Self {
            code: v.code,
            message: v.message.clone(),
            command_index: None,
            entities: v.entities.clone(),
            hint: None,
        }
    }
}

impl core::fmt::Display for CommandError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for CommandError {}

impl From<TimeError> for CommandError {
    fn from(e: TimeError) -> Self {
        match e {
            TimeError::Overflow => Self::new(ErrorCode::OutOfRange, "time value out of range"),
            TimeError::DivideByZero | TimeError::InvalidRatio => Self::invalid(e.to_string()),
        }
    }
}

impl From<OpError> for CommandError {
    fn from(e: OpError) -> Self {
        Self::new(e.code, e.message)
    }
}
