//! Catálogo de mídia do projeto (ADR-046/048, schema 2): onde está cada arquivo, seu hash,
//! metadados normalizados e disponibilidade conhecida. **Não** é parte do documento nem do undo.
//!
//! Escritas vêm de dois caminhos:
//! * **dentro do commit do engine** — [`PendingCatalog`] é drenado por `ProjectStore::append_inner`
//!   na MESMA transação do `register_asset` (import atômico);
//! * **catálogo isolado** — [`Catalog`] (relink, verify, alias) numa transação própria.

use crate::error::{StoreError, StoreErrorCode, StoreResult};
use capia_assets::{AssetKind, AssetLocation, AssetRecord, Availability, ContentHash};
use capia_media::MediaInfo;
use capia_model::AssetId;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Teto de cada coluna JSON do catálogo ao ler (arquivo hostil).
const MAX_JSON: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEventKind {
    Import,
    Reimport,
    Alias,
    Relink,
    ForceRelink,
    Verify,
}

impl CatalogEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Reimport => "reimport",
            Self::Alias => "alias",
            Self::Relink => "relink",
            Self::ForceRelink => "force_relink",
            Self::Verify => "verify",
        }
    }
}

/// Escrita do catálogo: grava/atualiza o registro e anexa um evento de auditoria.
#[derive(Clone, Debug)]
pub struct CatalogOp {
    pub record: AssetRecord,
    pub event: CatalogEventKind,
    pub detail: Value,
    pub at_ms: u64,
}

/// Fila de efeitos laterais que o `Journal` aplica na transação do próximo commit.
pub type PendingCatalog = Arc<Mutex<Vec<CatalogOp>>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogEvent {
    pub seq: i64,
    pub asset_id: String,
    pub kind: String,
    pub detail: Value,
    pub at_ms: u64,
}

fn corrupted(msg: impl Into<String>) -> StoreError {
    StoreError::corrupted(msg)
}

fn ser<T: Serialize>(what: &str, v: &T) -> StoreResult<String> {
    serde_json::to_string(v).map_err(|e| {
        StoreError::new(
            StoreErrorCode::TransactionFailed,
            format!("cannot serialize {what}"),
        )
        .with_cause(e)
    })
}

fn to_i64(v: u64) -> StoreResult<i64> {
    i64::try_from(v).map_err(|_| {
        StoreError::new(
            StoreErrorCode::InvalidArgument,
            "value does not fit in 64 bits",
        )
    })
}

/// Aplica uma operação dentro de uma transação já aberta (insere ou atualiza + evento).
pub(crate) fn apply_op(tx: &Transaction<'_>, op: &CatalogOp) -> StoreResult<()> {
    let r = &op.record;
    tx.execute(
        "INSERT INTO media_assets(asset_id, kind, content_hash, size_bytes, display_name, location_json, known_paths_json, media_info_json, status, status_checked_ms, imported_ms, fingerprint) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
         ON CONFLICT(asset_id) DO UPDATE SET kind = excluded.kind, content_hash = excluded.content_hash, size_bytes = excluded.size_bytes, \
            display_name = excluded.display_name, location_json = excluded.location_json, known_paths_json = excluded.known_paths_json, \
            media_info_json = excluded.media_info_json, status = excluded.status, status_checked_ms = excluded.status_checked_ms, \
            fingerprint = COALESCE(excluded.fingerprint, fingerprint)",
        params![
            r.asset_id.as_str(),
            r.kind.as_str(),
            r.content_hash.as_str(),
            to_i64(r.size_bytes)?,
            r.display_name,
            ser("location", &r.location)?,
            ser("known paths", &r.known_paths)?,
            ser("media info", &r.media)?,
            r.status.as_str(),
            to_i64(r.status_checked_ms)?,
            to_i64(r.imported_ms)?,
            r.fingerprint,
        ],
    )?;
    tx.execute(
        "INSERT INTO asset_events(asset_id, kind, detail_json, at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![
            r.asset_id.as_str(),
            op.event.as_str(),
            ser("event detail", &op.detail)?,
            to_i64(op.at_ms)?
        ],
    )?;
    Ok(())
}

struct Raw {
    asset_id: String,
    kind: String,
    content_hash: String,
    size_bytes: i64,
    display_name: String,
    location_json: String,
    known_paths_json: String,
    media_info_json: String,
    status: String,
    status_checked_ms: i64,
    imported_ms: i64,
    fingerprint: Option<String>,
}

const COLUMNS: &str = "asset_id, kind, content_hash, size_bytes, display_name, location_json, known_paths_json, media_info_json, status, status_checked_ms, imported_ms, fingerprint";

fn raw_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        asset_id: r.get(0)?,
        kind: r.get(1)?,
        content_hash: r.get(2)?,
        size_bytes: r.get(3)?,
        display_name: r.get(4)?,
        location_json: r.get(5)?,
        known_paths_json: r.get(6)?,
        media_info_json: r.get(7)?,
        status: r.get(8)?,
        status_checked_ms: r.get(9)?,
        imported_ms: r.get(10)?,
        fingerprint: r.get(11)?,
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(id: &str, what: &str, text: &str) -> StoreResult<T> {
    if text.len() > MAX_JSON {
        return Err(corrupted(format!("catalog {what} of {id} is too large")));
    }
    serde_json::from_str(text)
        .map_err(|e| corrupted(format!("catalog {what} of {id} is invalid")).with_cause(e))
}

fn to_record(raw: Raw) -> StoreResult<AssetRecord> {
    let id = raw.asset_id;
    if id.is_empty() || id.len() > 128 {
        return Err(corrupted("catalog asset id is invalid"));
    }
    let non_neg = |v: i64, what: &str| {
        u64::try_from(v).map_err(|_| corrupted(format!("catalog {what} of {id} is negative")))
    };
    Ok(AssetRecord {
        kind: AssetKind::parse(&raw.kind)
            .ok_or_else(|| corrupted(format!("catalog kind of {id} is unknown")))?,
        content_hash: ContentHash::parse(&raw.content_hash)
            .ok_or_else(|| corrupted(format!("catalog hash of {id} is malformed")))?,
        size_bytes: non_neg(raw.size_bytes, "size")?,
        display_name: raw.display_name,
        location: parse_json::<AssetLocation>(&id, "location", &raw.location_json)?,
        known_paths: parse_json::<Vec<String>>(&id, "known paths", &raw.known_paths_json)?,
        media: parse_json::<MediaInfo>(&id, "media info", &raw.media_info_json)?,
        status: Availability::parse(&raw.status)
            .ok_or_else(|| corrupted(format!("catalog status of {id} is unknown")))?,
        status_checked_ms: non_neg(raw.status_checked_ms, "status time")?,
        imported_ms: non_neg(raw.imported_ms, "import time")?,
        fingerprint: raw.fingerprint,
        asset_id: AssetId::new(id),
    })
}

/// Lê todo o catálogo (ordem estável por `asset_id`), validando cada linha.
pub(crate) fn read_all(conn: &Connection) -> StoreResult<Vec<AssetRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM media_assets ORDER BY asset_id"
    ))?;
    let rows = stmt.query_map([], raw_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(to_record(r?)?);
    }
    Ok(out)
}

pub(crate) fn count(conn: &Connection) -> StoreResult<u64> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// Conexão dedicada ao catálogo (WAL permite coexistir com a conexão do journal).
#[derive(Debug)]
pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    /// Abre o catálogo de um projeto **já aberto/migrado** por `ProjectStore`.
    pub fn open(path: &Path, busy_timeout: Duration) -> StoreResult<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(busy_timeout)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "trusted_schema", false)?;
        crate::schema::check_signature(
            crate::schema::peek_connection(&conn)?,
            crate::schema::CURRENT_SCHEMA_VERSION,
        )?;
        Ok(Self { conn })
    }

    pub fn list(&self) -> StoreResult<Vec<AssetRecord>> {
        read_all(&self.conn)
    }

    pub fn count(&self) -> StoreResult<u64> {
        count(&self.conn)
    }

    pub fn get(&self, id: &AssetId) -> StoreResult<Option<AssetRecord>> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM media_assets WHERE asset_id = ?1"),
                [id.as_str()],
                raw_row,
            )
            .optional()?;
        raw.map(to_record).transpose()
    }

    pub fn find_by_hash(&self, hash: &ContentHash) -> StoreResult<Option<AssetRecord>> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM media_assets WHERE content_hash = ?1"),
                [hash.as_str()],
                raw_row,
            )
            .optional()?;
        raw.map(to_record).transpose()
    }

    /// Grava uma operação numa transação própria (`IMMEDIATE`): tudo ou nada.
    pub fn apply(&mut self, op: &CatalogOp) -> StoreResult<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        apply_op(&tx, op)?;
        tx.commit()?;
        Ok(())
    }

    /// Várias operações numa ÚNICA transação (volume): tudo ou nada.
    pub fn apply_batch(&mut self, ops: &[CatalogOp]) -> StoreResult<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for op in ops {
            apply_op(&tx, op)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atualiza só a disponibilidade conhecida (sem evento: é cache de leitura, não fato).
    pub fn set_status(
        &mut self,
        id: &AssetId,
        status: Availability,
        at_ms: u64,
    ) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE media_assets SET status = ?2, status_checked_ms = ?3 WHERE asset_id = ?1",
            params![id.as_str(), status.as_str(), to_i64(at_ms)?],
        )?;
        Ok(())
    }

    pub fn events(&self, id: &AssetId) -> StoreResult<Vec<CatalogEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, asset_id, kind, detail_json, at_ms FROM asset_events WHERE asset_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([id.as_str()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, asset_id, kind, detail, at) = r?;
            out.push(CatalogEvent {
                seq,
                kind,
                detail: parse_json(&asset_id, "event detail", &detail)?,
                at_ms: u64::try_from(at).unwrap_or(0),
                asset_id,
            });
        }
        Ok(out)
    }
}

/// Validação do catálogo para `validate_file`: cada linha precisa ser legível e coerente.
pub(crate) fn validate(conn: &Connection) -> Vec<StoreError> {
    match read_all(conn) {
        Ok(records) => {
            let mut issues = Vec::new();
            for r in records {
                if r.asset_id.as_str().is_empty() {
                    issues.push(corrupted("catalog row with empty id"));
                }
            }
            issues
        }
        Err(e) => vec![e],
    }
}
