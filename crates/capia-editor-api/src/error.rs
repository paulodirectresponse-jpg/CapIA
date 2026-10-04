//! Erro estruturado da API do editor: `{code, message, details?}`. A UI mostra `message` (traduzida
//! por `code`) e guarda `details` para o diagnóstico expansível (docs/PHASE3 §45).

use capia_commands::CommandError;
use capia_project::ProjectError;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message)
    }

    pub fn no_project() -> Self {
        Self::new("NO_PROJECT_OPEN", "no project is open")
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.details {
            v["details"] = d.clone();
        }
        v
    }
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for ApiError {}

impl From<CommandError> for ApiError {
    fn from(e: CommandError) -> Self {
        let mut details = serde_json::Map::new();
        if let Some(i) = e.command_index {
            details.insert("command_index".into(), json!(i));
        }
        if !e.entities.is_empty() {
            details.insert(
                "entities".into(),
                serde_json::to_value(&e.entities).unwrap_or(Value::Null),
            );
        }
        if let Some(h) = e.hint {
            details.insert("hint".into(), h);
        }
        Self {
            code: e.code.to_string(),
            message: e.message,
            details: (!details.is_empty()).then_some(Value::Object(details)),
        }
    }
}

impl From<ProjectError> for ApiError {
    fn from(e: ProjectError) -> Self {
        let v = e.to_json();
        Self {
            code: e.code(),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_owned),
            details: v.get("details").cloned().or_else(|| {
                // erros de comando/store carregam entities/hint no próprio objeto
                let mut rest = v;
                if let Some(o) = rest.as_object_mut() {
                    o.remove("code");
                    o.remove("message");
                    if !o.is_empty() {
                        return Some(rest);
                    }
                }
                None
            }),
        }
    }
}

impl From<capia_store::StoreError> for ApiError {
    fn from(e: capia_store::StoreError) -> Self {
        ProjectError::Store(e).into()
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(format!("invalid parameters: {e}"))
    }
}
