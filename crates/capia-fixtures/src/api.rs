//! Sonda da Engine API (a mesma fachada que UI/REST/MCP futuros usam): `Session::call` com medição.
//! Só leitura de temporização — as chamadas de escrita passam pelo Command Engine como sempre.

use capia_editor_api::{ApiError, Reply, Session, SessionConfig};
use capia_media::MediaConfig;
use capia_store::{StoreOptions, Synchronous};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

/// Resultado de uma chamada medida.
#[derive(Debug)]
pub struct Timed {
    pub elapsed: Duration,
    /// Tamanho do JSON de resposta em bytes (calculado **fora** do tempo medido).
    pub reply_bytes: usize,
    pub value: Value,
}

#[derive(Debug)]
pub struct ApiProbe {
    session: Session,
}

impl ApiProbe {
    /// Sessão com `Synchronous::Full` (durabilidade real) e o FFmpeg que houver no PATH.
    pub fn new() -> Self {
        let cfg = SessionConfig {
            media: MediaConfig::default(),
            store: StoreOptions {
                synchronous: Synchronous::Full,
                ..StoreOptions::default()
            },
        };
        Self {
            session: Session::new(cfg),
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<Timed, ApiError> {
        self.call(
            "project.open",
            json!({ "path": path.display().to_string() }),
        )
    }

    pub fn close(&mut self) -> Result<Timed, ApiError> {
        self.call("project.close", json!({}))
    }

    /// Chama `method` e mede só a chamada.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Timed, ApiError> {
        let t = Instant::now();
        let reply = self.session.call(method, params)?;
        let elapsed = t.elapsed();
        let value = match reply {
            Reply::Json(v) => v,
            Reply::Binary { bytes, meta, .. } => {
                json!({ "binary_bytes": bytes.len(), "meta": meta })
            }
        };
        let reply_bytes = serde_json::to_string(&value).map_or(0, |s| s.len());
        Ok(Timed {
            elapsed,
            reply_bytes,
            value,
        })
    }

    pub fn session(&mut self) -> &mut Session {
        &mut self.session
    }
}

impl Default for ApiProbe {
    fn default() -> Self {
        Self::new()
    }
}
