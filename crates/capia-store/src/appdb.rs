//! Banco **global do app** (configuração do usuário que não pertence a um projeto: registry de
//! providers/modelos/Brain Profiles, orçamentos, preferências de IA — ADR-082). É um SQLite à
//! parte do `.capia`, com assinatura (`application_id` "CAPA"), migrations próprias e
//! forward-only, rejeição de schema futuro sem tocar o arquivo e leitura que nunca devolve SQLite
//! cru. **Nenhum segredo**: credenciais ficam no cofre do SO; valores registrados são redigidos.

use crate::error::{StoreError, StoreErrorCode, StoreResult};
use crate::schema::{Migration, Peek, check_signature_with, peek, peek_connection, run_migrations};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, Transaction, params};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// `PRAGMA application_id` do banco do app ("CAPA").
pub const APP_APPLICATION_ID: i64 = 0x4341_5041;
pub const APP_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_APP_JSON: usize = 8 * 1024 * 1024;

const APP_V1_SQL: &str = "
CREATE TABLE schema_migrations (
    version       INTEGER PRIMARY KEY NOT NULL,
    name          TEXT    NOT NULL,
    applied_at_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE kv (
    ns         TEXT    NOT NULL CHECK (length(ns) BETWEEN 1 AND 48),
    key        TEXT    NOT NULL CHECK (length(key) BETWEEN 1 AND 192),
    json       TEXT    NOT NULL CHECK (json_valid(json)),
    updated_ms INTEGER NOT NULL CHECK (updated_ms >= 0),
    PRIMARY KEY (ns, key)
) STRICT;
";

fn app_m001(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(APP_V1_SQL)?;
    tx.pragma_update(None, "application_id", APP_APPLICATION_ID)
}

pub const APP_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "app kv",
    up: app_m001,
}];

#[derive(Debug)]
pub struct AppDb {
    conn: Mutex<Connection>,
    path: PathBuf,
}

fn not_app_db(msg: &str) -> StoreError {
    StoreError::new(StoreErrorCode::NotACapiaProject, msg)
}

impl AppDb {
    /// Abre (criando se não existir). Arquivo alheio/corrompido/futuro ⇒ erro estruturado **sem** modificá-lo.
    pub fn open(path: &Path, busy_timeout: Duration) -> StoreResult<Self> {
        Self::open_with(path, busy_timeout, APP_MIGRATIONS, APP_SCHEMA_VERSION)
    }

    pub fn open_with(
        path: &Path,
        busy_timeout: Duration,
        migrations: &[Migration],
        target: u32,
    ) -> StoreResult<Self> {
        let existed = path.exists();
        let from = if existed {
            // lê a assinatura SEM abrir conexão (não cria -wal/-shm em arquivo alheio)
            let p: Peek = peek(path)?;
            if p.application_id != APP_APPLICATION_ID {
                return Err(not_app_db(
                    "the file is a SQLite database but not a CapIA app database",
                ));
            }
            check_signature_with(p, target, "app database")?;
            p.user_version
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            0
        };
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        conn.busy_timeout(busy_timeout)?;
        conn.pragma_update(None, "trusted_schema", false)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        // autenticidade pós-WAL
        if existed {
            let p = peek_connection(&conn)?;
            if p.application_id != APP_APPLICATION_ID {
                return Err(not_app_db("the file is not a CapIA app database"));
            }
            check_signature_with(p, target, "app database")?;
        }
        let from_live = if existed {
            peek_connection(&conn)?.user_version
        } else {
            from
        };
        if from_live < target {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
            run_migrations(
                &mut conn,
                if existed { Some(path) } else { None },
                migrations,
                from_live,
                target,
                now,
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn schema_version(&self) -> StoreResult<u32> {
        Ok(peek_connection(&self.conn())?.user_version)
    }

    pub fn put(&self, ns: &str, key: &str, json: &Value, now_ms: u64) -> StoreResult<()> {
        if ns.is_empty() || ns.len() > 48 || key.is_empty() || key.len() > 192 {
            return Err(StoreError::new(
                StoreErrorCode::InvalidArgument,
                "ns/key out of bounds",
            ));
        }
        let text = serde_json::to_string(json).map_err(|e| {
            StoreError::new(StoreErrorCode::InvalidArgument, "value is not serializable")
                .with_cause(e)
        })?;
        let text = capia_secrets::redact_registered_global(&text);
        if text.len() > MAX_APP_JSON {
            return Err(StoreError::new(
                StoreErrorCode::InvalidArgument,
                "value is too large",
            ));
        }
        self.conn().execute(
            "INSERT INTO kv(ns, key, json, updated_ms) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(ns, key) DO UPDATE SET json = excluded.json, updated_ms = excluded.updated_ms",
            params![ns, key, text, i64::try_from(now_ms).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    pub fn get(&self, ns: &str, key: &str) -> StoreResult<Option<Value>> {
        let text: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM kv WHERE ns = ?1 AND key = ?2",
                params![ns, key],
                |r| r.get(0),
            )
            .optional()?;
        match text {
            None => Ok(None),
            Some(t) if t.len() > MAX_APP_JSON => Err(StoreError::corrupted(format!(
                "app value {ns}/{key} is too large"
            ))),
            Some(t) => serde_json::from_str(&t).map(Some).map_err(|e| {
                StoreError::corrupted(format!("app value {ns}/{key} is not valid JSON"))
                    .with_cause(e)
            }),
        }
    }

    pub fn delete(&self, ns: &str, key: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "DELETE FROM kv WHERE ns = ?1 AND key = ?2",
            params![ns, key],
        )? > 0)
    }

    pub fn list_keys(&self, ns: &str) -> StoreResult<Vec<String>> {
        let conn = self.conn();
        let mut st = conn.prepare("SELECT key FROM kv WHERE ns = ?1 ORDER BY key")?;
        let rows = st
            .query_map([ns], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
