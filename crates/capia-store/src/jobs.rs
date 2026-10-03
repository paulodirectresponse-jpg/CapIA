//! Persistência dos jobs de mídia e dos tickets de import assíncrono (ADR-052/053, schema 3).
//!
//! `JobStore` tem **conexão própria** (WAL permite coexistir com o journal e o catálogo) e
//! implementa [`JobSink`]: o executor grava cada mudança de estado/progresso daqui, de qualquer
//! thread. Nada disto é documento nem undo: é estado operacional recuperável.

use crate::error::{StoreError, StoreResult};
use capia_jobs::{JobError, JobId, JobKind, JobSink, JobSnapshot, JobState, Priority, Progress};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const MAX_JSON: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    /// Aceito; hash/probe em andamento (ou na fila).
    Pending,
    Finalized,
    Failed,
    Cancelled,
    /// O processo terminou com o ticket pendente: pode ser **retomado**.
    Interrupted,
}

impl TicketState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Finalized => "finalized",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "finalized" => Self::Finalized,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TicketRow {
    pub ticket_id: String,
    pub path: String,
    pub size_bytes: u64,
    pub fingerprint: Option<String>,
    pub state: TicketState,
    pub job_id: Option<String>,
    pub asset_id: Option<String>,
    pub outcome: Option<Value>,
    pub error: Option<Value>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

/// O que a recuperação do abrir do projeto fez.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    pub jobs_interrupted: u64,
    pub tickets_interrupted: u64,
}

#[derive(Debug)]
pub struct JobStore {
    conn: Mutex<Connection>,
}

fn corrupted(msg: impl Into<String>) -> StoreError {
    StoreError::corrupted(msg)
}

fn i(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn json_of<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn parse_opt(id: &str, what: &str, text: Option<String>) -> StoreResult<Option<Value>> {
    match text {
        None => Ok(None),
        Some(t) if t.len() > MAX_JSON => Err(corrupted(format!("{what} of {id} is too large"))),
        Some(t) => serde_json::from_str(&t)
            .map(Some)
            .map_err(|e| corrupted(format!("{what} of {id} is invalid")).with_cause(e)),
    }
}

impl JobStore {
    /// Abre a conexão de um projeto **já aberto/migrado** por `ProjectStore`.
    pub fn open(path: &Path, busy_timeout: Duration) -> StoreResult<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(busy_timeout)?;
        conn.pragma_update(None, "trusted_schema", false)?;
        crate::schema::check_signature(
            crate::schema::peek_connection(&conn)?,
            crate::schema::CURRENT_SCHEMA_VERSION,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reabrir o projeto: o que estava `queued`/`running` não está mais (o processo acabou) ⇒
    /// `interrupted` (nunca `completed`); tickets `pending` idem. Idempotente.
    pub fn recover(&self, now_ms: u64) -> StoreResult<Recovery> {
        let conn = self.conn();
        let jobs = conn.execute(
            "UPDATE jobs SET state = 'interrupted', finished_ms = ?1, updated_ms = ?1 \
             WHERE state IN ('queued', 'running')",
            [i(now_ms)],
        )?;
        let tickets = conn.execute(
            "UPDATE import_tickets SET state = 'interrupted', updated_ms = ?1 WHERE state = 'pending'",
            [i(now_ms)],
        )?;
        Ok(Recovery {
            jobs_interrupted: jobs as u64,
            tickets_interrupted: tickets as u64,
        })
    }

    pub fn upsert(&self, s: &JobSnapshot) -> StoreResult<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO jobs(job_id, kind, priority, state, progress_done, progress_total, dedup_key, label, params_json, \
                result_json, error_json, created_ms, started_ms, finished_ms, updated_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15) \
             ON CONFLICT(job_id) DO UPDATE SET state = excluded.state, progress_done = excluded.progress_done, \
                progress_total = excluded.progress_total, result_json = excluded.result_json, error_json = excluded.error_json, \
                started_ms = excluded.started_ms, finished_ms = excluded.finished_ms, updated_ms = excluded.updated_ms",
            params![
                s.id.0,
                s.kind.as_str(),
                s.priority.as_str(),
                s.state.as_str(),
                i(s.progress.done),
                i(s.progress.total),
                s.dedup_key,
                s.label,
                json_of(&s.params),
                s.result.as_ref().map(json_of),
                s.error.as_ref().map(json_of),
                i(s.created_ms),
                s.started_ms.map(i),
                s.finished_ms.map(i),
                i(s.finished_ms.or(s.started_ms).unwrap_or(s.created_ms)),
            ],
        )?;
        Ok(())
    }

    fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawJob> {
        Ok(RawJob {
            id: r.get(0)?,
            kind: r.get(1)?,
            priority: r.get(2)?,
            state: r.get(3)?,
            done: r.get(4)?,
            total: r.get(5)?,
            dedup: r.get(6)?,
            label: r.get(7)?,
            params: r.get(8)?,
            result: r.get(9)?,
            error: r.get(10)?,
            created: r.get(11)?,
            started: r.get(12)?,
            finished: r.get(13)?,
        })
    }

    const COLS: &'static str = "job_id, kind, priority, state, progress_done, progress_total, dedup_key, label, params_json, result_json, error_json, created_ms, started_ms, finished_ms";

    pub fn get(&self, id: &JobId) -> StoreResult<Option<JobSnapshot>> {
        let conn = self.conn();
        let raw = conn
            .query_row(
                &format!("SELECT {} FROM jobs WHERE job_id = ?1", Self::COLS),
                [&id.0],
                Self::row,
            )
            .optional()?;
        raw.map(RawJob::into_snapshot).transpose()
    }

    /// Mais recentes primeiro. `state = None` ⇒ todos.
    pub fn list(&self, state: Option<JobState>, limit: u32) -> StoreResult<Vec<JobSnapshot>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {} FROM jobs {} ORDER BY created_ms DESC, job_id DESC LIMIT ?1",
            Self::COLS,
            if state.is_some() {
                "WHERE state = ?2"
            } else {
                ""
            }
        );
        let mut stmt = conn.prepare(&sql)?;
        let map = |r: &rusqlite::Row<'_>| Self::row(r);
        let rows = match state {
            Some(st) => stmt
                .query_map(params![limit, st.as_str()], map)?
                .collect::<Vec<_>>(),
            None => stmt.query_map(params![limit], map)?.collect::<Vec<_>>(),
        };
        rows.into_iter()
            .map(|r| r.map_err(StoreError::from).and_then(RawJob::into_snapshot))
            .collect()
    }

    /// Pede cancelamento (vale entre processos: o dono do executor observa a flag). `false` se o
    /// job não existe ou já terminou.
    pub fn request_cancel(&self, id: &JobId) -> StoreResult<bool> {
        let n = self.conn().execute(
            "UPDATE jobs SET cancel_requested = 1 WHERE job_id = ?1 AND state IN ('queued', 'running')",
            [&id.0],
        )?;
        Ok(n > 0)
    }

    /// Jobs não terminais com cancelamento pedido.
    pub fn pending_cancels(&self) -> StoreResult<Vec<JobId>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT job_id FROM jobs WHERE cancel_requested = 1 AND state IN ('queued', 'running')",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| r.map(JobId).map_err(StoreError::from))
            .collect()
    }

    /// Marca como `cancelled` um job que **nenhum executor está rodando** (ex.: dono morreu e
    /// ainda não houve recover). Usado pela CLI quando não há executor vivo.
    pub fn force_state(&self, id: &JobId, state: JobState, now_ms: u64) -> StoreResult<bool> {
        let n = self.conn().execute(
            "UPDATE jobs SET state = ?2, finished_ms = ?3, updated_ms = ?3 WHERE job_id = ?1 AND state IN ('queued','running','interrupted')",
            params![id.0, state.as_str(), i(now_ms)],
        )?;
        Ok(n > 0)
    }

    // ---- tickets -----------------------------------------------------------------------------

    pub fn put_ticket(&self, t: &TicketRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO import_tickets(ticket_id, path, size_bytes, fingerprint, state, job_id, asset_id, outcome_json, error_json, created_ms, updated_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) \
             ON CONFLICT(ticket_id) DO UPDATE SET state = excluded.state, job_id = excluded.job_id, asset_id = excluded.asset_id, \
                outcome_json = excluded.outcome_json, error_json = excluded.error_json, updated_ms = excluded.updated_ms",
            params![
                t.ticket_id,
                t.path,
                i(t.size_bytes),
                t.fingerprint,
                t.state.as_str(),
                t.job_id,
                t.asset_id,
                t.outcome.as_ref().map(json_of),
                t.error.as_ref().map(json_of),
                i(t.created_ms),
                i(t.updated_ms),
            ],
        )?;
        Ok(())
    }

    fn ticket_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawTicket> {
        Ok(RawTicket {
            ticket_id: r.get(0)?,
            path: r.get(1)?,
            size: r.get(2)?,
            fingerprint: r.get(3)?,
            state: r.get(4)?,
            job_id: r.get(5)?,
            asset_id: r.get(6)?,
            outcome: r.get(7)?,
            error: r.get(8)?,
            created: r.get(9)?,
            updated: r.get(10)?,
        })
    }

    const TCOLS: &'static str = "ticket_id, path, size_bytes, fingerprint, state, job_id, asset_id, outcome_json, error_json, created_ms, updated_ms";

    pub fn get_ticket(&self, id: &str) -> StoreResult<Option<TicketRow>> {
        let conn = self.conn();
        let raw = conn
            .query_row(
                &format!(
                    "SELECT {} FROM import_tickets WHERE ticket_id = ?1",
                    Self::TCOLS
                ),
                [id],
                Self::ticket_row,
            )
            .optional()?;
        raw.map(RawTicket::into_row).transpose()
    }

    pub fn list_tickets(&self, state: Option<TicketState>) -> StoreResult<Vec<TicketRow>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {} FROM import_tickets {} ORDER BY created_ms, ticket_id",
            Self::TCOLS,
            if state.is_some() {
                "WHERE state = ?1"
            } else {
                ""
            }
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = match state {
            Some(st) => stmt
                .query_map([st.as_str()], Self::ticket_row)?
                .collect::<Vec<_>>(),
            None => stmt.query_map([], Self::ticket_row)?.collect::<Vec<_>>(),
        };
        rows.into_iter()
            .map(|r| r.map_err(StoreError::from).and_then(RawTicket::into_row))
            .collect()
    }
}

struct RawJob {
    id: String,
    kind: String,
    priority: String,
    state: String,
    done: i64,
    total: i64,
    dedup: Option<String>,
    label: String,
    params: String,
    result: Option<String>,
    error: Option<String>,
    created: i64,
    started: Option<i64>,
    finished: Option<i64>,
}

impl RawJob {
    fn into_snapshot(self) -> StoreResult<JobSnapshot> {
        let id = self.id;
        let bad = |what: &str| corrupted(format!("job {id}: {what}"));
        if self.params.len() > MAX_JSON {
            return Err(bad("params are too large"));
        }
        Ok(JobSnapshot {
            kind: JobKind::parse(&self.kind).ok_or_else(|| bad("unknown kind"))?,
            priority: Priority::parse(&self.priority).ok_or_else(|| bad("unknown priority"))?,
            state: JobState::parse(&self.state).ok_or_else(|| bad("unknown state"))?,
            progress: Progress {
                done: u(self.done),
                total: u(self.total),
            },
            dedup_key: self.dedup,
            label: self.label,
            params: serde_json::from_str(&self.params).map_err(|_| bad("params are invalid"))?,
            result: parse_opt(&id, "result", self.result)?,
            error: parse_opt(&id, "error", self.error)?
                .map(|v| serde_json::from_value::<JobError>(v).map_err(|_| bad("error is invalid")))
                .transpose()?,
            created_ms: u(self.created),
            started_ms: self.started.map(u),
            finished_ms: self.finished.map(u),
            id: JobId(id.clone()),
        })
    }
}

struct RawTicket {
    ticket_id: String,
    path: String,
    size: i64,
    fingerprint: Option<String>,
    state: String,
    job_id: Option<String>,
    asset_id: Option<String>,
    outcome: Option<String>,
    error: Option<String>,
    created: i64,
    updated: i64,
}

impl RawTicket {
    fn into_row(self) -> StoreResult<TicketRow> {
        let id = self.ticket_id;
        Ok(TicketRow {
            state: TicketState::parse(&self.state)
                .ok_or_else(|| corrupted(format!("ticket {id}: unknown state")))?,
            outcome: parse_opt(&id, "ticket outcome", self.outcome)?,
            error: parse_opt(&id, "ticket error", self.error)?,
            path: self.path,
            size_bytes: u(self.size),
            fingerprint: self.fingerprint,
            job_id: self.job_id,
            asset_id: self.asset_id,
            created_ms: u(self.created),
            updated_ms: u(self.updated),
            ticket_id: id,
        })
    }
}

impl JobSink for JobStore {
    fn record(&self, snapshot: &JobSnapshot) {
        // falha de persistência do ESTADO operacional nunca derruba o job (ele já fez o trabalho);
        // o pior caso é o estado não sobreviver a um crash — que o `recover` trata.
        let _ = self.upsert(snapshot);
    }
}
