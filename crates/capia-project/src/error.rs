//! Erro unificado da fachada: cada camada mantém seu erro estruturado; aqui só se junta para os
//! adaptadores (CLI hoje) imprimirem JSON estável.

use capia_assets::AssetError;
use capia_commands::CommandError;
use capia_store::StoreError;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum ProjectError {
    Store(StoreError),
    Asset(AssetError),
    Command(CommandError),
    /// Pedido inválido da fachada (asset inexistente, não gerenciado, argumento ruim…).
    Invalid {
        code: &'static str,
        message: String,
        details: Option<Value>,
    },
}

impl ProjectError {
    pub(crate) fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn code(&self) -> String {
        match self {
            Self::Store(e) => e.code.to_string(),
            Self::Asset(e) => e.code.to_string(),
            Self::Command(e) => e.code.to_string(),
            Self::Invalid { code, .. } => (*code).to_owned(),
        }
    }

    /// Representação estruturada estável (`{code, message, details?, …}`).
    pub fn to_json(&self) -> Value {
        match self {
            Self::Store(e) => serde_json::to_value(e).unwrap_or(Value::Null),
            Self::Command(e) => serde_json::to_value(e).unwrap_or(Value::Null),
            Self::Asset(e) => {
                let mut v = json!({ "code": e.code.as_str(), "message": e.message });
                if let Some(d) = &e.details {
                    v["details"] = d.clone();
                }
                v
            }
            Self::Invalid {
                code,
                message,
                details,
            } => {
                let mut v = json!({ "code": code, "message": message });
                if let Some(d) = details {
                    v["details"] = d.clone();
                }
                v
            }
        }
    }
}

impl core::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Asset(e) => write!(f, "{e}"),
            Self::Command(e) => write!(f, "{e}"),
            Self::Invalid { code, message, .. } => write!(f, "{code}: {message}"),
        }
    }
}

impl core::error::Error for ProjectError {}

impl From<StoreError> for ProjectError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}
impl From<AssetError> for ProjectError {
    fn from(e: AssetError) -> Self {
        Self::Asset(e)
    }
}
impl From<CommandError> for ProjectError {
    fn from(e: CommandError) -> Self {
        Self::Command(e)
    }
}
