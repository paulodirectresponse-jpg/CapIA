//! Envelope de erro da API (`{code, message, details?, request_id}`) e o mapa código → HTTP.

use capia_editor_api::ApiError;
use capia_intelligence::IntelError;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct ApiErr {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
    /// Segundos até repetir (429/503).
    pub retry_after: Option<u64>,
}

impl ApiErr {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_owned(),
            message: message.into(),
            details: None,
            retry_after: None,
        }
    }

    pub fn with_details(mut self, d: Value) -> Self {
        self.details = Some(d);
        self
    }

    pub fn with_retry(mut self, secs: u64) -> Self {
        self.retry_after = Some(secs);
        self
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(400, "BAD_REQUEST", msg)
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(422, "INVALID_PARAMS", msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(404, "NOT_FOUND", msg)
    }

    pub fn conflict(code: &str, msg: impl Into<String>) -> Self {
        Self::new(409, code, msg)
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(401, "UNAUTHORIZED", msg)
    }

    pub fn forbidden(code: &str, msg: impl Into<String>) -> Self {
        Self::new(403, code, msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(500, "INTERNAL", msg)
    }

    pub fn unavailable(code: &str, msg: impl Into<String>) -> Self {
        Self::new(503, code, msg)
    }

    /// Corpo JSON público; a mensagem passa pelo redator central (nenhum segredo sai pela API).
    pub fn body(&self, request_id: &str) -> Value {
        let mut v = json!({
            "code": self.code,
            "message": capia_secrets::redact_global(&self.message),
            "request_id": request_id,
        });
        if let Some(d) = &self.details {
            v["details"] = d.clone();
        }
        v
    }
}

impl core::fmt::Display for ApiErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiErr {}

/// Código do engine/IA → status HTTP. Desconhecido ⇒ 422 (pedido entendido mas recusado).
fn status_of(code: &str) -> u16 {
    match code {
        "NOT_FOUND" | "RUN_NOT_FOUND" | "ASSET_NOT_FOUND" | "SEQUENCE_NOT_FOUND" => 404,
        "NO_PROJECT_OPEN" | "PROJECT_NOT_OPEN" => 409,
        "STALE_PLAN" | "PLAN_STALE" | "PLAN_STATE_CHANGED" | "PLAN_EXPIRED" | "REVISION_CONFLICT"
        | "CONFLICT" | "OVERLAP" | "PLAN_DRIFT" | "ILLEGAL_TRANSITION" | "RUN_BUSY"
        | "TOO_MANY_RUNS" | "INVALID_STATE" => 409,
        "PERMISSION_DENIED" => 403,
        "UNKNOWN_METHOD" => 404,
        "INTERNAL" | "POISONED" => 500,
        "AI_OFF" | "AI_DISABLED" | "NOT_CONFIGURED" => 409,
        "BAD_JSON" | "INVALID_PARAMS" => 400,
        _ => 422,
    }
}

impl From<ApiError> for ApiErr {
    fn from(e: ApiError) -> Self {
        let j = e.to_json();
        let code = j["code"].as_str().unwrap_or("ENGINE_ERROR").to_owned();
        let mut out = Self::new(
            status_of(&code),
            &code,
            j["message"].as_str().unwrap_or("engine error"),
        );
        if let Some(d) = j.get("details").filter(|d| !d.is_null()) {
            out.details = Some(d.clone());
        }
        out
    }
}

impl From<IntelError> for ApiErr {
    fn from(e: IntelError) -> Self {
        Self::new(status_of(&e.code), &e.code, e.message)
    }
}

impl ApiErr {
    /// Erro `{code, message}` já serializado pelo serviço `ai.*`.
    pub fn from_json(v: &Value) -> Self {
        let code = v["code"].as_str().unwrap_or("AI_ERROR").to_owned();
        Self::new(
            status_of(&code),
            &code,
            v["message"].as_str().unwrap_or("ai error"),
        )
    }
}

impl From<capia_store::StoreError> for ApiErr {
    fn from(e: capia_store::StoreError) -> Self {
        Self::internal(format!("server database error: {}", e.message))
    }
}

pub type ApiResult<T> = Result<T, ApiErr>;
