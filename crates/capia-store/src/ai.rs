//! Registros de inteligência do projeto (schema 4, ADR-082): documentos JSON versionados por
//! `(kind, id, version)` — transcripts, análises, ReferenceGrammar, DemandSpec, conversas, tarefas,
//! cache determinístico — e o uso/custo de IA. Conexão própria (WAL coexiste com o journal).
//! Nada aqui é documento nem undo. **Nenhum segredo**: valores registrados no `capia-secrets` são
//! redigidos antes de gravar.

use crate::error::{StoreError, StoreErrorCode, StoreResult};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// Teto de um registro (transcripts longos cabem; lixo não).
pub const MAX_RECORD_JSON: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordRow {
    pub kind: String,
    pub id: String,
    pub version: u32,
    pub parent: Option<String>,
    pub schema_version: u32,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub request_id: String,
    pub task_id: Option<String>,
    pub provider_id: String,
    pub endpoint_id: String,
    pub model_id: String,
    pub capability: String,
    pub purpose: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub synthetic: bool,
    pub cost_known: bool,
    pub cost_micros: u64,
    pub currency: Option<String>,
    pub latency_ms: u64,
    pub attempt: u32,
    pub status: String,
    pub error_code: Option<String>,
    pub pricing_date: Option<String>,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSummary {
    pub calls: u64,
    pub failed_calls: u64,
    pub cache_hits: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Soma só do custo **conhecido**.
    pub known_cost_micros: u64,
    pub currency: Option<String>,
    /// Chamadas bem-sucedidas sem preço declarado (custo desconhecido ≠ zero).
    pub unknown_cost_calls: u64,
}

#[derive(Debug)]
pub struct AiStore {
    conn: Mutex<Connection>,
}

fn i(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn bad(msg: impl Into<String>) -> StoreError {
    StoreError::new(StoreErrorCode::InvalidArgument, msg)
}

impl AiStore {
    /// Abre a conexão de um projeto **já aberto/migrado** por `ProjectStore` (schema ≥ 4).
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

    /// Grava (upsert) uma versão de um registro.
    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &self,
        kind: &str,
        id: &str,
        version: u32,
        parent: Option<&str>,
        schema_version: u32,
        json: &Value,
        now_ms: u64,
    ) -> StoreResult<()> {
        if kind.is_empty() || kind.len() > 48 || id.is_empty() || id.len() > 192 {
            return Err(bad("record kind/id out of bounds"));
        }
        if schema_version == 0 {
            return Err(bad("schema_version must be >= 1"));
        }
        let text = serde_json::to_string(json)
            .map_err(|e| bad("record is not serializable").with_cause(e))?;
        // defesa em profundidade: nenhum valor registrado como segredo chega ao disco
        let text = capia_secrets::redact_registered_global(&text);
        if text.len() > MAX_RECORD_JSON {
            return Err(bad(format!("record exceeds {MAX_RECORD_JSON} bytes")));
        }
        self.conn().execute(
            "INSERT INTO ai_records(kind, id, version, parent, schema_version, created_ms, updated_ms, json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7) \
             ON CONFLICT(kind, id, version) DO UPDATE SET parent = excluded.parent, schema_version = excluded.schema_version, \
                updated_ms = excluded.updated_ms, json = excluded.json",
            params![kind, id, version, parent, schema_version, i(now_ms), text],
        )?;
        Ok(())
    }

    fn row(
        r: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<(String, String, i64, Option<String>, i64, i64, i64, String)> {
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

    fn build(
        t: (String, String, i64, Option<String>, i64, i64, i64, String),
    ) -> StoreResult<RecordRow> {
        let json: Value = serde_json::from_str(&t.7).map_err(|e| {
            StoreError::corrupted(format!("record {}/{} is not valid JSON", t.0, t.1)).with_cause(e)
        })?;
        Ok(RecordRow {
            kind: t.0,
            id: t.1,
            version: u32::try_from(t.2).unwrap_or(0),
            parent: t.3,
            schema_version: u32::try_from(t.4).unwrap_or(0),
            created_ms: u(t.5),
            updated_ms: u(t.6),
            json,
        })
    }

    const COLS: &'static str =
        "kind, id, version, parent, schema_version, created_ms, updated_ms, json";

    pub fn get(&self, kind: &str, id: &str, version: u32) -> StoreResult<Option<RecordRow>> {
        let conn = self.conn();
        let t = conn
            .query_row(
                &format!(
                    "SELECT {} FROM ai_records WHERE kind = ?1 AND id = ?2 AND version = ?3",
                    Self::COLS
                ),
                params![kind, id, version],
                Self::row,
            )
            .optional()?;
        t.map(Self::build).transpose()
    }

    /// Maior versão do `(kind, id)`.
    pub fn latest(&self, kind: &str, id: &str) -> StoreResult<Option<RecordRow>> {
        let conn = self.conn();
        let t = conn
            .query_row(
                &format!("SELECT {} FROM ai_records WHERE kind = ?1 AND id = ?2 ORDER BY version DESC LIMIT 1", Self::COLS),
                params![kind, id],
                Self::row,
            )
            .optional()?;
        t.map(Self::build).transpose()
    }

    /// Lista registros de um `kind` (opcionalmente de um `parent`), mais recentes primeiro. Só a última
    /// versão de cada `id` quando `latest_only`.
    pub fn list(
        &self,
        kind: &str,
        parent: Option<&str>,
        latest_only: bool,
        limit: u32,
    ) -> StoreResult<Vec<RecordRow>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {cols} FROM ai_records r WHERE kind = ?1 AND (?2 IS NULL OR parent = ?2) {only} ORDER BY updated_ms DESC, id, version DESC LIMIT ?3",
            cols = Self::COLS,
            only = if latest_only {
                "AND version = (SELECT MAX(version) FROM ai_records x WHERE x.kind = r.kind AND x.id = r.id)"
            } else {
                ""
            }
        );
        let mut st = conn.prepare(&sql)?;
        let rows = st
            .query_map(params![kind, parent, limit.min(10_000)], Self::row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(Self::build).collect()
    }

    pub fn delete(&self, kind: &str, id: &str) -> StoreResult<u64> {
        Ok(self.conn().execute(
            "DELETE FROM ai_records WHERE kind = ?1 AND id = ?2",
            params![kind, id],
        )? as u64)
    }

    /// Reabrir o projeto: tarefas `queued`/`running` não estão mais rodando ⇒ `interrupted`
    /// (nunca ficam "rodando para sempre", nunca viram `completed`). Idempotente.
    pub fn recover_tasks(&self, now_ms: u64) -> StoreResult<u64> {
        Ok(self.conn().execute(
            "UPDATE ai_records SET json = json_set(json, '$.state', 'interrupted'), updated_ms = ?1 \
             WHERE kind = 'ai_task' AND json_extract(json, '$.state') IN ('queued', 'running')",
            [i(now_ms)],
        )? as u64)
    }

    pub fn add_usage(&self, r: &UsageRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO ai_usage(request_id, task_id, provider_id, endpoint_id, model_id, capability, purpose, input_tokens, \
                output_tokens, cached_tokens, synthetic, cost_known, cost_micros, currency, latency_ms, attempt, status, error_code, \
                pricing_date, timestamp_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
            params![
                r.request_id, r.task_id, r.provider_id, r.endpoint_id, r.model_id, r.capability, r.purpose,
                i(r.input_tokens), i(r.output_tokens), i(r.cached_tokens), r.synthetic, r.cost_known, i(r.cost_micros),
                r.currency, i(r.latency_ms), r.attempt, r.status,
                r.error_code.as_deref().map(capia_secrets::redact_registered_global), r.pricing_date, i(r.timestamp_ms)
            ],
        )?;
        Ok(())
    }

    pub fn usage_summary(&self, task_id: Option<&str>) -> StoreResult<UsageSummary> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT status, cost_known, cost_micros, currency, input_tokens, output_tokens FROM ai_usage \
             WHERE (?1 IS NULL OR task_id = ?1)",
        )?;
        let mut sum = UsageSummary::default();
        let rows = st.query_map([task_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        for row in rows {
            let (status, known, micros, cur, tin, tout) = row?;
            match status.as_str() {
                "ok" => {
                    sum.calls += 1;
                    sum.input_tokens += u(tin);
                    sum.output_tokens += u(tout);
                    if known == 1 {
                        sum.known_cost_micros += u(micros);
                        if sum.currency.is_none() {
                            sum.currency = cur;
                        }
                    } else {
                        sum.unknown_cost_calls += 1;
                    }
                }
                "failed" => sum.failed_calls += 1,
                "cache_hit" => sum.cache_hits += 1,
                _ => {}
            }
        }
        Ok(sum)
    }

    pub fn usage_for_task(&self, task_id: &str) -> StoreResult<Vec<UsageRow>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT request_id, task_id, provider_id, endpoint_id, model_id, capability, purpose, input_tokens, output_tokens, \
                cached_tokens, synthetic, cost_known, cost_micros, currency, latency_ms, attempt, status, error_code, pricing_date, timestamp_ms \
             FROM ai_usage WHERE task_id = ?1 ORDER BY seq",
        )?;
        let rows = st
            .query_map([task_id], |r| {
                Ok(UsageRow {
                    request_id: r.get(0)?,
                    task_id: r.get(1)?,
                    provider_id: r.get(2)?,
                    endpoint_id: r.get(3)?,
                    model_id: r.get(4)?,
                    capability: r.get(5)?,
                    purpose: r.get(6)?,
                    input_tokens: u(r.get(7)?),
                    output_tokens: u(r.get(8)?),
                    cached_tokens: u(r.get(9)?),
                    synthetic: r.get::<_, i64>(10)? == 1,
                    cost_known: r.get::<_, i64>(11)? == 1,
                    cost_micros: u(r.get(12)?),
                    currency: r.get(13)?,
                    latency_ms: u(r.get(14)?),
                    attempt: r.get(15)?,
                    status: r.get(16)?,
                    error_code: r.get(17)?,
                    pricing_date: r.get(18)?,
                    timestamp_ms: u(r.get(19)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
