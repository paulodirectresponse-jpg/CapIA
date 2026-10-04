//! Projeto aberto: `Engine` + arquivo `.capia` (store como journal). É a fachada que adaptadores
//! (CLI hoje; Tauri/REST/MCP depois) usam — nenhum deles fala com o SQLite nem escreve no
//! documento por fora do Command Engine.

use crate::pipeline::Pipeline;
use capia_commands::{
    Actor, CommandEnvelope, CommandError, CommitResult, Engine, EngineConfig, PreviewResult,
    Transaction,
};
use capia_model::Document;
use capia_store::{
    Catalog, CatalogOp, JobStore, PendingCatalog, ProjectInfo, ProjectStore, StoreError,
    StoreOptions, ValidationReport, random_plan_key,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Um projeto `.capia` aberto. Toda mudança passa pelo engine e é gravada **antes** de publicada.
#[derive(Debug)]
pub struct Project {
    engine: Engine,
    path: PathBuf,
    /// Catálogo de mídia (conexão própria; ADR-048).
    catalog: Catalog,
    /// Fila de efeitos do catálogo que o journal aplica na transação do próximo commit.
    pending: PendingCatalog,
    /// Persistência de jobs/tickets (conexão própria; schema 3).
    jobs: Arc<JobStore>,
    /// Executor de jobs de mídia, quando iniciado (`start_pipeline`).
    pub(crate) pipeline: Option<Pipeline>,
    /// Último grafo de render compilado, válido enquanto a revisão do documento não muda
    /// (o preview pede um quadro por movimento; recompilar milhares de clips a cada quadro custava
    /// dezenas de ms e disputava a sessão com os comandos).
    pub(crate) graph_cache: std::sync::Mutex<Option<GraphCacheEntry>>,
}

/// Grafo compilado para `(revisão do documento, sequence)`.
pub(crate) type GraphCacheEntry = (u64, capia_model::SequenceId, Arc<capia_render::RenderGraph>);

impl Project {
    /// Cria um projeto vazio (falha se o arquivo já existe).
    pub fn create(path: &Path, opts: &StoreOptions) -> Result<Self, StoreError> {
        let (store, state) = ProjectStore::create(path, opts)?;
        Self::assemble(path, opts, store, state)
    }

    /// Abre (e migra, se preciso) um projeto existente.
    pub fn open(path: &Path, opts: &StoreOptions) -> Result<Self, StoreError> {
        let (store, state) = ProjectStore::open(path, opts)?;
        Self::assemble(path, opts, store, state)
    }

    fn assemble(
        path: &Path,
        opts: &StoreOptions,
        store: ProjectStore,
        state: capia_commands::EngineState,
    ) -> Result<Self, StoreError> {
        let pending = store.pending_catalog();
        let engine = store.into_engine(state, random_plan_key()?, EngineConfig::default())?;
        let catalog = Catalog::open(path, opts.busy_timeout)?;
        let jobs = Arc::new(JobStore::open(path, opts.busy_timeout)?);
        Ok(Self {
            engine,
            path: path.to_path_buf(),
            catalog,
            pending,
            jobs,
            pipeline: None,
            graph_cache: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub(crate) fn catalog_mut(&mut self) -> &mut Catalog {
        &mut self.catalog
    }

    pub(crate) fn job_store(&self) -> &Arc<JobStore> {
        &self.jobs
    }

    pub(crate) fn queue_catalog(&self, op: CatalogOp) -> Result<(), StoreError> {
        self.pending
            .lock()
            .map(|mut q| q.push(op))
            .map_err(|_| StoreError::corrupted("catalog queue is poisoned"))
    }

    pub(crate) fn clear_catalog_queue(&self) {
        if let Ok(mut q) = self.pending.lock() {
            q.clear();
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn document(&self) -> &Document {
        self.engine.document()
    }

    /// Escrita direta (atores `User`/`System`). `Agent`/`Api` recebem `PREVIEW_REQUIRED`.
    pub fn execute(
        &mut self,
        actor: &Actor,
        tx: Transaction,
    ) -> Result<CommitResult, CommandError> {
        self.engine.execute(actor, tx, now_ms())
    }

    pub fn preview(
        &mut self,
        actor: &Actor,
        tx: Transaction,
    ) -> Result<PreviewResult, CommandError> {
        self.engine.preview(actor, tx, now_ms())
    }

    pub fn apply_plan(&mut self, actor: &Actor, token: &str) -> Result<CommitResult, CommandError> {
        self.engine.apply_plan(actor, token, now_ms())
    }

    pub fn undo(&mut self, actor: &Actor) -> Result<CommitResult, CommandError> {
        self.engine.undo(actor, now_ms())
    }

    pub fn redo(&mut self, actor: &Actor) -> Result<CommitResult, CommandError> {
        self.engine.redo(actor, now_ms())
    }

    /// Resumo do arquivo (somente leitura, não modifica nada).
    pub fn inspect(path: &Path) -> Result<ProjectInfo, StoreError> {
        ProjectStore::inspect(path)
    }

    /// Validação completa e não destrutiva do arquivo.
    pub fn validate(path: &Path) -> ValidationReport {
        ProjectStore::validate_file(path)
    }
}

/// Erro de interpretação de um comando/transação em JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for ParseError {}

/// Interpreta JSON de entrada de um cliente: uma [`Transaction`] completa, uma lista de
/// [`CommandEnvelope`] ou um único envelope. Os tipos são os do engine (sem parser paralelo).
pub fn parse_transaction(json: &str, default_label: &str) -> Result<Transaction, ParseError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ParseError(format!("invalid JSON: {e}")))?;
    let wrap = |commands: Vec<CommandEnvelope>| Transaction {
        transaction_id: None,
        label: default_label.to_owned(),
        base_revision: None,
        commands,
        max_ops: None,
    };
    let bad = |what: &str, e: serde_json::Error| ParseError(format!("invalid {what}: {e}"));
    match &value {
        serde_json::Value::Array(_) => serde_json::from_value(value)
            .map(wrap)
            .map_err(|e| bad("command list", e)),
        serde_json::Value::Object(o) if o.contains_key("commands") => {
            serde_json::from_value(value).map_err(|e| bad("transaction", e))
        }
        serde_json::Value::Object(_) => serde_json::from_value::<CommandEnvelope>(value)
            .map(|c| wrap(vec![c]))
            .map_err(|e| bad("command", e)),
        _ => Err(ParseError(
            "expected a transaction, a command or a list of commands".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const ENV: &str =
        r#"{"operation_id":"a","type":"create_sequence","name":"S","frame_rate":"30"}"#;

    #[test]
    fn accepts_a_transaction_a_list_and_a_single_command() {
        let single = parse_transaction(ENV, "cli").unwrap();
        assert_eq!((single.commands.len(), single.label.as_str()), (1, "cli"));
        let list = parse_transaction(&format!("[{ENV},{ENV}]"), "cli").unwrap();
        assert_eq!(list.commands.len(), 2);
        let full =
            parse_transaction(&format!(r#"{{"label":"mine","commands":[{ENV}]}}"#), "cli").unwrap();
        assert_eq!(full.label, "mine");
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for bad in [
            "",
            "{",
            "null",
            "42",
            "\"x\"",
            "{}",
            r#"{"commands":[{"type":"nope"}]}"#,
            "[1,2]",
        ] {
            assert!(parse_transaction(bad, "cli").is_err(), "{bad}");
        }
    }
}
