//! Banco do **capia-server** (Fase 6, ADR-102): tokens (só hash), registry de projetos, idempotência,
//! auditoria de chamadas externas, log de eventos, webhooks e entregas, exports. É um SQLite à parte
//! (assinatura "CAPS"), com migrations forward-only e a mesma disciplina do `AppDb`: schema futuro é
//! rejeitado sem tocar o arquivo e nenhum erro vaza SQLite cru. **Nenhum segredo em claro**: o
//! segredo de um token só existe como SHA-256 e o de um webhook fica no cofre (`secret_ref`).

use crate::appdb::open_connection;
use crate::error::{StoreError, StoreErrorCode, StoreResult};
use crate::schema::{Migration, peek_connection};
use rusqlite::{Connection, OptionalExtension as _, Row, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// `PRAGMA application_id` do banco do servidor ("CAPS").
pub const SERVER_APPLICATION_ID: i64 = 0x4341_5053;
pub const SERVER_SCHEMA_VERSION: u32 = 1;
const MAX_JSON: usize = 4 * 1024 * 1024;

const SERVER_V1_SQL: &str = "
CREATE TABLE schema_migrations (
    version       INTEGER PRIMARY KEY NOT NULL,
    name          TEXT    NOT NULL,
    applied_at_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE api_tokens (
    id           TEXT PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    secret_hash  TEXT NOT NULL CHECK (length(secret_hash) = 64),
    prefix       TEXT NOT NULL,
    scopes       TEXT NOT NULL CHECK (json_valid(scopes)),
    created_ms   INTEGER NOT NULL,
    expires_ms   INTEGER,
    last_used_ms INTEGER,
    revoked_ms   INTEGER,
    rotated_from TEXT
) STRICT;
CREATE UNIQUE INDEX api_tokens_hash ON api_tokens(secret_hash);
CREATE TABLE projects (
    id             TEXT PRIMARY KEY NOT NULL,
    name           TEXT NOT NULL,
    path           TEXT NOT NULL UNIQUE,
    created_ms     INTEGER NOT NULL,
    last_opened_ms INTEGER
) STRICT;
CREATE TABLE idempotency (
    token_id     TEXT NOT NULL,
    key          TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    op           TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status       INTEGER NOT NULL,
    body         TEXT,
    created_ms   INTEGER NOT NULL,
    PRIMARY KEY (token_id, key)
) STRICT;
CREATE TABLE audit (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms           INTEGER NOT NULL,
    request_id      TEXT NOT NULL,
    token_id        TEXT,
    surface         TEXT NOT NULL,
    op              TEXT NOT NULL,
    mutating        INTEGER NOT NULL,
    outcome         TEXT NOT NULL,
    code            TEXT,
    project_id      TEXT,
    revision_before INTEGER,
    revision_after  INTEGER,
    idempotency_key TEXT
) STRICT;
CREATE TABLE events (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id    TEXT NOT NULL UNIQUE,
    type        TEXT NOT NULL,
    occurred_ms INTEGER NOT NULL,
    project_id  TEXT,
    run_id      TEXT,
    export_id   TEXT,
    data        TEXT NOT NULL CHECK (json_valid(data))
) STRICT;
CREATE TABLE webhooks (
    id          TEXT PRIMARY KEY NOT NULL,
    url         TEXT NOT NULL,
    events      TEXT NOT NULL CHECK (json_valid(events)),
    secret_ref  TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    description TEXT NOT NULL DEFAULT '',
    created_ms  INTEGER NOT NULL,
    updated_ms  INTEGER NOT NULL
) STRICT;
CREATE TABLE deliveries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    webhook_id      TEXT NOT NULL,
    event_seq       INTEGER NOT NULL,
    event_id        TEXT NOT NULL,
    attempt         INTEGER NOT NULL DEFAULT 0,
    state           TEXT NOT NULL CHECK (state IN ('pending','delivering','retrying','delivered','dead')),
    next_attempt_ms INTEGER NOT NULL,
    last_status     INTEGER,
    last_latency_ms INTEGER,
    last_error      TEXT,
    created_ms      INTEGER NOT NULL,
    updated_ms      INTEGER NOT NULL,
    UNIQUE (webhook_id, event_id)
) STRICT;
CREATE INDEX deliveries_due ON deliveries(state, next_attempt_ms);
CREATE TABLE exports (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL,
    batch       TEXT NOT NULL,
    sequence    TEXT NOT NULL,
    preset      TEXT NOT NULL,
    state       TEXT NOT NULL CHECK (state IN ('queued','running','completed','failed','cancelled')),
    path        TEXT,
    report      TEXT CHECK (report IS NULL OR json_valid(report)),
    error       TEXT CHECK (error IS NULL OR json_valid(error)),
    created_ms  INTEGER NOT NULL,
    finished_ms INTEGER
) STRICT;
";

fn server_m001(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(SERVER_V1_SQL)?;
    tx.pragma_update(None, "application_id", SERVER_APPLICATION_ID)
}

pub const SERVER_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "server core",
    up: server_m001,
}];

fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn json_text(v: &Value) -> StoreResult<String> {
    let t = capia_secrets::redact_registered_global(&serde_json::to_string(v).map_err(|e| {
        StoreError::new(StoreErrorCode::InvalidArgument, "value is not serializable").with_cause(e)
    })?);
    if t.len() > MAX_JSON {
        return Err(StoreError::new(
            StoreErrorCode::InvalidArgument,
            "value is too large",
        ));
    }
    Ok(t)
}

fn parse_json(t: &str) -> StoreResult<Value> {
    serde_json::from_str(t)
        .map_err(|e| StoreError::corrupted("a stored JSON value is invalid").with_cause(e))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRow {
    pub id: String,
    pub name: String,
    pub secret_hash: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub created_ms: u64,
    pub expires_ms: Option<u64>,
    pub last_used_ms: Option<u64>,
    pub revoked_ms: Option<u64>,
    pub rotated_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_ms: u64,
    pub last_opened_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdemBegin {
    /// Chave nova: execute e chame `idem_finish`.
    New,
    /// Mesma chave e mesmo pedido já concluído: devolva a resposta guardada.
    Replay { status: u16, body: String },
    /// Mesma chave com pedido diferente.
    Mismatch,
    /// Outro pedido com esta chave ainda está em andamento.
    InProgress,
    /// Ficou `pending` de um processo que caiu: o efeito é indeterminado.
    Indeterminate,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditRow {
    pub seq: i64,
    pub at_ms: u64,
    pub request_id: String,
    pub token_id: Option<String>,
    pub surface: String,
    pub op: String,
    pub mutating: bool,
    pub outcome: String,
    pub code: Option<String>,
    pub project_id: Option<String>,
    pub revision_before: Option<u64>,
    pub revision_after: Option<u64>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerEventRow {
    pub seq: i64,
    pub event_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub occurred_ms: u64,
    pub project_id: Option<String>,
    pub run_id: Option<String>,
    pub export_id: Option<String>,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookRow {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub secret_ref: String,
    pub enabled: bool,
    pub description: String,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRow {
    pub id: i64,
    pub webhook_id: String,
    pub event_seq: i64,
    pub event_id: String,
    pub attempt: u32,
    pub state: String,
    pub next_attempt_ms: u64,
    pub last_status: Option<u16>,
    pub last_latency_ms: Option<u64>,
    pub last_error: Option<String>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportRow {
    pub id: String,
    pub project_id: String,
    pub batch: String,
    pub sequence: String,
    pub preset: String,
    pub state: String,
    pub path: Option<String>,
    pub report: Option<Value>,
    pub error: Option<Value>,
    pub created_ms: u64,
    pub finished_ms: Option<u64>,
}

#[derive(Debug)]
pub struct ServerDb {
    conn: Mutex<Connection>,
    path: PathBuf,
}

fn token_of(r: &Row<'_>) -> rusqlite::Result<(TokenRow, String)> {
    let scopes: String = r.get(4)?;
    Ok((
        TokenRow {
            id: r.get(0)?,
            name: r.get(1)?,
            secret_hash: r.get(2)?,
            prefix: r.get(3)?,
            scopes: Vec::new(),
            created_ms: u(r.get(5)?),
            expires_ms: r.get::<_, Option<i64>>(6)?.map(u),
            last_used_ms: r.get::<_, Option<i64>>(7)?.map(u),
            revoked_ms: r.get::<_, Option<i64>>(8)?.map(u),
            rotated_from: r.get(9)?,
        },
        scopes,
    ))
}

const TOKEN_COLS: &str = "id, name, secret_hash, prefix, scopes, created_ms, expires_ms, last_used_ms, revoked_ms, rotated_from";

fn finish_token((mut t, scopes): (TokenRow, String)) -> StoreResult<TokenRow> {
    t.scopes = serde_json::from_str(&scopes)
        .map_err(|e| StoreError::corrupted("token scopes are invalid").with_cause(e))?;
    Ok(t)
}

fn webhook_of(r: &Row<'_>) -> rusqlite::Result<(WebhookRow, String)> {
    Ok((
        WebhookRow {
            id: r.get(0)?,
            url: r.get(1)?,
            events: Vec::new(),
            secret_ref: r.get(3)?,
            enabled: r.get::<_, i64>(4)? != 0,
            description: r.get(5)?,
            created_ms: u(r.get(6)?),
            updated_ms: u(r.get(7)?),
        },
        r.get(2)?,
    ))
}

const WEBHOOK_COLS: &str =
    "id, url, events, secret_ref, enabled, description, created_ms, updated_ms";

fn finish_webhook((mut w, events): (WebhookRow, String)) -> StoreResult<WebhookRow> {
    w.events = serde_json::from_str(&events)
        .map_err(|e| StoreError::corrupted("webhook events are invalid").with_cause(e))?;
    Ok(w)
}

fn delivery_of(r: &Row<'_>) -> rusqlite::Result<DeliveryRow> {
    Ok(DeliveryRow {
        id: r.get(0)?,
        webhook_id: r.get(1)?,
        event_seq: r.get(2)?,
        event_id: r.get(3)?,
        attempt: u32::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
        state: r.get(5)?,
        next_attempt_ms: u(r.get(6)?),
        last_status: r
            .get::<_, Option<i64>>(7)?
            .and_then(|v| u16::try_from(v).ok()),
        last_latency_ms: r.get::<_, Option<i64>>(8)?.map(u),
        last_error: r.get(9)?,
        created_ms: u(r.get(10)?),
        updated_ms: u(r.get(11)?),
    })
}

const DELIVERY_COLS: &str = "id, webhook_id, event_seq, event_id, attempt, state, next_attempt_ms, last_status, last_latency_ms, last_error, created_ms, updated_ms";

fn event_of(r: &Row<'_>) -> rusqlite::Result<(ServerEventRow, String)> {
    Ok((
        ServerEventRow {
            seq: r.get(0)?,
            event_id: r.get(1)?,
            kind: r.get(2)?,
            occurred_ms: u(r.get(3)?),
            project_id: r.get(4)?,
            run_id: r.get(5)?,
            export_id: r.get(6)?,
            data: Value::Null,
        },
        r.get(7)?,
    ))
}

const EVENT_COLS: &str = "seq, event_id, type, occurred_ms, project_id, run_id, export_id, data";

fn export_of(r: &Row<'_>) -> rusqlite::Result<(ExportRow, Option<String>, Option<String>)> {
    Ok((
        ExportRow {
            id: r.get(0)?,
            project_id: r.get(1)?,
            batch: r.get(2)?,
            sequence: r.get(3)?,
            preset: r.get(4)?,
            state: r.get(5)?,
            path: r.get(6)?,
            report: None,
            error: None,
            created_ms: u(r.get(9)?),
            finished_ms: r.get::<_, Option<i64>>(10)?.map(u),
        },
        r.get(7)?,
        r.get(8)?,
    ))
}

const EXPORT_COLS: &str =
    "id, project_id, batch, sequence, preset, state, path, report, error, created_ms, finished_ms";

fn finish_export(
    (mut e, report, error): (ExportRow, Option<String>, Option<String>),
) -> StoreResult<ExportRow> {
    e.report = report.as_deref().map(parse_json).transpose()?;
    e.error = error.as_deref().map(parse_json).transpose()?;
    Ok(e)
}

impl ServerDb {
    /// Abre (criando se não existir). Arquivo alheio/corrompido/futuro ⇒ erro estruturado **sem** modificá-lo.
    pub fn open(path: &Path, busy_timeout: Duration) -> StoreResult<Self> {
        let conn = open_connection(
            path,
            busy_timeout,
            SERVER_MIGRATIONS,
            SERVER_SCHEMA_VERSION,
            SERVER_APPLICATION_ID,
            "server database",
        )?;
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

    // ---- tokens -----------------------------------------------------------------------------

    pub fn token_insert(&self, t: &TokenRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO api_tokens(id, name, secret_hash, prefix, scopes, created_ms, expires_ms, \
             last_used_ms, revoked_ms, rotated_from) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,NULL,?8)",
            params![
                t.id,
                t.name,
                t.secret_hash,
                t.prefix,
                serde_json::to_string(&t.scopes).unwrap_or_else(|_| "[]".into()),
                ms(t.created_ms),
                t.expires_ms.map(ms),
                t.rotated_from,
            ],
        )?;
        Ok(())
    }

    pub fn token_by_hash(&self, hash: &str) -> StoreResult<Option<TokenRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {TOKEN_COLS} FROM api_tokens WHERE secret_hash = ?1"),
                [hash],
                token_of,
            )
            .optional()?
            .map(finish_token)
            .transpose()
    }

    pub fn token_get(&self, id: &str) -> StoreResult<Option<TokenRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {TOKEN_COLS} FROM api_tokens WHERE id = ?1"),
                [id],
                token_of,
            )
            .optional()?
            .map(finish_token)
            .transpose()
    }

    pub fn token_list(&self) -> StoreResult<Vec<TokenRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {TOKEN_COLS} FROM api_tokens ORDER BY created_ms, id"
        ))?;
        let rows = st.query_map([], token_of)?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(finish_token).collect()
    }

    pub fn token_count_active(&self, now_ms: u64) -> StoreResult<u64> {
        Ok(u(self.conn().query_row(
            "SELECT COUNT(*) FROM api_tokens WHERE revoked_ms IS NULL AND (expires_ms IS NULL OR expires_ms > ?1)",
            [ms(now_ms)],
            |r| r.get(0),
        )?))
    }

    pub fn token_revoke(&self, id: &str, now_ms: u64) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE api_tokens SET revoked_ms = ?2 WHERE id = ?1 AND revoked_ms IS NULL",
            params![id, ms(now_ms)],
        )? > 0)
    }

    pub fn token_touch(&self, id: &str, now_ms: u64) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE api_tokens SET last_used_ms = ?2 WHERE id = ?1",
            params![id, ms(now_ms)],
        )?;
        Ok(())
    }

    /// Rotação atômica: cria o novo e revoga o antigo na mesma transação.
    pub fn token_rotate(&self, old_id: &str, new: &TokenRow, now_ms: u64) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE api_tokens SET revoked_ms = ?2 WHERE id = ?1 AND revoked_ms IS NULL",
            params![old_id, ms(now_ms)],
        )?;
        if n == 0 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO api_tokens(id, name, secret_hash, prefix, scopes, created_ms, expires_ms, \
             last_used_ms, revoked_ms, rotated_from) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,NULL,?8)",
            params![
                new.id,
                new.name,
                new.secret_hash,
                new.prefix,
                serde_json::to_string(&new.scopes).unwrap_or_else(|_| "[]".into()),
                ms(new.created_ms),
                new.expires_ms.map(ms),
                new.rotated_from,
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    // ---- projetos ---------------------------------------------------------------------------

    pub fn project_insert(&self, p: &ProjectRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO projects(id, name, path, created_ms, last_opened_ms) VALUES (?1,?2,?3,?4,?5)",
            params![p.id, p.name, p.path, ms(p.created_ms), p.last_opened_ms.map(ms)],
        )?;
        Ok(())
    }

    fn project_of(r: &Row<'_>) -> rusqlite::Result<ProjectRow> {
        Ok(ProjectRow {
            id: r.get(0)?,
            name: r.get(1)?,
            path: r.get(2)?,
            created_ms: u(r.get(3)?),
            last_opened_ms: r.get::<_, Option<i64>>(4)?.map(u),
        })
    }

    pub fn project_get(&self, id: &str) -> StoreResult<Option<ProjectRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, path, created_ms, last_opened_ms FROM projects WHERE id = ?1",
                [id],
                Self::project_of,
            )
            .optional()?)
    }

    pub fn project_by_path(&self, path: &str) -> StoreResult<Option<ProjectRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, path, created_ms, last_opened_ms FROM projects WHERE path = ?1",
                [path],
                Self::project_of,
            )
            .optional()?)
    }

    pub fn project_list(&self, after_id: Option<&str>, limit: u32) -> StoreResult<Vec<ProjectRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT id, name, path, created_ms, last_opened_ms FROM projects \
             WHERE id > ?1 ORDER BY id LIMIT ?2",
        )?;
        let rows = st
            .query_map(
                params![after_id.unwrap_or(""), i64::from(limit)],
                Self::project_of,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn project_touch_open(&self, id: &str, now_ms: u64) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE projects SET last_opened_ms = ?2 WHERE id = ?1",
            params![id, ms(now_ms)],
        )?;
        Ok(())
    }

    // ---- idempotência -----------------------------------------------------------------------

    /// Reserva `(token, key)`. `stale_ms`: um `pending` mais velho que isto veio de um processo que
    /// caiu — o efeito é indeterminado e o cliente deve verificar o estado e usar outra chave.
    pub fn idem_begin(
        &self,
        token_id: &str,
        key: &str,
        op: &str,
        request_hash: &str,
        now_ms: u64,
        stale_ms: u64,
    ) -> StoreResult<IdemBegin> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let row: Option<(String, String, i64, Option<String>, i64)> = tx
            .query_row(
                "SELECT op, request_hash, status, body, created_ms FROM idempotency \
                 WHERE token_id = ?1 AND key = ?2",
                params![token_id, key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let out = match row {
            None => {
                tx.execute(
                    "INSERT INTO idempotency(token_id, key, op, request_hash, status, body, created_ms) \
                     VALUES (?1,?2,?3,?4,0,NULL,?5)",
                    params![token_id, key, op, request_hash, ms(now_ms)],
                )?;
                IdemBegin::New
            }
            Some((o, h, status, body, created)) => {
                if o != op || h != request_hash {
                    IdemBegin::Mismatch
                } else if status == 0 {
                    if now_ms.saturating_sub(u(created)) > stale_ms {
                        IdemBegin::Indeterminate
                    } else {
                        IdemBegin::InProgress
                    }
                } else {
                    IdemBegin::Replay {
                        status: u16::try_from(status).unwrap_or(500),
                        body: body.unwrap_or_default(),
                    }
                }
            }
        };
        tx.commit()?;
        Ok(out)
    }

    pub fn idem_finish(
        &self,
        token_id: &str,
        key: &str,
        status: u16,
        body: &str,
    ) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE idempotency SET status = ?3, body = ?4 WHERE token_id = ?1 AND key = ?2",
            params![token_id, key, i64::from(status), body],
        )?;
        Ok(())
    }

    /// Falha transitória antes de qualquer efeito: libera a chave para nova tentativa.
    pub fn idem_release(&self, token_id: &str, key: &str) -> StoreResult<()> {
        self.conn().execute(
            "DELETE FROM idempotency WHERE token_id = ?1 AND key = ?2 AND status = 0",
            params![token_id, key],
        )?;
        Ok(())
    }

    /// Na abertura: tudo que ficou `pending` veio de um processo que morreu ⇒ vira "indeterminado"
    /// (criado em 0 ⇒ passa do prazo de `stale` de imediato).
    pub fn idem_recover(&self) -> StoreResult<u64> {
        Ok(self
            .conn()
            .execute("UPDATE idempotency SET created_ms = 0 WHERE status = 0", [])?
            as u64)
    }

    pub fn idem_purge_older_than(&self, cutoff_ms: u64) -> StoreResult<u64> {
        Ok(self.conn().execute(
            "DELETE FROM idempotency WHERE created_ms < ?1 AND status <> 0",
            [ms(cutoff_ms)],
        )? as u64)
    }

    // ---- auditoria --------------------------------------------------------------------------

    pub fn audit_append(&self, a: &AuditRow) -> StoreResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO audit(at_ms, request_id, token_id, surface, op, mutating, outcome, code, \
             project_id, revision_before, revision_after, idempotency_key) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                ms(a.at_ms),
                a.request_id,
                a.token_id,
                a.surface,
                a.op,
                i64::from(a.mutating),
                a.outcome,
                a.code,
                a.project_id,
                a.revision_before.map(ms),
                a.revision_after.map(ms),
                a.idempotency_key,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Mantém só as `keep` entradas mais recentes (retenção limitada do log de auditoria).
    pub fn audit_trim(&self, keep: u64) -> StoreResult<u64> {
        Ok(self.conn().execute(
            "DELETE FROM audit WHERE seq <= (SELECT COALESCE(MAX(seq), 0) FROM audit) - ?1",
            [ms(keep)],
        )? as u64)
    }

    pub fn audit_list(&self, after_seq: i64, limit: u32) -> StoreResult<Vec<AuditRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT seq, at_ms, request_id, token_id, surface, op, mutating, outcome, code, \
             project_id, revision_before, revision_after, idempotency_key \
             FROM audit WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = st
            .query_map(params![after_seq, i64::from(limit)], |r| {
                Ok(AuditRow {
                    seq: r.get(0)?,
                    at_ms: u(r.get(1)?),
                    request_id: r.get(2)?,
                    token_id: r.get(3)?,
                    surface: r.get(4)?,
                    op: r.get(5)?,
                    mutating: r.get::<_, i64>(6)? != 0,
                    outcome: r.get(7)?,
                    code: r.get(8)?,
                    project_id: r.get(9)?,
                    revision_before: r.get::<_, Option<i64>>(10)?.map(u),
                    revision_after: r.get::<_, Option<i64>>(11)?.map(u),
                    idempotency_key: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- webhooks ---------------------------------------------------------------------------

    pub fn webhook_insert(&self, w: &WebhookRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO webhooks(id, url, events, secret_ref, enabled, description, created_ms, updated_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                w.id,
                w.url,
                serde_json::to_string(&w.events).unwrap_or_else(|_| "[]".into()),
                w.secret_ref,
                i64::from(w.enabled),
                w.description,
                ms(w.created_ms),
                ms(w.updated_ms),
            ],
        )?;
        Ok(())
    }

    pub fn webhook_update(&self, w: &WebhookRow) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE webhooks SET url = ?2, events = ?3, secret_ref = ?4, enabled = ?5, \
             description = ?6, updated_ms = ?7 WHERE id = ?1",
            params![
                w.id,
                w.url,
                serde_json::to_string(&w.events).unwrap_or_else(|_| "[]".into()),
                w.secret_ref,
                i64::from(w.enabled),
                w.description,
                ms(w.updated_ms),
            ],
        )? > 0)
    }

    pub fn webhook_get(&self, id: &str) -> StoreResult<Option<WebhookRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {WEBHOOK_COLS} FROM webhooks WHERE id = ?1"),
                [id],
                webhook_of,
            )
            .optional()?
            .map(finish_webhook)
            .transpose()
    }

    pub fn webhook_list(&self) -> StoreResult<Vec<WebhookRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {WEBHOOK_COLS} FROM webhooks ORDER BY created_ms, id"
        ))?;
        let rows = st
            .query_map([], webhook_of)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(finish_webhook).collect()
    }

    pub fn webhook_delete(&self, id: &str) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM deliveries WHERE webhook_id = ?1", [id])?;
        let n = tx.execute("DELETE FROM webhooks WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(n > 0)
    }

    // ---- eventos + entregas -----------------------------------------------------------------

    /// Registra um evento (idempotente por `event_id`) e enfileira, **na mesma transação**, uma
    /// entrega por webhook habilitado que assina o tipo (ou só para `only_webhook`, nos eventos de
    /// teste). Devolve `None` se o evento já existia.
    #[allow(clippy::too_many_arguments)]
    pub fn event_append(
        &self,
        event_id: &str,
        kind: &str,
        occurred_ms: u64,
        project_id: Option<&str>,
        run_id: Option<&str>,
        export_id: Option<&str>,
        data: &Value,
        only_webhook: Option<&str>,
    ) -> StoreResult<Option<i64>> {
        let text = json_text(data)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "INSERT OR IGNORE INTO events(event_id, type, occurred_ms, project_id, run_id, export_id, data) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![event_id, kind, ms(occurred_ms), project_id, run_id, export_id, text],
        )?;
        if n == 0 {
            return Ok(None);
        }
        let seq = tx.last_insert_rowid();
        let hooks: Vec<WebhookRow> = {
            let mut st = tx.prepare(&format!(
                "SELECT {WEBHOOK_COLS} FROM webhooks WHERE enabled = 1 OR ?1 IS NOT NULL"
            ))?;
            let rows = st
                .query_map(params![only_webhook], webhook_of)?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter()
                .map(finish_webhook)
                .collect::<StoreResult<_>>()?
        };
        for h in hooks.iter().filter(|h| match only_webhook {
            Some(w) => h.id == w,
            None => h.events.iter().any(|e| e == "*" || e == kind),
        }) {
            tx.execute(
                "INSERT OR IGNORE INTO deliveries(webhook_id, event_seq, event_id, attempt, state, \
                 next_attempt_ms, created_ms, updated_ms) VALUES (?1,?2,?3,0,'pending',?4,?4,?4)",
                params![h.id, seq, event_id, ms(occurred_ms)],
            )?;
        }
        tx.commit()?;
        Ok(Some(seq))
    }

    pub fn events_after(&self, after_seq: i64, limit: u32) -> StoreResult<Vec<ServerEventRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {EVENT_COLS} FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2"
        ))?;
        let rows = st
            .query_map(params![after_seq, i64::from(limit)], event_of)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(mut e, d)| {
                e.data = parse_json(&d)?;
                Ok(e)
            })
            .collect()
    }

    pub fn event_get(&self, seq: i64) -> StoreResult<Option<ServerEventRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {EVENT_COLS} FROM events WHERE seq = ?1"),
                [seq],
                event_of,
            )
            .optional()?
            .map(|(mut e, d)| {
                e.data = parse_json(&d)?;
                Ok(e)
            })
            .transpose()
    }

    pub fn last_event_seq(&self) -> StoreResult<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?)
    }

    /// Enfileira uma entrega de teste/reentrega para um evento já existente (cria ou reabre).
    pub fn delivery_enqueue(
        &self,
        webhook_id: &str,
        event: &ServerEventRow,
        now_ms: u64,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO deliveries(webhook_id, event_seq, event_id, attempt, state, next_attempt_ms, \
             created_ms, updated_ms) VALUES (?1,?2,?3,0,'pending',?4,?4,?4) \
             ON CONFLICT(webhook_id, event_id) DO UPDATE SET state='pending', attempt=0, \
             next_attempt_ms=excluded.next_attempt_ms, updated_ms=excluded.updated_ms",
            params![webhook_id, event.seq, event.event_id, ms(now_ms)],
        )?;
        Ok(())
    }

    /// Reabre uma entrega (`dead`/`delivered`) pelo id, para reentrega manual.
    pub fn delivery_requeue(&self, id: i64, now_ms: u64) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE deliveries SET state='pending', attempt=0, next_attempt_ms=?2, updated_ms=?2 \
             WHERE id = ?1 AND state IN ('dead','delivered','retrying')",
            params![id, ms(now_ms)],
        )? > 0)
    }

    /// Entregas vencidas; marca-as `delivering` (reserva) na mesma transação.
    pub fn deliveries_claim_due(&self, now_ms: u64, limit: u32) -> StoreResult<Vec<DeliveryRow>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let rows = {
            let mut st = tx.prepare(&format!(
                "SELECT {DELIVERY_COLS} FROM deliveries WHERE state IN ('pending','retrying') \
                 AND next_attempt_ms <= ?1 ORDER BY next_attempt_ms, id LIMIT ?2"
            ))?;
            st.query_map(params![ms(now_ms), i64::from(limit)], delivery_of)?
                .collect::<Result<Vec<_>, _>>()?
        };
        for r in &rows {
            tx.execute(
                "UPDATE deliveries SET state='delivering', updated_ms=?2 WHERE id = ?1",
                params![r.id, ms(now_ms)],
            )?;
        }
        tx.commit()?;
        Ok(rows)
    }

    /// Reabertura do servidor: o que estava `delivering` volta a `retrying` (entrega at-least-once).
    pub fn deliveries_recover(&self, now_ms: u64) -> StoreResult<u64> {
        Ok(self.conn().execute(
            "UPDATE deliveries SET state='retrying', next_attempt_ms=?1, updated_ms=?1 \
             WHERE state='delivering'",
            [ms(now_ms)],
        )? as u64)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn delivery_finish(
        &self,
        id: i64,
        state: &str,
        attempt: u32,
        next_attempt_ms: u64,
        status: Option<u16>,
        latency_ms: Option<u64>,
        error: Option<&str>,
        now_ms: u64,
    ) -> StoreResult<()> {
        let error = error.map(capia_secrets::redact_registered_global);
        self.conn().execute(
            "UPDATE deliveries SET state=?2, attempt=?3, next_attempt_ms=?4, last_status=?5, \
             last_latency_ms=?6, last_error=?7, updated_ms=?8 WHERE id = ?1",
            params![
                id,
                state,
                i64::from(attempt),
                ms(next_attempt_ms),
                status.map(i64::from),
                latency_ms.map(ms),
                error,
                ms(now_ms),
            ],
        )?;
        Ok(())
    }

    pub fn deliveries_pending_count(&self) -> StoreResult<u64> {
        Ok(u(self.conn().query_row(
            "SELECT COUNT(*) FROM deliveries WHERE state IN ('pending','retrying','delivering')",
            [],
            |r| r.get(0),
        )?))
    }

    pub fn delivery_get(&self, id: i64) -> StoreResult<Option<DeliveryRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {DELIVERY_COLS} FROM deliveries WHERE id = ?1"),
                [id],
                delivery_of,
            )
            .optional()?)
    }

    pub fn deliveries_list(
        &self,
        webhook_id: Option<&str>,
        after_id: i64,
        limit: u32,
    ) -> StoreResult<Vec<DeliveryRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {DELIVERY_COLS} FROM deliveries WHERE id > ?1 AND (?2 IS NULL OR webhook_id = ?2) \
             ORDER BY id LIMIT ?3"
        ))?;
        let rows = st
            .query_map(params![after_id, webhook_id, i64::from(limit)], delivery_of)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- exports ----------------------------------------------------------------------------

    pub fn export_insert(&self, e: &ExportRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO exports(id, project_id, batch, sequence, preset, state, path, report, error, \
             created_ms, finished_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,NULL,?8,NULL)",
            params![
                e.id,
                e.project_id,
                e.batch,
                e.sequence,
                e.preset,
                e.state,
                e.path,
                ms(e.created_ms)
            ],
        )?;
        Ok(())
    }

    pub fn export_update(
        &self,
        id: &str,
        state: &str,
        report: Option<&Value>,
        error: Option<&Value>,
        now_ms: u64,
    ) -> StoreResult<bool> {
        let report = report.map(json_text).transpose()?;
        let error = error.map(json_text).transpose()?;
        let finished = matches!(state, "completed" | "failed" | "cancelled");
        Ok(self.conn().execute(
            "UPDATE exports SET state=?2, report=COALESCE(?3, report), error=COALESCE(?4, error), \
             finished_ms=CASE WHEN ?5 THEN ?6 ELSE finished_ms END WHERE id = ?1",
            params![id, state, report, error, i64::from(finished), ms(now_ms)],
        )? > 0)
    }

    pub fn export_set_batch(&self, id: &str, batch: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE exports SET batch = ?2 WHERE id = ?1",
            params![id, batch],
        )?;
        Ok(())
    }

    pub fn export_get(&self, id: &str) -> StoreResult<Option<ExportRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {EXPORT_COLS} FROM exports WHERE id = ?1"),
                [id],
                export_of,
            )
            .optional()?
            .map(finish_export)
            .transpose()
    }

    pub fn export_by_batch_item(&self, batch: &str, id: &str) -> StoreResult<Option<ExportRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {EXPORT_COLS} FROM exports WHERE batch = ?1 AND id = ?2"),
                params![batch, id],
                export_of,
            )
            .optional()?
            .map(finish_export)
            .transpose()
    }

    pub fn export_list(
        &self,
        project_id: Option<&str>,
        after_id: &str,
        limit: u32,
    ) -> StoreResult<Vec<ExportRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {EXPORT_COLS} FROM exports WHERE id > ?1 AND (?2 IS NULL OR project_id = ?2) \
             ORDER BY id LIMIT ?3"
        ))?;
        let rows = st
            .query_map(params![after_id, project_id, i64::from(limit)], export_of)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(finish_export).collect()
    }

    /// Reabertura: exports que estavam `queued/running` não sobreviveram ao processo.
    pub fn exports_recover(&self, now_ms: u64) -> StoreResult<u64> {
        let err = r#"{"code":"INTERRUPTED","message":"the server stopped while this export was running"}"#;
        Ok(self.conn().execute(
            "UPDATE exports SET state='failed', error=?2, finished_ms=?1 WHERE state IN ('queued','running')",
            params![ms(now_ms), err],
        )? as u64)
    }
}
