use core::fmt;

/// Falha de render com código estável.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError {
    pub code: &'static str,
    pub message: String,
}

impl RenderError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for RenderError {}

/// Aviso determinístico (não impede o render): ordenado e deduplicado pelo chamador.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenderWarning {
    pub code: &'static str,
    pub message: String,
}

impl RenderWarning {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
