//! Carga do estado: último snapshot + replay dos eventos seguintes + verificações de consistência.
//! Toda leitura inválida vira `PROJECT_CORRUPTED` estruturado — nunca `panic`.

use crate::error::{StoreError, StoreResult};
use capia_commands::hash::{canonical_json, sha256_hex};
use capia_commands::{
    Actor, AppliedOperation, AuditEvent, AuditKind, CommitResult, EngineState, HistoryEntry,
};
use capia_model::{Document, EntityRef, validate_document};
use rusqlite::{Connection, OptionalExtension as _};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};

/// Espelho em memória do que o banco contém (para checar o *head* e montar snapshots).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mirror {
    pub head_seq: i64,
    pub head_revision: u64,
    pub history_ids: Vec<u64>,
    pub cursor: usize,
    pub events_since_snapshot: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoreStats {
    pub history_entries: u64,
    pub events: u64,
    pub operations: u64,
    pub snapshot_seq: i64,
    pub snapshot_revision: u64,
    pub events_since_snapshot: u64,
    pub undo_depth: usize,
    pub redo_depth: usize,
}

pub(crate) struct Loaded {
    pub state: EngineState,
    pub mirror: Mirror,
    pub stats: StoreStats,
}

fn json<T: DeserializeOwned>(what: &str, text: &str) -> StoreResult<T> {
    serde_json::from_str(text).map_err(|e| {
        StoreError::corrupted(format!("invalid {what} in the project file")).with_cause(e)
    })
}

fn int(what: &str, v: i64) -> StoreResult<u64> {
    u64::try_from(v)
        .map_err(|_| StoreError::corrupted(format!("negative {what} in the project file")))
}

pub(crate) fn digest_of(doc: &Document) -> String {
    capia_commands::document_digest(doc)
}

pub(crate) fn document_text(doc: &Document) -> String {
    canonical_json(doc)
}

fn load_entry(conn: &Connection, id: u64) -> StoreResult<HistoryEntry> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT entry_json, entry_sha256 FROM history_entries WHERE id = ?1",
            [id_i64(id)?],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (text, sha) =
        row.ok_or_else(|| StoreError::corrupted(format!("history entry {id} is missing")))?;
    if sha256_hex(text.as_bytes()) != sha {
        return Err(StoreError::corrupted(format!(
            "history entry {id} fails its checksum"
        )));
    }
    let entry: HistoryEntry = json("history entry", &text)?;
    if entry.id != id {
        return Err(StoreError::corrupted(format!(
            "history entry {id} stores id {}",
            entry.id
        )));
    }
    Ok(entry)
}

fn id_i64(id: u64) -> StoreResult<i64> {
    i64::try_from(id).map_err(|_| StoreError::corrupted("id out of range"))
}

struct EventRow {
    seq: i64,
    kind: AuditKind,
    entry_id: u64,
    revision: u64,
    timestamp_ms: u64,
    actor: Actor,
}

fn kind_of(s: &str) -> StoreResult<AuditKind> {
    match s {
        "commit" => Ok(AuditKind::Commit),
        "undo" => Ok(AuditKind::Undo),
        "redo" => Ok(AuditKind::Redo),
        other => Err(StoreError::corrupted(format!("unknown event kind {other}"))),
    }
}

pub(crate) fn kind_str(k: AuditKind) -> &'static str {
    match k {
        AuditKind::Commit => "commit",
        AuditKind::Undo => "undo",
        AuditKind::Redo => "redo",
    }
}

fn read_events(conn: &Connection, after_seq: Option<i64>) -> StoreResult<Vec<EventRow>> {
    let mut stmt = conn.prepare(
        "SELECT seq, kind, entry_id, revision_after, timestamp_ms, actor_json FROM events WHERE seq > ?1 ORDER BY seq",
    )?;
    let mut rows = stmt.query([after_seq.unwrap_or(-1)])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(EventRow {
            seq: r.get(0)?,
            kind: kind_of(&r.get::<_, String>(1)?)?,
            entry_id: int("entry id", r.get(2)?)?,
            revision: int("revision", r.get(3)?)?,
            timestamp_ms: int("timestamp", r.get(4)?)?,
            actor: json("event actor", &r.get::<_, String>(5)?)?,
        });
    }
    Ok(out)
}

/// Lê todo o estado dentro de **uma** transação de leitura: em WAL isso dá um snapshot consistente
/// mesmo com outro escritor gravando em paralelo (sem isso, snapshot e eventos podiam ser de
/// instantes diferentes).
pub(crate) fn load(conn: &Connection) -> StoreResult<Loaded> {
    let tx = conn.unchecked_transaction()?;
    let loaded = load_inner(&tx)?;
    tx.commit()?;
    Ok(loaded)
}

fn load_inner(conn: &Connection) -> StoreResult<Loaded> {
    // ---- snapshot ----------------------------------------------------------------------------
    let snap = conn
        .query_row(
            "SELECT seq, revision, cursor, history_ids_json, document_json, digest FROM snapshots ORDER BY seq DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((snap_seq, snap_rev, snap_cursor, ids_json, doc_json, digest)) = snap else {
        return Err(StoreError::corrupted("the project has no snapshot"));
    };
    let mut doc: Document = json("document snapshot", &doc_json)?;
    if digest_of(&doc) != digest {
        return Err(StoreError::corrupted(
            "the document snapshot does not match its digest",
        ));
    }
    if doc.revision != int("snapshot revision", snap_rev)? {
        return Err(StoreError::corrupted(
            "snapshot revision does not match the document",
        ));
    }
    let mut history_ids: Vec<u64> = json("history ids", &ids_json)?;
    let mut cursor =
        usize::try_from(snap_cursor).map_err(|_| StoreError::corrupted("negative cursor"))?;
    if cursor > history_ids.len() {
        return Err(StoreError::corrupted("snapshot cursor beyond history"));
    }

    // ---- replay dos eventos posteriores ----------------------------------------------------
    let tail = read_events(conn, Some(snap_seq))?;
    let mut cache: BTreeMap<u64, HistoryEntry> = BTreeMap::new();
    let mut prev_seq = snap_seq;
    for ev in &tail {
        if ev.seq != prev_seq + 1 {
            return Err(StoreError::corrupted(format!(
                "gap in events after seq {prev_seq}"
            )));
        }
        prev_seq = ev.seq;
        if ev.revision != doc.revision + 1 {
            return Err(StoreError::corrupted(format!(
                "event {} jumps from revision {} to {}",
                ev.seq, doc.revision, ev.revision
            )));
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = cache.entry(ev.entry_id) {
            slot.insert(load_entry(conn, ev.entry_id)?);
        }
        let entry = &cache[&ev.entry_id];
        let replay = |ops: &[capia_model::PrimitiveOp], doc: &mut Document| {
            doc.apply_ops(ops).map_err(|e| {
                StoreError::corrupted(format!(
                    "event {} does not replay onto the document",
                    ev.seq
                ))
                .with_cause(e)
            })
        };
        match ev.kind {
            AuditKind::Commit => {
                if entry.revision_before != doc.revision || entry.revision_after != ev.revision {
                    return Err(StoreError::corrupted(format!(
                        "commit event {} disagrees with its entry",
                        ev.seq
                    )));
                }
                replay(&entry.ops, &mut doc)?;
                history_ids.truncate(cursor);
                history_ids.push(ev.entry_id);
                cursor = history_ids.len();
            }
            AuditKind::Undo => {
                if cursor == 0 || history_ids[cursor - 1] != ev.entry_id {
                    return Err(StoreError::corrupted(format!(
                        "undo event {} does not match the history stack",
                        ev.seq
                    )));
                }
                replay(&entry.inverse_ops, &mut doc)?;
                cursor -= 1;
            }
            AuditKind::Redo => {
                if cursor >= history_ids.len() || history_ids[cursor] != ev.entry_id {
                    return Err(StoreError::corrupted(format!(
                        "redo event {} does not match the history stack",
                        ev.seq
                    )));
                }
                replay(&entry.ops, &mut doc)?;
                cursor += 1;
            }
        }
        doc.revision = ev.revision;
    }
    if let Some(v) = validate_document(&doc).first() {
        return Err(StoreError::corrupted(format!(
            "the stored document violates an invariant: {}",
            v.message
        ))
        .with_details(serde_json::json!({ "violation": v })));
    }

    // ---- demais estruturas do engine -------------------------------------------------------
    let all_events = read_events(conn, None)?;
    let mut affected_by_entry: BTreeMap<u64, BTreeSet<EntityRef>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, affected_json FROM history_entries")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            affected_by_entry.insert(
                int("entry id", r.get(0)?)?,
                json("affected set", &r.get::<_, String>(1)?)?,
            );
        }
    }
    let mut audit = Vec::with_capacity(all_events.len());
    let mut change_log = Vec::with_capacity(all_events.len());
    for (expected_seq, ev) in (1..).zip(all_events.iter()) {
        if ev.seq != expected_seq {
            return Err(StoreError::corrupted(format!(
                "event sequence is not contiguous at {}",
                ev.seq
            )));
        }
        let affected = affected_by_entry.get(&ev.entry_id).ok_or_else(|| {
            StoreError::corrupted(format!("event {} references a missing entry", ev.seq))
        })?;
        change_log.push((ev.revision, affected.clone()));
        audit.push(AuditEvent {
            kind: ev.kind,
            entry_id: ev.entry_id,
            revision: ev.revision,
            timestamp_ms: ev.timestamp_ms,
            actor: ev.actor.clone(),
        });
    }
    let head_seq = all_events.last().map_or(snap_seq, |e| e.seq);
    if head_seq != prev_seq {
        return Err(StoreError::corrupted(
            "events do not extend the latest snapshot",
        ));
    }

    let mut history = Vec::with_capacity(history_ids.len());
    for id in &history_ids {
        history.push(match cache.get(id) {
            Some(e) => e.clone(),
            None => load_entry(conn, *id)?,
        });
    }

    let mut applied = BTreeMap::new();
    {
        let mut stmt =
            conn.prepare("SELECT operation_id, payload_hash, entry_id, applied_at_ms, actor_json FROM applied_operations")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            applied.insert(
                r.get::<_, String>(0)?,
                AppliedOperation {
                    payload_hash: r.get(1)?,
                    history_entry_id: int("entry id", r.get(2)?)?,
                    applied_at_ms: int("timestamp", r.get(3)?)?,
                    actor: json("operation actor", &r.get::<_, String>(4)?)?,
                },
            );
        }
    }
    let mut commit_results: BTreeMap<u64, CommitResult> = BTreeMap::new();
    {
        let mut stmt =
            conn.prepare("SELECT entry_id, result_json, result_sha256 FROM commit_results")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let id = int("entry id", r.get(0)?)?;
            let (text, sha) = (r.get::<_, String>(1)?, r.get::<_, String>(2)?);
            if sha256_hex(text.as_bytes()) != sha {
                return Err(StoreError::corrupted(format!(
                    "commit result of entry {id} fails its checksum"
                )));
            }
            commit_results.insert(id, json("commit result", &text)?);
        }
    }
    // o log de operações, os resultados e as entradas de histórico precisam se corresponder
    for (op_id, op) in &applied {
        if !commit_results.contains_key(&op.history_entry_id)
            || !affected_by_entry.contains_key(&op.history_entry_id)
        {
            return Err(StoreError::corrupted(format!(
                "operation {op_id} points to entry {} which has no stored result",
                op.history_entry_id
            )));
        }
    }
    for id in affected_by_entry.keys() {
        if !commit_results.contains_key(id) {
            return Err(StoreError::corrupted(format!(
                "history entry {id} has no stored result"
            )));
        }
    }
    if commit_results.len() != affected_by_entry.len() {
        return Err(StoreError::corrupted(
            "commit results and history entries do not match",
        ));
    }
    let max_entry: Option<i64> =
        conn.query_row("SELECT MAX(id) FROM history_entries", [], |r| r.get(0))?;
    let next_entry = match max_entry {
        Some(m) => int("entry id", m)?
            .checked_add(1)
            .ok_or_else(|| StoreError::corrupted("entry id overflow"))?,
        None => 1,
    };

    let stats = StoreStats {
        history_entries: u64::try_from(affected_by_entry.len()).unwrap_or(u64::MAX),
        events: u64::try_from(all_events.len()).unwrap_or(u64::MAX),
        operations: u64::try_from(applied.len()).unwrap_or(u64::MAX),
        snapshot_seq: snap_seq,
        snapshot_revision: int("snapshot revision", snap_rev)?,
        events_since_snapshot: u64::try_from(tail.len()).unwrap_or(u64::MAX),
        undo_depth: cursor,
        redo_depth: history_ids.len() - cursor,
    };
    let mirror = Mirror {
        head_seq,
        head_revision: doc.revision,
        history_ids,
        cursor,
        events_since_snapshot: stats.events_since_snapshot,
    };
    Ok(Loaded {
        state: EngineState {
            doc,
            history,
            cursor,
            applied,
            commit_results,
            change_log,
            audit,
            next_entry,
        },
        mirror,
        stats,
    })
}
