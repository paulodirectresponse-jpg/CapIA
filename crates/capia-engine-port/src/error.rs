use capia_intelligence::IntelError;

/// Erro estruturado do EnginePort (código estável + mensagem).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortError {
    pub code: String,
    pub message: String,
}

impl PortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl core::fmt::Display for PortError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PortError {}

impl From<PortError> for IntelError {
    fn from(e: PortError) -> Self {
        IntelError::new(e.code, e.message)
    }
}
