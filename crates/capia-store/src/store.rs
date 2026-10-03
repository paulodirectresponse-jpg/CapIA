//! `ProjectStore`: o arquivo `.capia` aberto. Sem lógica de edição — só persiste o que o engine
//! entrega (journal atômico), carrega o estado e valida o arquivo.

use crate::error::{StoreError, StoreErrorCode, StoreResult};
use crate::failpoints::fp;
use crate::load::{self, Loaded, Mirror, StoreStats, kind_str};
use crate::schema::{self, CURRENT_SCHEMA_VERSION, MIGRATIONS, Migration};
use capia_commands::journal::{Journal, JournalError, JournalRecord};
use capia_commands::{
    Actor, AppliedOperation, AuditKind, CommitResult, Engine, EngineConfig, EngineState,
};
use capia_model::{Document, validate_document};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `PRAGMA synchronous`. Padrão **Full**: commit reportado = sobrevive a queda de energia
/// (necessário para idempotência durável — ADR-044). `Normal`/`Off` só para testes de volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Synchronous {
    Full,
    Normal,
    Off,
}

#[derive(Clone, Debug)]
pub struct StoreOptions {
    pub synchronous: Synchronous,
    /// Quanto esperar pelo lock de escrita antes de devolver `STORE_BUSY`.
    pub busy_timeout: Duration,
    /// Um snapshot novo a cada N eventos (na mesma transação do N-ésimo commit).
    pub snapshot_every: u64,
    /// Teto de um blob JSON lido do arquivo (defesa contra arquivo hostil; padrão 512 MiB).
    pub max_blob_bytes: i64,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            synchronous: Synchronous::Full,
            busy_timeout: Duration::from_millis(5_000),
            snapshot_every: 256,
            max_blob_bytes: load::DEFAULT_MAX_BLOB_BYTES,
        }
    }
}

/// Chave aleatória de 256 bits para o token de plano do engine (só em memória, ADR-030).
pub fn random_plan_key() -> StoreResult<[u8; 32]> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| {
        StoreError::new(StoreErrorCode::StoreIoError, "no source of randomness").with_cause(e)
    })?;
    Ok(key)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn random_hex(bytes: usize) -> StoreResult<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| {
        StoreError::new(StoreErrorCode::StoreIoError, "no source of randomness").with_cause(e)
    })?;
    Ok(capia_commands::hash::hex(&buf))
}

fn configure(conn: &Connection, opts: &StoreOptions) -> StoreResult<()> {
    conn.busy_timeout(opts.busy_timeout)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::new(
            StoreErrorCode::StoreIoError,
            format!("the file system does not support WAL (journal_mode = {mode})"),
        ));
    }
    conn.pragma_update(
        None,
        "synchronous",
        match opts.synchronous {
            Synchronous::Full => "FULL",
            Synchronous::Normal => "NORMAL",
            Synchronous::Off => "OFF",
        },
    )?;
    conn.pragma_update(None, "trusted_schema", false)?;
    conn.pragma_update(None, "cell_size_check", true)?;
    Ok(())
}

/// Resumo de uma sequence (para `inspect`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceInfo {
    pub id: String,
    pub name: String,
    pub frame_rate: String,
    pub tracks: usize,
    pub clips: usize,
    pub duration_ticks: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub path: String,
    pub project_id: String,
    pub schema_version: u32,
    pub supported_schema_version: u32,
    pub needs_migration: bool,
    pub created_at_ms: i64,
    pub revision: u64,
    pub digest: String,
    pub sequences: Vec<SequenceInfo>,
    pub total_tracks: usize,
    pub total_clips: usize,
    pub assets: usize,
    pub stats: StoreStats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub issues: Vec<StoreError>,
    pub info: Option<ProjectInfo>,
}

/// Registro de uma operação já aplicada, como gravado (para ferramentas e testes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredOperation {
    pub operation_id: String,
    pub applied: AppliedOperation,
    pub result: CommitResult,
}

pub struct ProjectStore {
    conn: Connection,
    path: PathBuf,
    opts: StoreOptions,
    mirror: Mirror,
    project_id: String,
    schema_version: u32,
}

impl core::fmt::Debug for ProjectStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProjectStore")
            .field("path", &self.path)
            .field("project_id", &self.project_id)
            .field("schema_version", &self.schema_version)
            .field("head_revision", &self.mirror.head_revision)
            .finish_non_exhaustive()
    }
}

fn write_snapshot(tx: &Transaction<'_>, seq: i64, doc: &Document, m: &Mirror) -> StoreResult<()> {
    tx.execute("DELETE FROM snapshots WHERE seq < ?1", [seq])?;
    tx.execute(
        "INSERT INTO snapshots(seq, revision, cursor, history_ids_json, document_json, digest) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            seq,
            i64::try_from(doc.revision).map_err(|_| StoreError::corrupted("revision out of range"))?,
            i64::try_from(m.cursor).map_err(|_| StoreError::corrupted("cursor out of range"))?,
            serde_json::to_string(&m.history_ids).map_err(|e| StoreError::new(StoreErrorCode::TransactionFailed, "serialize history ids").with_cause(e))?,
            load::document_text(doc),
            load::digest_of(doc),
        ],
    )?;
    Ok(())
}

fn read_head(tx: &Transaction<'_>) -> StoreResult<(i64, u64)> {
    let last: Option<(i64, i64)> = tx
        .query_row(
            "SELECT seq, revision_after FROM events ORDER BY seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (seq, rev) = match last {
        Some(v) => v,
        None => tx.query_row(
            "SELECT seq, revision FROM snapshots ORDER BY seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?,
    };
    Ok((
        seq,
        u64::try_from(rev).map_err(|_| StoreError::corrupted("negative revision"))?,
    ))
}

fn ser<T: Serialize>(what: &str, v: &T) -> StoreResult<String> {
    serde_json::to_string(v).map_err(|e| {
        StoreError::new(
            StoreErrorCode::TransactionFailed,
            format!("could not serialize {what}"),
        )
        .with_cause(e)
    })
}

fn to_i64(v: u64) -> StoreResult<i64> {
    i64::try_from(v).map_err(|_| {
        StoreError::new(
            StoreErrorCode::TransactionFailed,
            "integer out of range for storage",
        )
    })
}

impl ProjectStore {
    /// Cria um projeto novo (documento vazio). Falha se `path` já existe. O arquivo só aparece no
    /// caminho final **completo** (construído num temporário e ligado atomicamente).
    pub fn create(path: &Path, opts: &StoreOptions) -> StoreResult<(Self, EngineState)> {
        Self::create_with_document(path, &Document::new(), opts)
    }

    /// Como [`create`](Self::create), mas parte de um documento já existente (importação, testes
    /// de volume). O documento precisa respeitar todas as invariantes.
    pub fn create_with_document(
        path: &Path,
        doc: &Document,
        opts: &StoreOptions,
    ) -> StoreResult<(Self, EngineState)> {
        if path.exists() {
            return Err(StoreError::new(
                StoreErrorCode::ProjectAlreadyExists,
                format!("{} already exists", path.display()),
            ));
        }
        if let Some(v) = validate_document(doc).first() {
            return Err(StoreError::new(
                StoreErrorCode::InvalidArgument,
                format!("the initial document is invalid: {}", v.message),
            ));
        }
        let mut tmp_name = path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(format!(".creating-{}", random_hex(4)?));
        let tmp = path.with_file_name(tmp_name);
        let built = Self::build_new_file(&tmp, doc, opts);
        if let Err(e) = built {
            remove_with_sidecars(&tmp);
            return Err(e);
        }
        // ligação atômica sem sobrescrever: o destino nunca aparece pela metade
        let linked = std::fs::hard_link(&tmp, path).or_else(|_| {
            if path.exists() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "destination exists",
                ))
            } else {
                std::fs::rename(&tmp, path)
            }
        });
        remove_with_sidecars(&tmp);
        linked.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                StoreError::new(
                    StoreErrorCode::ProjectAlreadyExists,
                    format!("{} already exists", path.display()),
                )
                .with_cause(e)
            } else {
                StoreError::from(e)
            }
        })?;
        Self::open(path, opts)
    }

    fn build_new_file(tmp: &Path, doc: &Document, opts: &StoreOptions) -> StoreResult<()> {
        let mut conn = Connection::open_with_flags(
            tmp,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        configure(&conn, opts)?;
        let now = now_ms();
        schema::run_migrations(&mut conn, None, MIGRATIONS, 0, CURRENT_SCHEMA_VERSION, now)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (k, v) in [
            ("project_id", format!("prj_{}", random_hex(16)?)),
            ("created_at_ms", now.to_string()),
            (
                "created_by",
                format!("capia-store {}", env!("CARGO_PKG_VERSION")),
            ),
            ("document_schema_version", doc.schema_version.to_string()),
        ] {
            tx.execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)",
                params![k, v],
            )?;
        }
        let mirror = Mirror {
            head_seq: 0,
            head_revision: doc.revision,
            history_ids: Vec::new(),
            cursor: 0,
            events_since_snapshot: 0,
        };
        write_snapshot(&tx, 0, doc, &mirror)?;
        tx.commit()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        conn.close().map_err(|(_, e)| StoreError::from(e))?;
        Ok(())
    }

    /// Abre um projeto existente: confere assinatura e versão **antes de qualquer escrita**,
    /// migra se necessário, verifica a integridade rápida e reconstrói o estado do engine.
    pub fn open(path: &Path, opts: &StoreOptions) -> StoreResult<(Self, EngineState)> {
        Self::open_with_migrations(path, opts, MIGRATIONS, CURRENT_SCHEMA_VERSION)
    }

    /// `open` com lista de migrations explícita (testes de migração com versões sintéticas).
    #[doc(hidden)]
    pub fn open_with_migrations(
        path: &Path,
        opts: &StoreOptions,
        migrations: &[Migration],
        supported: u32,
    ) -> StoreResult<(Self, EngineState)> {
        let peeked = schema::peek(path)?;
        schema::check_signature(peeked, supported)?;
        let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        // O cabeçalho do arquivo principal pode estar defasado (páginas só no WAL após uma queda):
        // a assinatura e a versão valem pela leitura da conexão — ainda antes de qualquer escrita.
        let authoritative = schema::peek_connection(&conn)?;
        schema::check_signature(authoritative, supported)?;
        configure(&conn, opts)?;
        if authoritative.user_version < supported {
            schema::run_migrations(
                &mut conn,
                Some(path),
                migrations,
                authoritative.user_version,
                supported,
                now_ms(),
            )?;
        }
        quick_check(&conn)?;
        let project_id = read_meta(&conn, "project_id")?;
        let Loaded { state, mirror, .. } = load::load(&conn, opts.max_blob_bytes)?;
        let store = Self {
            conn,
            path: path.to_path_buf(),
            opts: opts.clone(),
            mirror,
            project_id,
            schema_version: supported,
        };
        Ok((store, state))
    }

    /// Abre o projeto já com um [`Engine`] ligado a este store (persistir antes de publicar).
    pub fn open_engine(
        path: &Path,
        opts: &StoreOptions,
        plan_key: [u8; 32],
        config: EngineConfig,
    ) -> StoreResult<Engine> {
        let (store, state) = Self::open(path, opts)?;
        store.into_engine(state, plan_key, config)
    }

    /// Cria o projeto já com um [`Engine`] ligado.
    pub fn create_engine(
        path: &Path,
        opts: &StoreOptions,
        plan_key: [u8; 32],
        config: EngineConfig,
    ) -> StoreResult<Engine> {
        let (store, state) = Self::create(path, opts)?;
        store.into_engine(state, plan_key, config)
    }

    /// Restaura o engine a partir do estado carregado e entrega este store como seu journal.
    pub fn into_engine(
        self,
        state: EngineState,
        plan_key: [u8; 32],
        config: EngineConfig,
    ) -> StoreResult<Engine> {
        let mut engine = Engine::restore(state, plan_key, config).map_err(|e| {
            StoreError::corrupted(format!("the stored state is inconsistent: {}", e.message))
        })?;
        engine.set_journal(Box::new(self));
        Ok(engine)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Recarrega o estado atual do **banco** (inclui o que outro escritor tenha gravado).
    pub fn load_state(&self) -> StoreResult<EngineState> {
        Ok(load::load(&self.conn, self.opts.max_blob_bytes)?.state)
    }

    pub fn stats(&self) -> StoreResult<StoreStats> {
        Ok(load::load(&self.conn, self.opts.max_blob_bytes)?.stats)
    }

    pub fn has_operation_id(&self, operation_id: &str) -> StoreResult<bool> {
        let hit: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM applied_operations WHERE operation_id = ?1",
                [operation_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(hit.is_some())
    }

    /// O registro durável de uma operação: quem aplicou, o hash do comando e o resultado original.
    pub fn load_operation_result(
        &self,
        operation_id: &str,
    ) -> StoreResult<Option<StoredOperation>> {
        let row: Option<(String, i64, i64, String, String, String)> = self
            .conn
            .query_row(
                "SELECT a.payload_hash, a.entry_id, a.applied_at_ms, a.actor_json, r.result_json, r.result_sha256 \
                 FROM applied_operations a JOIN commit_results r ON r.entry_id = a.entry_id WHERE a.operation_id = ?1",
                [operation_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        let Some((payload_hash, entry_id, applied_at, actor, result, result_sha)) = row else {
            return Ok(None);
        };
        if capia_commands::hash::sha256_hex(result.as_bytes()) != result_sha {
            return Err(StoreError::corrupted(format!(
                "the stored result of operation {operation_id} fails its checksum"
            )));
        }
        let bad = |what: &str, e: serde_json::Error| {
            StoreError::corrupted(format!("invalid {what} for operation {operation_id}"))
                .with_cause(e)
        };
        Ok(Some(StoredOperation {
            operation_id: operation_id.to_owned(),
            applied: AppliedOperation {
                payload_hash,
                history_entry_id: u64::try_from(entry_id)
                    .map_err(|_| StoreError::corrupted("negative entry id"))?,
                applied_at_ms: u64::try_from(applied_at)
                    .map_err(|_| StoreError::corrupted("negative timestamp"))?,
                actor: serde_json::from_str::<Actor>(&actor).map_err(|e| bad("actor", e))?,
            },
            result: serde_json::from_str::<CommitResult>(&result).map_err(|e| bad("result", e))?,
        }))
    }

    /// Fecha com checkpoint (`TRUNCATE`): sem `-wal`/`-shm` em repouso.
    pub fn close(self) -> StoreResult<()> {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        self.conn.close().map_err(|(_, e)| StoreError::from(e))
    }

    // ---- ferramentas somente leitura -------------------------------------------------------

    fn open_read_only(path: &Path) -> StoreResult<Connection> {
        // Preferimos uma conexão de leitura-escrita com `query_only`: ao fechar como última conexão
        // o SQLite limpa `-wal`/`-shm`, deixando o diretório como encontrou. Sem permissão de
        // escrita, caímos para somente leitura.
        let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
            Ok(c) => c,
            Err(_) => Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?,
        };
        conn.busy_timeout(Duration::from_millis(5_000))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "query_only", true)?;
        Ok(conn)
    }

    /// Resumo do projeto sem modificar o arquivo.
    pub fn inspect(path: &Path) -> StoreResult<ProjectInfo> {
        let peeked = schema::peek(path)?;
        schema::check_signature(peeked, CURRENT_SCHEMA_VERSION)?;
        let conn = Self::open_read_only(path)?;
        info_from(&conn, path, peeked.user_version)
    }

    /// Validação completa e **não destrutiva**: assinatura, `integrity_check`, `foreign_key_check`,
    /// carga do estado (snapshot + replay + digest) e invariantes do documento.
    pub fn validate_file(path: &Path) -> ValidationReport {
        let mut issues = Vec::new();
        let fail = |issues: Vec<StoreError>| ValidationReport {
            ok: false,
            issues,
            info: None,
        };
        let peeked = match schema::peek(path)
            .and_then(|p| schema::check_signature(p, CURRENT_SCHEMA_VERSION).map(|()| p))
        {
            Ok(p) => p,
            Err(e) => return fail(vec![e]),
        };
        let conn = match Self::open_read_only(path) {
            Ok(c) => c,
            Err(e) => return fail(vec![e]),
        };
        let push_rows =
            |sql: &str, what: &str, issues: &mut Vec<StoreError>| match conn.prepare(sql) {
                Ok(mut stmt) => {
                    let rows: Vec<String> = stmt
                        .query_map([], |r| r.get::<_, String>(0))
                        .map(|it| it.filter_map(Result::ok).collect())
                        .unwrap_or_default();
                    if !(rows.len() == 1 && rows[0] == "ok") && !rows.is_empty() {
                        for r in rows.into_iter().take(20) {
                            issues.push(StoreError::corrupted(format!("{what}: {r}")));
                        }
                    }
                }
                Err(e) => issues.push(StoreError::from(e)),
            };
        push_rows("PRAGMA integrity_check", "integrity_check", &mut issues);
        match conn.prepare("PRAGMA foreign_key_check").and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(0))
                .map(|it| it.filter_map(Result::ok).count())
        }) {
            Ok(0) => {}
            Ok(n) => issues.push(StoreError::corrupted(format!(
                "foreign_key_check: {n} violation(s)"
            ))),
            Err(e) => issues.push(StoreError::from(e)),
        }
        if !issues.is_empty() {
            return fail(issues);
        }
        match load::load(&conn, load::DEFAULT_MAX_BLOB_BYTES).and_then(|l| {
            // o engine tem a última palavra sobre consistência interna do estado
            Engine::restore(l.state.clone(), [0; 32], EngineConfig::default()).map_err(|e| {
                StoreError::corrupted(format!("inconsistent engine state: {}", e.message))
            })?;
            Ok(l)
        }) {
            Ok(_) => {}
            Err(e) => return fail(vec![e]),
        }
        match info_from(&conn, path, peeked.user_version) {
            Ok(info) => ValidationReport {
                ok: true,
                issues,
                info: Some(info),
            },
            Err(e) => fail(vec![e]),
        }
    }

    // ---- escrita (via Journal) --------------------------------------------------------------

    fn append_inner(
        &mut self,
        record: &JournalRecord<'_>,
        doc_after: &Document,
    ) -> StoreResult<()> {
        fp!("before_begin");
        let event = record.event();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // escritor obsoleto? o banco precisa estar exatamente onde este engine acha que está
        let (db_seq, db_revision) = read_head(&tx)?;
        if db_seq != self.mirror.head_seq
            || db_revision != self.mirror.head_revision
            || db_revision.checked_add(1) != Some(event.revision)
        {
            return Err(StoreError::new(
                StoreErrorCode::StoreConflict,
                "the project was changed by another writer since it was opened: reopen it and retry",
            )
            .with_details(serde_json::json!({
                "reason": "STALE_HEAD",
                "database_revision": db_revision,
                "engine_revision": self.mirror.head_revision,
                "event_revision": event.revision,
            })));
        }
        let seq = db_seq
            .checked_add(1)
            .ok_or_else(|| StoreError::corrupted("event sequence space exhausted"))?;
        let mut next = self.mirror.clone();
        if let JournalRecord::Commit {
            entry,
            result,
            applied,
            ..
        } = record
        {
            let entry_text = ser("history entry", entry)?;
            let entry_sha = capia_commands::hash::sha256_hex(entry_text.as_bytes());
            tx.execute(
                "INSERT INTO history_entries(id, revision_before, revision_after, label, actor_json, transaction_id, plan_id, timestamp_ms, affected_json, entry_json, entry_sha256) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    to_i64(entry.id)?,
                    to_i64(entry.revision_before)?,
                    to_i64(entry.revision_after)?,
                    entry.label,
                    ser("actor", &entry.actor)?,
                    entry.transaction_id,
                    entry.plan_id,
                    to_i64(entry.timestamp_ms)?,
                    ser("affected set", &entry.affected)?,
                    entry_text,
                    entry_sha,
                ],
            )?;
            fp!("in_tx_after_entry");
            let result_text = ser("commit result", result)?;
            let result_sha = capia_commands::hash::sha256_hex(result_text.as_bytes());
            tx.execute(
                "INSERT INTO commit_results(entry_id, result_json, result_sha256) VALUES (?1, ?2, ?3)",
                params![to_i64(entry.id)?, result_text, result_sha],
            )?;
            for (operation_id, op) in *applied {
                tx.execute(
                    "INSERT INTO applied_operations(operation_id, payload_hash, entry_id, applied_at_ms, actor_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![operation_id, op.payload_hash, to_i64(op.history_entry_id)?, to_i64(op.applied_at_ms)?, ser("actor", &op.actor)?],
                )?;
            }
            next.history_ids.truncate(next.cursor);
            next.history_ids.push(entry.id);
            next.cursor = next.history_ids.len();
        } else {
            match event.kind {
                AuditKind::Undo => {
                    if next.cursor == 0 || next.history_ids[next.cursor - 1] != event.entry_id {
                        return Err(StoreError::new(
                            StoreErrorCode::StoreConflict,
                            "undo does not match the stored history stack",
                        ));
                    }
                    next.cursor -= 1;
                }
                _ => {
                    if next.cursor >= next.history_ids.len()
                        || next.history_ids[next.cursor] != event.entry_id
                    {
                        return Err(StoreError::new(
                            StoreErrorCode::StoreConflict,
                            "redo does not match the stored history stack",
                        ));
                    }
                    next.cursor += 1;
                }
            }
        }
        tx.execute(
            "INSERT INTO events(seq, kind, entry_id, revision_after, timestamp_ms, actor_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![seq, kind_str(event.kind), to_i64(event.entry_id)?, to_i64(event.revision)?, to_i64(event.timestamp_ms)?, ser("actor", &event.actor)?],
        )?;
        next.head_seq = seq;
        next.head_revision = event.revision;
        next.events_since_snapshot += 1;
        if next.events_since_snapshot >= self.opts.snapshot_every.max(1) {
            write_snapshot(&tx, seq, doc_after, &next)?;
            next.events_since_snapshot = 0;
        }
        fp!("in_tx_after_writes");
        fp!("before_commit");
        tx.commit()?;
        fp!("after_commit");
        self.mirror = next;
        Ok(())
    }
}

impl Journal for ProjectStore {
    fn append(
        &mut self,
        record: &JournalRecord<'_>,
        doc_after: &Document,
    ) -> Result<(), JournalError> {
        self.append_inner(record, doc_after)
            .map_err(JournalError::from)
    }
}

fn quick_check(conn: &Connection) -> StoreResult<()> {
    let result: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StoreError::corrupted("quick_check failed").with_cause(result))
    }
}

fn read_meta(conn: &Connection, key: &str) -> StoreResult<String> {
    let v: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?;
    v.ok_or_else(|| StoreError::corrupted(format!("meta.{key} is missing")))
}

fn info_from(conn: &Connection, path: &Path, schema_version: u32) -> StoreResult<ProjectInfo> {
    let loaded = load::load(conn, load::DEFAULT_MAX_BLOB_BYTES)?;
    let doc = &loaded.state.doc;
    let sequences: Vec<SequenceInfo> = doc
        .sequences()
        .map(|(id, s)| SequenceInfo {
            id: id.to_string(),
            name: s.header.name.clone(),
            frame_rate: s.frame_rate().rate().to_string(),
            tracks: s.tracks().len(),
            clips: s.clip_count(),
            duration_ticks: s.duration().0,
        })
        .collect();
    Ok(ProjectInfo {
        path: path.display().to_string(),
        project_id: read_meta(conn, "project_id")?,
        schema_version,
        supported_schema_version: CURRENT_SCHEMA_VERSION,
        needs_migration: schema_version < CURRENT_SCHEMA_VERSION,
        created_at_ms: read_meta(conn, "created_at_ms")?.parse().unwrap_or(0),
        revision: doc.revision,
        digest: load::digest_of(doc),
        total_tracks: sequences.iter().map(|s| s.tracks).sum(),
        total_clips: sequences.iter().map(|s| s.clips).sum(),
        assets: doc.assets().count(),
        sequences,
        stats: loaded.stats,
    })
}

fn remove_with_sidecars(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(suffix);
        let _ = std::fs::remove_file(path.with_file_name(name));
    }
}
