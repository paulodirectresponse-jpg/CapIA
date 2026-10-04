//! Read-model do editor: snapshot do projeto (sem o conteúdo das sequences, carregado sob demanda)
//! e patches incrementais derivados das **ops primitivas** das entradas de histórico — a UI aplica
//! `new` de cada op na sua réplica (docs/ARCHITECTURE.md §7); nunca reserializa o projeto inteiro.

use capia_commands::{Engine, HistoryEntry};
use capia_model::{Document, SequenceId};
use capia_project::{AssetView, Project};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Linha de asset para a UI (biblioteca): documento ∪ catálogo, já achatada.
pub(crate) fn asset_rows(assets: &[AssetView]) -> Value {
    let rows: Vec<Value> = assets
        .iter()
        .map(|a| {
            let video = a.catalog.as_ref().and_then(|c| c.media.video());
            let status = a.catalog.as_ref().map_or("online", |c| c.status.as_str());
            json!({
                "id": a.asset_id,
                "in_document": a.in_document,
                "name": a.catalog.as_ref().map(|c| c.display_name.clone())
                    .or_else(|| a.document.as_ref().map(|d| d.name.clone()))
                    .unwrap_or_default(),
                "kind": a.catalog.as_ref().map(|c| c.kind.as_str()),
                "duration": a.document.as_ref().and_then(|d| d.duration)
                    .or_else(|| a.catalog.as_ref().and_then(|c| c.media.duration)),
                "has_video": a.document.as_ref().map(|d| d.has_video),
                "has_audio": a.document.as_ref().map(|d| d.has_audio),
                "width": video.map(|v| v.width),
                "height": video.map(|v| v.height),
                "size_bytes": a.catalog.as_ref().map(|c| c.size_bytes),
                "status": status,
                "path": a.resolved_path.clone().or_else(|| a.catalog.as_ref().map(|c| c.location.path.clone())),
                "has_file": a.catalog.is_some(),
            })
        })
        .collect();
    Value::Array(rows)
}

fn sequence_summary(
    doc: &Document,
    id: &SequenceId,
    usage: &BTreeMap<&SequenceId, usize>,
) -> Value {
    let Some(seq) = doc.sequence(id) else {
        return Value::Null;
    };
    json!({
        "id": id,
        "name": seq.header.name,
        "frame_rate": seq.header.frame_rate,
        "sample_rate": seq.header.sample_rate,
        "width": seq.header.width,
        "height": seq.header.height,
        "folder": seq.header.folder,
        "frame_ticks": seq.frame_rate().frame_duration(),
        "duration": seq.duration(),
        "clip_count": seq.clip_count(),
        "nested_usage": usage.get(id).copied().unwrap_or(0),
    })
}

/// Quantas vezes cada sequence é usada como nested por outra(s).
fn nested_usage(doc: &Document) -> BTreeMap<&SequenceId, usize> {
    let mut usage: BTreeMap<&SequenceId, usize> = BTreeMap::new();
    for (_, seq) in doc.sequences() {
        for (_, n) in seq.nested_refs() {
            if let Some((k, _)) = doc.sequences().find(|(k, _)| **k == n.target) {
                *usage.entry(k).or_default() += 1;
            }
        }
    }
    usage
}

pub(crate) fn engine_flags(engine: &Engine) -> Value {
    json!({
        "revision": engine.revision(),
        "can_undo": engine.can_undo(),
        "can_redo": engine.can_redo(),
    })
}

/// Snapshot completo (abrir projeto / ressincronizar).
pub(crate) fn snapshot(project: &Project, assets: &[AssetView]) -> Value {
    let doc = project.document();
    let usage = nested_usage(doc);
    let sequences: Vec<Value> = doc
        .sequences()
        .map(|(id, _)| sequence_summary(doc, id, &usage))
        .collect();
    let folders: Vec<Value> = doc
        .folders()
        .map(|f| serde_json::to_value(f).unwrap_or(Value::Null))
        .collect();
    let deliverables: Vec<Value> = doc
        .deliverables()
        .map(|d| serde_json::to_value(d).unwrap_or(Value::Null))
        .collect();
    let mut v = engine_flags(project.engine());
    v["project"] = json!({
        "path": project.path().display().to_string(),
        "name": project.path().file_name().map(|n| n.to_string_lossy().into_owned()),
    });
    v["sequences"] = Value::Array(sequences);
    v["folders"] = Value::Array(folders);
    v["deliverables"] = Value::Array(deliverables);
    v["assets"] = asset_rows(assets);
    v
}

/// Conteúdo de uma sequence (tracks, clips, marcadores) — formato do `Serialize` do modelo.
pub(crate) fn sequence_model(project: &Project, id: &SequenceId) -> Option<Value> {
    let seq = project.document().sequence(id)?;
    let mut v = serde_json::to_value(seq).ok()?;
    v["frame_ticks"] = json!(seq.frame_rate().frame_duration());
    Some(v)
}

fn entry_ops(e: &HistoryEntry, inverse: bool) -> Vec<Value> {
    let ops = if inverse { &e.inverse_ops } else { &e.ops };
    ops.iter()
        .map(|o| serde_json::to_value(o).unwrap_or(Value::Null))
        .collect()
}

/// Patches (ops) + resumos de sequences afetadas + flags, para as entradas dadas.
/// `direction`: `Commit`/`Redo` aplicam `ops`; `Undo` aplica `inverse_ops`.
pub(crate) fn change_set(project: &Project, entries: &[(&HistoryEntry, bool)]) -> Value {
    let doc = project.document();
    let usage = nested_usage(doc);
    let mut patches = Vec::new();
    let mut touched: std::collections::BTreeSet<SequenceId> = std::collections::BTreeSet::new();
    for (e, inverse) in entries {
        patches.extend(entry_ops(e, *inverse));
        for op in if *inverse { &e.inverse_ops } else { &e.ops } {
            if let Some(s) = op.sequence_id() {
                touched.insert(s.clone());
            }
        }
    }
    // resumos atualizados (nome, duração, contagem de usos) das sequences tocadas e das que
    // passaram a ser/deixaram de ser alvo de nested
    let mut summaries: Vec<Value> = touched
        .iter()
        .map(|id| sequence_summary(doc, id, &usage))
        .collect();
    summaries.sort_by_key(|v| v["id"].as_str().map(str::to_owned));
    let mut v = engine_flags(project.engine());
    v["patches"] = Value::Array(patches);
    v["sequence_summaries"] = Value::Array(summaries);
    v["touched_sequences"] = json!(touched);
    v
}

/// Histórico visível: entradas aplicadas + as desfeitas (redo), com o cursor.
pub(crate) fn history(project: &Project) -> Value {
    let engine = project.engine();
    let applied = engine.applied_history().len();
    let entries: Vec<Value> = engine
        .history()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            json!({
                "id": e.id,
                "label": e.label,
                "actor": e.actor,
                "timestamp_ms": e.timestamp_ms,
                "revision": e.revision_after,
                "commands": e.commands,
                "applied": i < applied,
            })
        })
        .collect();
    json!({ "entries": entries, "cursor": applied })
}
