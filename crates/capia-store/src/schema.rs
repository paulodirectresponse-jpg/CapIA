//! Schema do `.capia` e migrations (ADR-042). Forward-only, numeradas, cada uma numa transação;
//! `PRAGMA user_version` + `schema_migrations` registram a versão; `application_id` identifica o
//! formato. **Nunca** `CREATE TABLE IF NOT EXISTS` como substituto de migration.

use crate::error::{StoreError, StoreErrorCode, StoreResult};
use crate::failpoints::fp;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};

/// Versão de schema que este software escreve e entende.
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

/// `PRAGMA application_id` de todo `.capia` ("CAPI").
pub const APPLICATION_ID: i64 = 0x4341_5049;

/// Uma migration: leva o arquivo de `version - 1` para `version`.
#[derive(Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub up: fn(&Transaction<'_>) -> rusqlite::Result<()>,
}

impl core::fmt::Debug for Migration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Migration")
            .field("version", &self.version)
            .field("name", &self.name)
            .finish()
    }
}

/// Migrations conhecidas por este software (contíguas a partir de 1).
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial schema",
        up: m001_initial,
    },
    Migration {
        version: 2,
        name: "media catalog (assets, events)",
        up: m002_media_catalog,
    },
    Migration {
        version: 3,
        name: "media jobs (jobs, import_tickets) + asset fingerprint",
        up: m003_media_jobs,
    },
];

/// Schema 3 (ADR-052/053): fila de jobs de mídia persistida (estado/progresso/resultado/erro, para
/// sobreviver a fechar/crash: `running` vira `interrupted` ao reabrir) e tickets de import
/// assíncrono. Aditiva: projetos v1/v2 ganham tabelas vazias e uma coluna anulável; nada existente
/// é reescrito. Nada daqui entra no documento nem no undo.
const MEDIA_JOBS_SQL: &str = "
ALTER TABLE media_assets ADD COLUMN fingerprint TEXT;

CREATE TABLE jobs (
    job_id          TEXT    PRIMARY KEY NOT NULL CHECK (length(job_id) BETWEEN 1 AND 128),
    kind            TEXT    NOT NULL CHECK (length(kind) BETWEEN 1 AND 64),
    priority        TEXT    NOT NULL CHECK (priority IN ('interactive', 'normal', 'background')),
    state           TEXT    NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed', 'cancelled', 'interrupted')),
    progress_done   INTEGER NOT NULL CHECK (progress_done >= 0),
    progress_total  INTEGER NOT NULL CHECK (progress_total >= 0),
    dedup_key       TEXT,
    label           TEXT    NOT NULL,
    params_json     TEXT    NOT NULL CHECK (json_valid(params_json)),
    result_json     TEXT    CHECK (result_json IS NULL OR json_valid(result_json)),
    error_json      TEXT    CHECK (error_json IS NULL OR json_valid(error_json)),
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    created_ms      INTEGER NOT NULL CHECK (created_ms >= 0),
    started_ms      INTEGER CHECK (started_ms IS NULL OR started_ms >= 0),
    finished_ms     INTEGER CHECK (finished_ms IS NULL OR finished_ms >= 0),
    updated_ms      INTEGER NOT NULL CHECK (updated_ms >= 0)
) STRICT;
CREATE INDEX jobs_state ON jobs(state);

CREATE TABLE import_tickets (
    ticket_id    TEXT    PRIMARY KEY NOT NULL CHECK (length(ticket_id) BETWEEN 1 AND 128),
    path         TEXT    NOT NULL,
    size_bytes   INTEGER NOT NULL CHECK (size_bytes >= 0),
    fingerprint  TEXT,
    state        TEXT    NOT NULL CHECK (state IN ('pending', 'finalized', 'failed', 'cancelled', 'interrupted')),
    job_id       TEXT,
    asset_id     TEXT,
    outcome_json TEXT    CHECK (outcome_json IS NULL OR json_valid(outcome_json)),
    error_json   TEXT    CHECK (error_json IS NULL OR json_valid(error_json)),
    created_ms   INTEGER NOT NULL CHECK (created_ms >= 0),
    updated_ms   INTEGER NOT NULL CHECK (updated_ms >= 0)
) STRICT;
CREATE INDEX import_tickets_state ON import_tickets(state);
";

fn m003_media_jobs(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(MEDIA_JOBS_SQL)
}

/// Schema 2 (ADR-048): catálogo de mídia. Aditiva: projetos v1 ganham tabelas vazias; nenhuma linha
/// existente é tocada. O arquivo de mídia nunca entra no `.capia`.
const MEDIA_CATALOG_SQL: &str = "
CREATE TABLE media_assets (
    asset_id          TEXT    PRIMARY KEY NOT NULL CHECK (length(asset_id) BETWEEN 1 AND 128),
    kind              TEXT    NOT NULL CHECK (kind IN ('video', 'audio', 'image')),
    content_hash      TEXT    NOT NULL CHECK (length(content_hash) = 71 AND substr(content_hash, 1, 7) = 'sha256:'),
    size_bytes        INTEGER NOT NULL CHECK (size_bytes >= 0),
    display_name      TEXT    NOT NULL,
    location_json     TEXT    NOT NULL CHECK (json_valid(location_json)),
    known_paths_json  TEXT    NOT NULL CHECK (json_valid(known_paths_json)),
    media_info_json   TEXT    NOT NULL CHECK (json_valid(media_info_json)),
    status            TEXT    NOT NULL CHECK (status IN ('online', 'offline', 'modified')),
    status_checked_ms INTEGER NOT NULL CHECK (status_checked_ms >= 0),
    imported_ms       INTEGER NOT NULL CHECK (imported_ms >= 0)
) STRICT;
-- Deduplicação no próprio banco: um asset por conteúdo (ADR-046 §4).
CREATE UNIQUE INDEX media_assets_content_hash ON media_assets(content_hash);

CREATE TABLE asset_events (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    asset_id    TEXT    NOT NULL REFERENCES media_assets(asset_id),
    kind        TEXT    NOT NULL CHECK (kind IN ('import', 'reimport', 'alias', 'relink', 'force_relink', 'verify')),
    detail_json TEXT    NOT NULL CHECK (json_valid(detail_json)),
    at_ms       INTEGER NOT NULL CHECK (at_ms >= 0)
) STRICT;
CREATE INDEX asset_events_asset ON asset_events(asset_id);
CREATE TRIGGER asset_events_no_update BEFORE UPDATE ON asset_events BEGIN SELECT RAISE(ABORT, 'asset_events is append-only'); END;
CREATE TRIGGER asset_events_no_delete BEFORE DELETE ON asset_events BEGIN SELECT RAISE(ABORT, 'asset_events is append-only'); END;
";

fn m002_media_catalog(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(MEDIA_CATALOG_SQL)
}

const INITIAL_SQL: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE schema_migrations (
    version       INTEGER PRIMARY KEY NOT NULL,
    name          TEXT    NOT NULL,
    applied_at_ms INTEGER NOT NULL
) STRICT;

-- Cada transação confirmada do engine, completa (ops + inversas): base do undo/redo durável.
CREATE TABLE history_entries (
    id              INTEGER PRIMARY KEY NOT NULL,
    revision_before INTEGER NOT NULL,
    revision_after  INTEGER NOT NULL,
    label           TEXT    NOT NULL,
    actor_json      TEXT    NOT NULL CHECK (json_valid(actor_json)),
    transaction_id  TEXT,
    plan_id         TEXT,
    timestamp_ms    INTEGER NOT NULL,
    affected_json   TEXT    NOT NULL CHECK (json_valid(affected_json)),
    entry_json      TEXT    NOT NULL CHECK (json_valid(entry_json)),
    -- SHA-256 de entry_json: o SQLite não tem checksum por página; isto detecta corrupção silenciosa
    entry_sha256    TEXT    NOT NULL,
    CHECK (revision_after = revision_before + 1)
) STRICT;

-- Linha do tempo de mudanças de estado: commit / undo / redo (a pilha é a dobra desta tabela).
CREATE TABLE events (
    seq            INTEGER PRIMARY KEY NOT NULL,
    kind           TEXT    NOT NULL CHECK (kind IN ('commit', 'undo', 'redo')),
    entry_id       INTEGER NOT NULL REFERENCES history_entries(id),
    revision_after INTEGER NOT NULL UNIQUE,
    timestamp_ms   INTEGER NOT NULL,
    actor_json     TEXT    NOT NULL CHECK (json_valid(actor_json))
) STRICT;

-- Idempotência durável (ADR-029): gravada na MESMA transação do commit.
CREATE TABLE applied_operations (
    operation_id  TEXT    PRIMARY KEY NOT NULL CHECK (length(operation_id) BETWEEN 1 AND 128),
    payload_hash  TEXT    NOT NULL,
    entry_id      INTEGER NOT NULL REFERENCES history_entries(id),
    applied_at_ms INTEGER NOT NULL,
    actor_json    TEXT    NOT NULL CHECK (json_valid(actor_json))
) STRICT;
CREATE INDEX applied_operations_entry ON applied_operations(entry_id);

CREATE TABLE commit_results (
    entry_id    INTEGER PRIMARY KEY NOT NULL REFERENCES history_entries(id),
    result_json TEXT    NOT NULL CHECK (json_valid(result_json)),
    result_sha256 TEXT  NOT NULL
) STRICT;

-- Último estado materializado do documento; o resto é replay de `events` posteriores.
CREATE TABLE snapshots (
    seq              INTEGER PRIMARY KEY NOT NULL,
    revision         INTEGER NOT NULL,
    cursor           INTEGER NOT NULL,
    history_ids_json TEXT    NOT NULL CHECK (json_valid(history_ids_json)),
    document_json    TEXT    NOT NULL CHECK (json_valid(document_json)),
    digest           TEXT    NOT NULL
) STRICT;

-- Tabelas de auditoria são append-only: nenhuma UPDATE/DELETE passa (defesa contra bug e adulteração casual).
CREATE TRIGGER history_entries_no_update BEFORE UPDATE ON history_entries BEGIN SELECT RAISE(ABORT, 'history_entries is append-only'); END;
CREATE TRIGGER history_entries_no_delete BEFORE DELETE ON history_entries BEGIN SELECT RAISE(ABORT, 'history_entries is append-only'); END;
CREATE TRIGGER events_no_update BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT, 'events is append-only'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT, 'events is append-only'); END;
CREATE TRIGGER applied_operations_no_update BEFORE UPDATE ON applied_operations BEGIN SELECT RAISE(ABORT, 'applied_operations is append-only'); END;
CREATE TRIGGER applied_operations_no_delete BEFORE DELETE ON applied_operations BEGIN SELECT RAISE(ABORT, 'applied_operations is append-only'); END;
CREATE TRIGGER commit_results_no_update BEFORE UPDATE ON commit_results BEGIN SELECT RAISE(ABORT, 'commit_results is append-only'); END;
CREATE TRIGGER commit_results_no_delete BEFORE DELETE ON commit_results BEGIN SELECT RAISE(ABORT, 'commit_results is append-only'); END;
";

fn m001_initial(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(INITIAL_SQL)?;
    // a assinatura do formato nasce na MESMA transação do schema: nunca existe arquivo "meio criado"
    tx.pragma_update(None, "application_id", APPLICATION_ID)
}

/// Assinatura lida **sem** modificar o arquivo (conexão somente leitura).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Peek {
    pub application_id: i64,
    pub user_version: u32,
}

pub(crate) fn peek(path: &Path) -> StoreResult<Peek> {
    use std::io::Read as _;
    if !path.exists() {
        return Err(StoreError::new(
            StoreErrorCode::ProjectNotFound,
            format!("project file {} does not exist", path.display()),
        ));
    }
    if path.is_dir() {
        return Err(StoreError::new(
            StoreErrorCode::NotACapiaProject,
            "the path is a directory",
        ));
    }
    // Lê o cabeçalho de 100 bytes direto do arquivo: nenhuma conexão SQLite, logo nenhum efeito
    // colateral (nem `-wal`/`-shm`) ao examinar um arquivo que talvez nem seja nosso.
    let mut header = [0u8; 100];
    let mut file = std::fs::File::open(path)?;
    let mut read = 0;
    while read < header.len() {
        match file.read(&mut header[read..])? {
            0 => break,
            n => read += n,
        }
    }
    if read == 0 {
        return Err(StoreError::new(
            StoreErrorCode::NotACapiaProject,
            "the file is empty",
        ));
    }
    if read < header.len() || &header[..16] != b"SQLite format 3\0" {
        return Err(StoreError::new(
            StoreErrorCode::NotACapiaProject,
            "the file is not a SQLite database",
        ));
    }
    let be32 = |at: usize| {
        u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Ok(Peek {
        // application_id em 68..72; user_version em 60..64 (big-endian, ver formato de arquivo do SQLite)
        application_id: i64::from(i32::from_be_bytes([
            header[68], header[69], header[70], header[71],
        ])),
        user_version: be32(60),
    })
}

/// Releitura **autêntica** da assinatura pela conexão (já com o WAL recuperado): o cabeçalho do
/// arquivo principal pode estar defasado se houver páginas só no `-wal` após uma queda.
pub(crate) fn peek_connection(conn: &Connection) -> StoreResult<Peek> {
    let application_id: i64 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let user_version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok(Peek {
        application_id,
        user_version: u32::try_from(user_version).unwrap_or(u32::MAX),
    })
}

/// Valida a assinatura e a versão **antes de qualquer escrita**.
pub(crate) fn check_signature(p: Peek, supported: u32) -> StoreResult<()> {
    if p.application_id != APPLICATION_ID {
        return Err(StoreError::new(
            StoreErrorCode::NotACapiaProject,
            "the file is a SQLite database but not a CapIA project",
        )
        .with_details(serde_json::json!({ "application_id": p.application_id })));
    }
    if p.user_version == 0 {
        return Err(StoreError::corrupted(
            "CapIA project without a schema version",
        ));
    }
    if p.user_version > supported {
        return Err(StoreError::new(
            StoreErrorCode::UnsupportedSchemaVersion,
            format!(
                "the project uses schema {} but this version of CapIA understands up to {supported}; the file was not modified",
                p.user_version
            ),
        )
        .with_details(serde_json::json!({ "found": p.user_version, "supported": supported })));
    }
    Ok(())
}

fn backup_path(path: &Path, from: u32, now_ms: i64) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".v{from}.bak"));
    let mut candidate = path.with_file_name(&name);
    if candidate.exists() {
        name.push(format!("-{now_ms}"));
        candidate = path.with_file_name(name);
    }
    candidate
}

/// Aplica as migrations `from+1 ..= target`, cada uma numa transação `IMMEDIATE`. Antes de migrar
/// um arquivo existente cria um backup consistente (`VACUUM INTO`). Falha ⇒ `MIGRATION_FAILED`,
/// o arquivo permanece na última versão concluída. Devolve as versões aplicadas.
pub(crate) fn run_migrations(
    conn: &mut Connection,
    path: Option<&Path>,
    migrations: &[Migration],
    from: u32,
    target: u32,
    now_ms: i64,
) -> StoreResult<Vec<u32>> {
    let mut expected = from + 1;
    let pending: Vec<&Migration> = migrations
        .iter()
        .filter(|m| m.version > from && m.version <= target)
        .collect();
    for m in &pending {
        if m.version != expected {
            return Err(StoreError::new(
                StoreErrorCode::MigrationFailed,
                format!(
                    "migration list has a gap: expected {expected}, found {}",
                    m.version
                ),
            ));
        }
        expected += 1;
    }
    if expected != target + 1 {
        return Err(StoreError::new(
            StoreErrorCode::MigrationFailed,
            format!("no migration path from schema {from} to {target}"),
        ));
    }
    if from > 0
        && !pending.is_empty()
        && let Some(path) = path
    {
        let backup = backup_path(path, from, now_ms);
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])
            .map_err(|e| {
                StoreError::new(
                    StoreErrorCode::MigrationFailed,
                    "could not create the pre-migration backup",
                )
                .with_cause(e)
            })?;
    }
    let mut applied = Vec::new();
    for m in pending {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let step = (|| -> rusqlite::Result<()> {
            (m.up)(&tx)?;
            // queda NO MEIO da migration (DDL aplicado, nada commitado): o arquivo tem de continuar na versão anterior
            fp!("in_migration");
            tx.execute(
                "INSERT INTO schema_migrations(version, name, applied_at_ms) VALUES (?1, ?2, ?3)",
                rusqlite::params![m.version, m.name, now_ms],
            )?;
            tx.pragma_update(None, "user_version", m.version)
        })();
        match step {
            Ok(()) => tx.commit().map_err(|e| {
                StoreError::new(
                    StoreErrorCode::MigrationFailed,
                    format!("migration {} failed to commit", m.version),
                )
                .with_cause(e)
            })?,
            Err(e) => {
                drop(tx); // rollback
                return Err(StoreError::new(
                    StoreErrorCode::MigrationFailed,
                    format!(
                        "migration {} ({}) failed; the project stays at schema {}",
                        m.version,
                        m.name,
                        m.version - 1
                    ),
                )
                .with_cause(e)
                .with_details(
                    serde_json::json!({ "failed_version": m.version, "applied": applied }),
                ));
            }
        }
        applied.push(m.version);
    }
    Ok(applied)
}
