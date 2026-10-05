//! Persistência da **autonomia** (schema 5, ADR-087): Runs com cursor atômico, registro de cada
//! execução de stage, eventos duráveis, livro de efeitos colaterais (idempotência), proveniência,
//! memória de projeto e livro-razão de orçamento. Conexão própria (WAL coexiste com o journal).
//!
//! Garantias:
//! * toda transição de Run é **uma** transação SQLite com *compare-and-swap* por `revision`
//!   (`advance`): o registro do stage, os eventos e o novo cursor entram juntos ou não entram;
//! * `claim_effect` é atômico (`INSERT OR IGNORE`): duas tentativas do mesmo efeito (resume,
//!   retry, callback duplicado) enxergam **o mesmo** registro — só a primeira executa;
//! * nenhum segredo: valores registrados no `capia-secrets` são redigidos antes de gravar.
//!
//! Nada aqui é documento nem undo.

use crate::error::{StoreError, StoreErrorCode, StoreResult};
use crate::failpoints::fp;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// Teto de um registro de autonomia (planos longos cabem; lixo não).
pub const MAX_AUTONOMY_JSON: usize = 8 * 1024 * 1024;

fn i(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn bad(msg: impl Into<String>) -> StoreError {
    StoreError::new(StoreErrorCode::InvalidArgument, msg)
}

fn corrupted(msg: impl Into<String>) -> StoreError {
    StoreError::corrupted(msg)
}

fn to_text(v: &Value) -> StoreResult<String> {
    let text =
        serde_json::to_string(v).map_err(|e| bad("value is not serializable").with_cause(e))?;
    let text = capia_secrets::redact_registered_global(&text);
    if text.len() > MAX_AUTONOMY_JSON {
        return Err(bad(format!("record exceeds {MAX_AUTONOMY_JSON} bytes")));
    }
    Ok(text)
}

fn from_text(text: &str, what: &str) -> StoreResult<Value> {
    serde_json::from_str(text)
        .map_err(|e| corrupted(format!("{what} is not valid JSON")).with_cause(e))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRow {
    pub run_id: String,
    pub status: String,
    pub stage: String,
    pub revision: u64,
    pub parent_run_id: Option<String>,
    pub variant_group: Option<String>,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub json: Value,
}

/// Mudança do cursor de uma Run (aplicada com CAS).
#[derive(Clone, Debug)]
pub struct RunUpdate {
    pub status: String,
    pub stage: String,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageRow {
    pub run_id: String,
    pub seq: u64,
    pub stage: String,
    pub attempt: u32,
    pub idem_key: String,
    pub input_digest: String,
    pub output_digest: Option<String>,
    pub status: String,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectRow {
    pub effect_key: String,
    pub run_id: String,
    pub kind: String,
    pub state: String,
    pub external_id: Option<String>,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub json: Value,
}

/// Resultado de `claim_effect`: quem chega primeiro executa; os demais leem o registro.
#[derive(Clone, Debug, PartialEq)]
pub enum Claim {
    New(EffectRow),
    Existing(EffectRow),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRow {
    pub run_id: String,
    pub seq: u64,
    pub kind: String,
    pub ts_ms: u64,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceRow {
    pub asset_id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub content_hash: String,
    pub created_ms: u64,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRow {
    pub id: String,
    pub status: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryLogRow {
    pub seq: u64,
    pub memory_id: String,
    pub event: String,
    pub actor: String,
    pub run_id: Option<String>,
    pub ts_ms: u64,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedgerRow {
    pub seq: u64,
    pub run_id: String,
    pub reservation_id: String,
    pub kind: String,
    pub micros: u64,
    pub ts_ms: u64,
    pub json: Value,
}

#[derive(Debug)]
pub struct AutonomyStore {
    conn: Mutex<Connection>,
}

const RUN_COLS: &str =
    "run_id, status, stage, revision, parent_run_id, variant_group, created_ms, updated_ms, json";

type RawRun = (
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    i64,
    i64,
    String,
);

fn raw_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRun> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    ))
}

fn run_from(t: RawRun) -> StoreResult<RunRow> {
    Ok(RunRow {
        json: from_text(&t.8, &format!("run {}", t.0))?,
        run_id: t.0,
        status: t.1,
        stage: t.2,
        revision: u(t.3),
        parent_run_id: t.4,
        variant_group: t.5,
        created_ms: u(t.6),
        updated_ms: u(t.7),
    })
}

const STAGE_COLS: &str = "run_id, seq, stage, attempt, idem_key, input_digest, output_digest, status, started_ms, ended_ms, json";

type RawStage = (
    String,
    i64,
    String,
    i64,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<i64>,
    String,
);

fn raw_stage(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawStage> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ))
}

fn stage_from(t: RawStage) -> StoreResult<StageRow> {
    Ok(StageRow {
        json: from_text(&t.10, "stage record")?,
        run_id: t.0,
        seq: u(t.1),
        stage: t.2,
        attempt: u32::try_from(t.3).unwrap_or(0),
        idem_key: t.4,
        input_digest: t.5,
        output_digest: t.6,
        status: t.7,
        started_ms: u(t.8),
        ended_ms: t.9.map(u),
    })
}

const EFFECT_COLS: &str =
    "effect_key, run_id, kind, state, external_id, created_ms, updated_ms, json";

type RawEffect = (
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    String,
);

fn raw_effect(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawEffect> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}

fn effect_from(t: RawEffect) -> StoreResult<EffectRow> {
    Ok(EffectRow {
        json: from_text(&t.7, "side effect")?,
        effect_key: t.0,
        run_id: t.1,
        kind: t.2,
        state: t.3,
        external_id: t.4,
        created_ms: u(t.5),
        updated_ms: u(t.6),
    })
}

fn insert_stage(tx: &Transaction<'_>, s: &StageRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO ai_run_stages(run_id, seq, stage, attempt, idem_key, input_digest, output_digest, status, started_ms, ended_ms, json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(run_id, seq) DO UPDATE SET output_digest = excluded.output_digest, status = excluded.status, \
            ended_ms = excluded.ended_ms, json = excluded.json",
        params![
            s.run_id,
            i(s.seq),
            s.stage,
            s.attempt,
            s.idem_key,
            s.input_digest,
            s.output_digest,
            s.status,
            i(s.started_ms),
            s.ended_ms.map(i),
            to_text(&s.json)?
        ],
    )?;
    Ok(())
}

fn next_event_seq(tx: &Transaction<'_>, run_id: &str) -> StoreResult<i64> {
    Ok(tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM ai_run_events WHERE run_id = ?1",
        [run_id],
        |r| r.get(0),
    )?)
}

fn insert_event(
    tx: &Transaction<'_>,
    run_id: &str,
    kind: &str,
    json: &Value,
    now_ms: u64,
) -> StoreResult<u64> {
    if kind.is_empty() || kind.len() > 48 {
        return Err(bad("event kind out of bounds"));
    }
    let seq = next_event_seq(tx, run_id)?;
    tx.execute(
        "INSERT INTO ai_run_events(run_id, seq, kind, ts_ms, json) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id, seq, kind, i(now_ms), to_text(json)?],
    )?;
    Ok(u(seq))
}

impl AutonomyStore {
    /// Abre a conexão de um projeto **já aberto/migrado** por `ProjectStore` (schema ≥ 5).
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

    // ---- runs ---------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn create_run(
        &self,
        run_id: &str,
        status: &str,
        stage: &str,
        parent: Option<&str>,
        variant_group: Option<&str>,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<()> {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO ai_runs(run_id, status, stage, revision, parent_run_id, variant_group, created_ms, updated_ms, json) \
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?6, ?7)",
            params![run_id, status, stage, parent, variant_group, i(now_ms), to_text(json)?],
        )?;
        if n == 0 {
            return Err(StoreError::new(
                StoreErrorCode::InvalidArgument,
                format!("run {run_id} already exists"),
            ));
        }
        Ok(())
    }

    pub fn get_run(&self, run_id: &str) -> StoreResult<Option<RunRow>> {
        let t = self
            .conn()
            .query_row(
                &format!("SELECT {RUN_COLS} FROM ai_runs WHERE run_id = ?1"),
                [run_id],
                raw_run,
            )
            .optional()?;
        t.map(run_from).transpose()
    }

    /// Runs (mais recentes primeiro), opcionalmente filtradas por status.
    pub fn list_runs(&self, statuses: Option<&[&str]>, limit: u32) -> StoreResult<Vec<RunRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {RUN_COLS} FROM ai_runs ORDER BY created_ms DESC, run_id DESC LIMIT ?1"
        ))?;
        let rows = st
            .query_map([i64::from(limit.clamp(1, 1_000))], raw_run)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        let mut out = Vec::new();
        for t in rows {
            let r = run_from(t)?;
            if statuses.is_none_or(|s| s.contains(&r.status.as_str())) {
                out.push(r);
            }
        }
        Ok(out)
    }

    pub fn runs_in_group(&self, group: &str) -> StoreResult<Vec<RunRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {RUN_COLS} FROM ai_runs WHERE variant_group = ?1 ORDER BY created_ms, run_id"
        ))?;
        let rows = st
            .query_map([group], raw_run)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter().map(run_from).collect()
    }

    /// **Transição atômica**: CAS no `revision`, novo cursor, registro de stage opcional e eventos —
    /// tudo na mesma transação. `Conflict` se outro escritor avançou a Run no meio-tempo.
    pub fn advance(
        &self,
        run_id: &str,
        expected_revision: u64,
        update: &RunUpdate,
        stage: Option<&StageRow>,
        events: &[(String, Value)],
        now_ms: u64,
    ) -> StoreResult<u64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE ai_runs SET status = ?1, stage = ?2, json = ?3, revision = revision + 1, updated_ms = ?4 \
             WHERE run_id = ?5 AND revision = ?6",
            params![
                update.status,
                update.stage,
                to_text(&update.json)?,
                i(now_ms),
                run_id,
                i(expected_revision)
            ],
        )?;
        if n == 0 {
            return Err(StoreError::new(
                StoreErrorCode::StoreConflict,
                format!(
                    "run {run_id} changed concurrently (expected revision {expected_revision})"
                ),
            ));
        }
        if let Some(s) = stage {
            insert_stage(&tx, s)?;
        }
        for (kind, json) in events {
            insert_event(&tx, run_id, kind, json, now_ms)?;
        }
        fp!("autonomy_advance_before_commit");
        tx.commit()?;
        fp!("autonomy_advance_after_commit");
        Ok(expected_revision + 1)
    }

    /// Reescreve o `json` da Run sem mudar status/stage (CAS por revision), p.ex. uso/custo.
    pub fn touch_run(
        &self,
        run_id: &str,
        expected_revision: u64,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<u64> {
        let n = self.conn().execute(
            "UPDATE ai_runs SET json = ?1, revision = revision + 1, updated_ms = ?2 WHERE run_id = ?3 AND revision = ?4",
            params![to_text(json)?, i(now_ms), run_id, i(expected_revision)],
        )?;
        if n == 0 {
            return Err(StoreError::new(
                StoreErrorCode::StoreConflict,
                format!("run {run_id} changed concurrently"),
            ));
        }
        Ok(expected_revision + 1)
    }

    // ---- stages -------------------------------------------------------------------------------

    pub fn put_stage(&self, s: &StageRow) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        insert_stage(&tx, s)?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_stages(&self, run_id: &str) -> StoreResult<Vec<StageRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {STAGE_COLS} FROM ai_run_stages WHERE run_id = ?1 ORDER BY seq"
        ))?;
        let rows = st
            .query_map([run_id], raw_stage)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter().map(stage_from).collect()
    }

    pub fn stage_by_key(&self, idem_key: &str) -> StoreResult<Option<StageRow>> {
        let t = self
            .conn()
            .query_row(
                &format!("SELECT {STAGE_COLS} FROM ai_run_stages WHERE idem_key = ?1"),
                [idem_key],
                raw_stage,
            )
            .optional()?;
        t.map(stage_from).transpose()
    }

    pub fn next_stage_seq(&self, run_id: &str) -> StoreResult<u64> {
        let n: i64 = self.conn().query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM ai_run_stages WHERE run_id = ?1",
            [run_id],
            |r| r.get(0),
        )?;
        Ok(u(n))
    }

    /// Reabrir o projeto: stages `started` de um processo que morreu viram `interrupted` (nunca
    /// `completed`). Idempotente.
    pub fn interrupt_started_stages(&self, now_ms: u64) -> StoreResult<usize> {
        Ok(self.conn().execute(
            "UPDATE ai_run_stages SET status = 'interrupted', ended_ms = ?1 WHERE status = 'started'",
            [i(now_ms)],
        )?)
    }

    // ---- eventos ------------------------------------------------------------------------------

    pub fn append_event(
        &self,
        run_id: &str,
        kind: &str,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<u64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let seq = insert_event(&tx, run_id, kind, json, now_ms)?;
        tx.commit()?;
        Ok(seq)
    }

    pub fn events_after(&self, run_id: &str, after: u64, limit: u32) -> StoreResult<Vec<EventRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT run_id, seq, kind, ts_ms, json FROM ai_run_events WHERE run_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
        )?;
        let rows = st
            .query_map(
                params![run_id, i(after), i64::from(limit.clamp(1, 5_000))],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter()
            .map(|t| {
                Ok(EventRow {
                    json: from_text(&t.4, "run event")?,
                    run_id: t.0,
                    seq: u(t.1),
                    kind: t.2,
                    ts_ms: u(t.3),
                })
            })
            .collect()
    }

    // ---- efeitos colaterais (idempotência) ----------------------------------------------------

    /// Reivindica um efeito. A **primeira** chamada cria o registro em `intent` e devolve `New`;
    /// qualquer outra (resume, retry, callback duplicado) devolve `Existing` com o estado atual.
    pub fn claim_effect(
        &self,
        key: &str,
        run_id: &str,
        kind: &str,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<Claim> {
        if key.is_empty() || key.len() > 256 || kind.is_empty() || kind.len() > 48 {
            return Err(bad("effect key/kind out of bounds"));
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "INSERT OR IGNORE INTO ai_side_effects(effect_key, run_id, kind, state, external_id, created_ms, updated_ms, json) \
             VALUES (?1, ?2, ?3, 'intent', NULL, ?4, ?4, ?5)",
            params![key, run_id, kind, i(now_ms), to_text(json)?],
        )?;
        let row = tx.query_row(
            &format!("SELECT {EFFECT_COLS} FROM ai_side_effects WHERE effect_key = ?1"),
            [key],
            raw_effect,
        )?;
        tx.commit()?;
        let row = effect_from(row)?;
        Ok(if n == 1 {
            Claim::New(row)
        } else {
            Claim::Existing(row)
        })
    }

    pub fn update_effect(
        &self,
        key: &str,
        state: &str,
        external_id: Option<&str>,
        json: Option<&Value>,
        now_ms: u64,
    ) -> StoreResult<()> {
        let text = json.map(to_text).transpose()?;
        let n = self.conn().execute(
            "UPDATE ai_side_effects SET state = ?1, external_id = COALESCE(?2, external_id), \
             json = COALESCE(?3, json), updated_ms = ?4 WHERE effect_key = ?5",
            params![state, external_id, text, i(now_ms), key],
        )?;
        if n == 0 {
            return Err(bad(format!("unknown side effect {key}")));
        }
        Ok(())
    }

    pub fn get_effect(&self, key: &str) -> StoreResult<Option<EffectRow>> {
        let t = self
            .conn()
            .query_row(
                &format!("SELECT {EFFECT_COLS} FROM ai_side_effects WHERE effect_key = ?1"),
                [key],
                raw_effect,
            )
            .optional()?;
        t.map(effect_from).transpose()
    }

    pub fn effects_for_run(&self, run_id: &str) -> StoreResult<Vec<EffectRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {EFFECT_COLS} FROM ai_side_effects WHERE run_id = ?1 ORDER BY created_ms, effect_key"
        ))?;
        let rows = st
            .query_map([run_id], raw_effect)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter().map(effect_from).collect()
    }

    // ---- proveniência -------------------------------------------------------------------------

    pub fn put_provenance(
        &self,
        asset_id: &str,
        run_id: Option<&str>,
        kind: &str,
        content_hash: &str,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO ai_provenance(asset_id, run_id, kind, content_hash, created_ms, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(asset_id) DO UPDATE SET run_id = excluded.run_id, kind = excluded.kind, \
                content_hash = excluded.content_hash, json = excluded.json",
            params![asset_id, run_id, kind, content_hash, i(now_ms), to_text(json)?],
        )?;
        Ok(())
    }

    fn prov_from(
        t: (String, Option<String>, String, String, i64, String),
    ) -> StoreResult<ProvenanceRow> {
        Ok(ProvenanceRow {
            json: from_text(&t.5, "provenance")?,
            asset_id: t.0,
            run_id: t.1,
            kind: t.2,
            content_hash: t.3,
            created_ms: u(t.4),
        })
    }

    pub fn get_provenance(&self, asset_id: &str) -> StoreResult<Option<ProvenanceRow>> {
        let t = self
            .conn()
            .query_row(
                "SELECT asset_id, run_id, kind, content_hash, created_ms, json FROM ai_provenance WHERE asset_id = ?1",
                [asset_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        t.map(Self::prov_from).transpose()
    }

    pub fn provenance_by_hash(&self, content_hash: &str) -> StoreResult<Option<ProvenanceRow>> {
        let t = self
            .conn()
            .query_row(
                "SELECT asset_id, run_id, kind, content_hash, created_ms, json FROM ai_provenance WHERE content_hash = ?1 ORDER BY created_ms LIMIT 1",
                [content_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        t.map(Self::prov_from).transpose()
    }

    pub fn list_provenance(&self, run_id: Option<&str>) -> StoreResult<Vec<ProvenanceRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT asset_id, run_id, kind, content_hash, created_ms, json FROM ai_provenance \
             WHERE (?1 IS NULL OR run_id = ?1) ORDER BY created_ms, asset_id",
        )?;
        let rows = st
            .query_map([run_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter().map(Self::prov_from).collect()
    }

    // ---- memória de projeto -------------------------------------------------------------------

    pub fn put_memory(&self, id: &str, status: &str, json: &Value, now_ms: u64) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO ai_memory(id, scope, status, created_ms, updated_ms, json) VALUES (?1, 'project', ?2, ?3, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET status = excluded.status, updated_ms = excluded.updated_ms, json = excluded.json",
            params![id, status, i(now_ms), to_text(json)?],
        )?;
        Ok(())
    }

    pub fn get_memory(&self, id: &str) -> StoreResult<Option<MemoryRow>> {
        let t = self
            .conn()
            .query_row(
                "SELECT id, status, created_ms, updated_ms, json FROM ai_memory WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        t.map(|t| {
            Ok(MemoryRow {
                json: from_text(&t.4, "memory item")?,
                id: t.0,
                status: t.1,
                created_ms: u(t.2),
                updated_ms: u(t.3),
            })
        })
        .transpose()
    }

    pub fn list_memory(&self, status: Option<&str>) -> StoreResult<Vec<MemoryRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT id, status, created_ms, updated_ms, json FROM ai_memory WHERE (?1 IS NULL OR status = ?1) ORDER BY created_ms, id",
        )?;
        let rows = st
            .query_map([status], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter()
            .map(|t| {
                Ok(MemoryRow {
                    json: from_text(&t.4, "memory item")?,
                    id: t.0,
                    status: t.1,
                    created_ms: u(t.2),
                    updated_ms: u(t.3),
                })
            })
            .collect()
    }

    pub fn delete_memory(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM ai_memory WHERE id = ?1", [id])?
            > 0)
    }

    pub fn log_memory(
        &self,
        memory_id: &str,
        event: &str,
        actor: &str,
        run_id: Option<&str>,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO ai_memory_log(memory_id, event, actor, run_id, ts_ms, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![memory_id, event, actor, run_id, i(now_ms), to_text(json)?],
        )?;
        Ok(())
    }

    pub fn memory_log(&self, memory_id: Option<&str>) -> StoreResult<Vec<MemoryLogRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT seq, memory_id, event, actor, run_id, ts_ms, json FROM ai_memory_log WHERE (?1 IS NULL OR memory_id = ?1) ORDER BY seq",
        )?;
        let rows = st
            .query_map([memory_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter()
            .map(|t| {
                Ok(MemoryLogRow {
                    json: from_text(&t.6, "memory log")?,
                    seq: u(t.0),
                    memory_id: t.1,
                    event: t.2,
                    actor: t.3,
                    run_id: t.4,
                    ts_ms: u(t.5),
                })
            })
            .collect()
    }

    // ---- livro-razão de orçamento -------------------------------------------------------------

    /// Registra um lançamento de orçamento **só se** `kind != reserve` ou se o total comprometido
    /// (reservas abertas + liquidado) mais `micros` não ultrapassar `limit` — numa única transação,
    /// o que impede workers paralelos de estourarem o teto por corrida.
    pub fn ledger_reserve(
        &self,
        run_id: &str,
        reservation_id: &str,
        micros: u64,
        limit: Option<u64>,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT seq FROM ai_budget_ledger WHERE run_id = ?1 AND reservation_id = ?2 AND kind = 'reserve'",
                params![run_id, reservation_id],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_some() {
            // reserva idempotente (retry/resume)
            tx.commit()?;
            return Ok(true);
        }
        let committed = committed_micros(&tx, run_id)?;
        if let Some(l) = limit
            && committed.saturating_add(micros) > l
        {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO ai_budget_ledger(run_id, reservation_id, kind, micros, ts_ms, json) VALUES (?1, ?2, 'reserve', ?3, ?4, ?5)",
            params![run_id, reservation_id, i(micros), i(now_ms), to_text(json)?],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Liquida uma reserva com o custo real (`actual`) e libera o excedente. Idempotente.
    pub fn ledger_settle(
        &self,
        run_id: &str,
        reservation_id: &str,
        actual: u64,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let closed: Option<i64> = tx
            .query_row(
                "SELECT seq FROM ai_budget_ledger WHERE run_id = ?1 AND reservation_id = ?2 AND kind IN ('settle','release')",
                params![run_id, reservation_id],
                |r| r.get(0),
            )
            .optional()?;
        if closed.is_some() {
            tx.commit()?;
            return Ok(());
        }
        tx.execute(
            "INSERT INTO ai_budget_ledger(run_id, reservation_id, kind, micros, ts_ms, json) VALUES (?1, ?2, 'settle', ?3, ?4, ?5)",
            params![run_id, reservation_id, i(actual), i(now_ms), to_text(json)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Libera uma reserva sem gasto (a ação não aconteceu). Idempotente.
    pub fn ledger_release(
        &self,
        run_id: &str,
        reservation_id: &str,
        now_ms: u64,
    ) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let closed: Option<i64> = tx
            .query_row(
                "SELECT seq FROM ai_budget_ledger WHERE run_id = ?1 AND reservation_id = ?2 AND kind IN ('settle','release')",
                params![run_id, reservation_id],
                |r| r.get(0),
            )
            .optional()?;
        if closed.is_none() {
            tx.execute(
                "INSERT INTO ai_budget_ledger(run_id, reservation_id, kind, micros, ts_ms, json) VALUES (?1, ?2, 'release', 0, ?3, '{}')",
                params![run_id, reservation_id, i(now_ms)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Total comprometido da Run: liquidado + reservas ainda abertas.
    pub fn ledger_committed(&self, run_id: &str) -> StoreResult<u64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let v = committed_micros(&tx, run_id)?;
        tx.commit()?;
        Ok(v)
    }

    pub fn ledger_rows(&self, run_id: &str) -> StoreResult<Vec<LedgerRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT seq, run_id, reservation_id, kind, micros, ts_ms, json FROM ai_budget_ledger WHERE run_id = ?1 ORDER BY seq",
        )?;
        let rows = st
            .query_map([run_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        drop(conn);
        rows.into_iter()
            .map(|t| {
                Ok(LedgerRow {
                    json: from_text(&t.6, "ledger row")?,
                    seq: u(t.0),
                    run_id: t.1,
                    reservation_id: t.2,
                    kind: t.3,
                    micros: u(t.4),
                    ts_ms: u(t.5),
                })
            })
            .collect()
    }
}

/// Comprometido = liquidado + reservas **sem** liquidação/liberação correspondente.
fn committed_micros(tx: &Transaction<'_>, run_id: &str) -> StoreResult<u64> {
    let settled: i64 = tx.query_row(
        "SELECT COALESCE(SUM(micros), 0) FROM ai_budget_ledger WHERE run_id = ?1 AND kind = 'settle'",
        [run_id],
        |r| r.get(0),
    )?;
    let open: i64 = tx.query_row(
        "SELECT COALESCE(SUM(r.micros), 0) FROM ai_budget_ledger r WHERE r.run_id = ?1 AND r.kind = 'reserve' \
         AND NOT EXISTS (SELECT 1 FROM ai_budget_ledger c WHERE c.run_id = r.run_id AND c.reservation_id = r.reservation_id AND c.kind IN ('settle','release'))",
        [run_id],
        |r| r.get(0),
    )?;
    Ok(u(settled).saturating_add(u(open)))
}
