//! Fronteira com o Engine API. A inteligência é **cliente**: lê por métodos de leitura e escreve
//! **só** por `preview → apply_plan` com ator `Agent` (ADR-029/030). Não existe caminho de escrita
//! direta nesta interface.

use crate::error::{IntelError, IntelResult};
use capia_commands::Actor;
use capia_editor_api::{Reply, Session};
use capia_media::MediaToolchain;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Métodos de **leitura** que uma IA pode pedir (lista fechada; nada que escreva).
pub const READ_METHODS: &[&str] = &[
    "engine.info",
    "project.snapshot",
    "sequence.get",
    "assets.list",
    "history.list",
];

pub trait Engine: Send + Sync + core::fmt::Debug {
    /// Leitura (JSON). Métodos fora de [`READ_METHODS`] são recusados.
    fn read(&self, method: &str, params: Value) -> IntelResult<Value>;
    /// Fase 1 do gate de escrita (nada é gravado).
    fn preview(&self, actor: &Actor, label: &str, commands: Value) -> IntelResult<Value>;
    /// Fase 2: aplica só o plano revisado.
    fn apply(&self, actor: &Actor, token: &str) -> IntelResult<Value>;
    fn project_path(&self) -> Option<PathBuf>;
    fn toolchain(&self) -> Option<MediaToolchain>;
    /// Revisão atual do documento (para detectar *drift* durante uma Run).
    fn revision(&self) -> IntelResult<u64>;
    /// Início do import de um arquivo **já em staging** (só o Asset Gateway chama; ADR-087).
    fn import_begin(&self, path: &std::path::Path) -> IntelResult<String>;
    /// Estado do ticket de import (finaliza o que estiver pronto).
    fn import_poll(&self, ticket_id: &str) -> IntelResult<Value>;
    fn import_cancel(&self, ticket_id: &str) -> IntelResult<bool>;
}

/// Adaptador sobre a [`Session`] do editor (a mesma que a UI usa — um único documento).
#[derive(Clone, Debug)]
pub struct SessionEngine {
    session: Arc<Mutex<Session>>,
}

impl SessionEngine {
    pub fn new(session: Arc<Mutex<Session>>) -> Self {
        Self { session }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn api_err(e: capia_editor_api::ApiError) -> IntelError {
    let v = e.to_json();
    IntelError::new(
        v.get("code")
            .and_then(Value::as_str)
            .unwrap_or("ENGINE_ERROR"),
        v.get("message")
            .and_then(Value::as_str)
            .unwrap_or("engine error"),
    )
}

impl Engine for SessionEngine {
    fn read(&self, method: &str, params: Value) -> IntelResult<Value> {
        if !READ_METHODS.contains(&method) {
            return Err(IntelError::new(
                "NOT_ALLOWED",
                format!("`{method}` is not a read method available to the AI"),
            ));
        }
        match self.lock().call(method, params).map_err(api_err)? {
            Reply::Json(v) => Ok(v),
            Reply::Binary { .. } => Err(IntelError::new("ENGINE_ERROR", "unexpected binary reply")),
        }
    }

    fn preview(&self, actor: &Actor, label: &str, commands: Value) -> IntelResult<Value> {
        self.lock()
            .agent_preview(actor, label, commands)
            .map_err(api_err)
    }

    fn apply(&self, actor: &Actor, token: &str) -> IntelResult<Value> {
        self.lock().agent_apply(actor, token).map_err(api_err)
    }

    fn project_path(&self) -> Option<PathBuf> {
        self.lock().project_path()
    }

    fn toolchain(&self) -> Option<MediaToolchain> {
        self.lock().media_toolchain().cloned()
    }

    fn revision(&self) -> IntelResult<u64> {
        let doc_rev = self
            .lock()
            .document()
            .map(|d| d.revision)
            .map_err(api_err)?;
        Ok(doc_rev)
    }

    fn import_begin(&self, path: &std::path::Path) -> IntelResult<String> {
        self.lock().agent_import_begin(path).map_err(api_err)
    }

    fn import_poll(&self, ticket_id: &str) -> IntelResult<Value> {
        self.lock().agent_import_poll(ticket_id).map_err(api_err)
    }

    fn import_cancel(&self, ticket_id: &str) -> IntelResult<bool> {
        self.lock().agent_import_cancel(ticket_id).map_err(api_err)
    }
}
