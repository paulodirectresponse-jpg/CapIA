//! Executores das tools do assistente. Cada tool lê/escreve o projeto **só** pela fachada
//! [`crate::engine::Engine`] (leitura por lista fechada; escrita por `preview → apply_plan` com ator
//! `Agent`). Entradas já foram validadas contra o schema da tool; aqui valida-se o que o schema não
//! enxerga (ids existentes, comandos permitidos, token de plano pertencente a esta tarefa).

use crate::captions::{self, CaptionOptions};
use crate::ctx::IntelCtx;
use crate::error::IntelError;
use crate::records::{KIND_DEMAND, KIND_REFERENCE, KIND_TRANSCRIPT};
use crate::reference::{self, ReferenceOptions};
use crate::scenes::{self, SceneParams};
use crate::silence::{self, SilenceParams};
use crate::transcript::{
    TranscribeParams, TranscriptRecord, asset_file, ticks_from_us, transcribe_asset, us_from_ticks,
};
use capia_ai::dispatcher::TaskCtx;
use capia_ai::tools::{ToolError, ToolErrorCode, operation_id};
use capia_ai::types::Part;
use capia_time::{TICKS_PER_SECOND, Ticks};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

/// Comandos que o assistente pode propor. Fora desta lista (registrar/remover assets, apagar
/// sequences/pastas, entregáveis, variantes) o preview é recusado — nunca há "best effort".
pub const WRITABLE_COMMANDS: &[&str] = &[
    "add_track",
    "set_track_flags",
    "delete_track",
    "add_marker",
    "move_marker",
    "delete_marker",
    "insert_clip",
    "move_clips",
    "delete_clip",
    "trim_clip",
    "split_clip",
    "set_clip_speed",
    "reorder_clip",
    "rename_clip",
    "set_clip_enabled",
    "set_text",
    "group_clips",
    "ungroup",
    "set_transition",
    "detach_audio",
    "set_property",
    "add_keyframe",
    "move_keyframe",
    "delete_keyframe",
    "set_keyframe_interp",
];

/// Tools que este executor sabe rodar (o resto do catálogo nunca é oferecido ao modelo).
pub const SUPPORTED: &[&str] = &[
    "project.read_brief",
    "project.list_sequences",
    "timeline.get_state",
    "timeline.query_clips",
    "timeline.preview",
    "timeline.apply_plan",
    "assets.search",
    "assets.get",
    "assets.get_transcript",
    "media.analyze",
    "media.sample_frames",
    "media.transcribe",
    "media.detect_scenes",
    "reference.analyze",
    "reference.get_grammar",
    "captions.build_commands",
    "silence.propose_cuts",
    "project.get_demand_spec",
];

#[derive(Clone, Debug, PartialEq)]
pub struct PlanInfo {
    pub label: String,
    pub op_count: u64,
    pub summary: Value,
}

/// Estado mutável de uma tarefa visível às tools.
#[derive(Debug, Default)]
pub struct TaskState {
    /// Planos pré-visualizados NESTA tarefa (token → info). Token de outra origem é recusado.
    pub plans: BTreeMap<String, PlanInfo>,
    pub applied: Vec<String>,
    /// Imagens a anexar à próxima mensagem do modelo (só quando há visão).
    pub pending_images: Vec<Part>,
    pub vision_ok: bool,
}

#[derive(Debug)]
pub struct Env<'a> {
    pub ctx: &'a IntelCtx,
    pub task: &'a TaskCtx,
    pub step: u32,
    pub state: &'a mut TaskState,
}

fn te(code: ToolErrorCode, msg: impl Into<String>) -> ToolError {
    ToolError::new(code, msg)
}

fn fail(e: IntelError) -> ToolError {
    let code = match e.code.as_str() {
        "CANCELLED" => ToolErrorCode::Failed,
        "NOT_ALLOWED" => ToolErrorCode::PermissionDenied,
        "PRIVACY_POLICY_BLOCKED" => ToolErrorCode::PrivacyBlocked,
        "BUDGET_EXCEEDED" => ToolErrorCode::BudgetExceeded,
        _ => ToolErrorCode::Failed,
    };
    te(code, format!("{}: {}", e.code, e.message))
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn secs_to_ticks(sec: f64) -> Ticks {
    Ticks((sec.max(0.0) * TICKS_PER_SECOND as f64).round() as i64)
}

fn to_s(t: i64) -> f64 {
    (t as f64 / TICKS_PER_SECOND as f64 * 1000.0).round() / 1000.0
}

fn assets(ctx: &IntelCtx) -> Result<Vec<Value>, ToolError> {
    let v = ctx.engine.read("assets.list", json!({})).map_err(fail)?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

fn sequence(ctx: &IntelCtx, id: &str) -> Result<Value, ToolError> {
    ctx.engine
        .read("sequence.get", json!({ "sequence": id }))
        .map_err(|e| {
            if e.code == "NOT_FOUND" {
                te(
                    ToolErrorCode::InvalidArguments,
                    format!("sequence `{id}` does not exist"),
                )
            } else {
                fail(e)
            }
        })
}

fn clip_row(id: &str, c: &Value, frame: i64) -> Value {
    let start = c["start"].as_i64().unwrap_or(0);
    let dur = c["duration"].as_i64().unwrap_or(0);
    let content = &c["content"];
    let kind = content["type"].as_str().unwrap_or("?");
    json!({
        "id": id,
        "track": c["track"],
        "name": c["name"],
        "kind": kind,
        "asset": content["asset"],
        "text": content["text"],
        "start_s": to_s(start),
        "end_s": to_s(start + dur),
        "start_frame": if frame > 0 { start / frame } else { 0 },
        "duration_frames": if frame > 0 { dur / frame } else { 0 },
        "enabled": c["enabled"],
    })
}

pub async fn execute(name: &str, args: &Value, env: &mut Env<'_>) -> Result<Value, ToolError> {
    match name {
        "project.read_brief" => {
            let snap = env
                .ctx
                .engine
                .read("project.snapshot", json!({}))
                .map_err(fail)?;
            let rows = assets(env.ctx)?;
            let offline = rows
                .iter()
                .filter(|a| a["status"].as_str().is_some_and(|s| s != "online"))
                .count();
            Ok(json!({
                "sequences": snap["sequences"],
                "assets": { "total": rows.len(), "offline": offline },
            }))
        }
        "project.list_sequences" => {
            let snap = env
                .ctx
                .engine
                .read("project.snapshot", json!({}))
                .map_err(fail)?;
            Ok(json!({ "sequences": snap["sequences"] }))
        }
        "timeline.get_state" => {
            let id = s(args, "sequence").unwrap_or_default();
            let seq = sequence(env.ctx, &id)?;
            let frame = seq["frame_ticks"].as_i64().unwrap_or(0);
            let detail = s(args, "detail").unwrap_or_else(|| "outline".into());
            let tracks: Vec<Value> = seq["tracks"].as_array().cloned().unwrap_or_default();
            let clips = seq["clips"].as_object().cloned().unwrap_or_default();
            let mut per_track: BTreeMap<String, u32> = BTreeMap::new();
            for c in clips.values() {
                *per_track
                    .entry(c["track"].as_str().unwrap_or("").to_owned())
                    .or_default() += 1;
            }
            let outline = json!({
                "sequence": id,
                "name": seq["header"]["name"],
                "width": seq["header"]["width"],
                "height": seq["header"]["height"],
                "frame_rate": seq["header"]["frame_rate"],
                "frame_ticks": frame,
                "ticks_per_second": TICKS_PER_SECOND,
                "clip_count": clips.len(),
                "tracks": tracks.iter().map(|t| json!({
                    "id": t["id"], "kind": t["kind"], "role": t["role"], "name": t["name"],
                    "locked": t["locked"], "clips": per_track.get(t["id"].as_str().unwrap_or("")).copied().unwrap_or(0),
                })).collect::<Vec<_>>(),
            });
            if detail == "outline" {
                return Ok(outline);
            }
            let from = args
                .get("from_s")
                .and_then(Value::as_f64)
                .map(secs_to_ticks);
            let to = args.get("to_s").and_then(Value::as_f64).map(secs_to_ticks);
            let mut rows: Vec<(i64, Value)> = clips
                .iter()
                .filter(|(_, c)| {
                    let st = c["start"].as_i64().unwrap_or(0);
                    let en = st + c["duration"].as_i64().unwrap_or(0);
                    from.is_none_or(|f| en > f.0) && to.is_none_or(|t| st < t.0)
                })
                .map(|(id, c)| (c["start"].as_i64().unwrap_or(0), clip_row(id, c, frame)))
                .collect();
            rows.sort_by_key(|(t, _)| *t);
            let page = args.get("page").and_then(Value::as_u64).unwrap_or(0) as usize;
            let size = args
                .get("page_size")
                .and_then(Value::as_u64)
                .unwrap_or(50)
                .clamp(1, 200) as usize;
            let total = rows.len();
            let slice: Vec<Value> = rows
                .into_iter()
                .skip(page * size)
                .take(size)
                .map(|(_, v)| v)
                .collect();
            Ok(json!({
                "outline": outline, "clips": slice, "page": page, "page_size": size,
                "total_clips": total, "has_more": (page + 1) * size < total,
            }))
        }
        "timeline.query_clips" => {
            let id = s(args, "sequence").unwrap_or_default();
            let seq = sequence(env.ctx, &id)?;
            let frame = seq["frame_ticks"].as_i64().unwrap_or(0);
            let track = s(args, "track");
            let text = s(args, "text").map(|t| t.to_lowercase());
            let asset = s(args, "asset");
            let from = args
                .get("from_s")
                .and_then(Value::as_f64)
                .map(secs_to_ticks);
            let to = args.get("to_s").and_then(Value::as_f64).map(secs_to_ticks);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(50)
                .clamp(1, 200) as usize;
            let mut rows: Vec<(i64, Value)> = seq["clips"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, c)| {
                    track
                        .as_deref()
                        .is_none_or(|t| c["track"].as_str() == Some(t))
                        && asset
                            .as_deref()
                            .is_none_or(|a| c["content"]["asset"].as_str() == Some(a))
                        && text.as_deref().is_none_or(|t| {
                            c["content"]["text"]
                                .as_str()
                                .is_some_and(|x| x.to_lowercase().contains(t))
                        })
                        && from.is_none_or(|f| {
                            c["start"].as_i64().unwrap_or(0) + c["duration"].as_i64().unwrap_or(0)
                                > f.0
                        })
                        && to.is_none_or(|t| c["start"].as_i64().unwrap_or(0) < t.0)
                })
                .map(|(id, c)| (c["start"].as_i64().unwrap_or(0), clip_row(id, c, frame)))
                .collect();
            rows.sort_by_key(|(t, _)| *t);
            let total = rows.len();
            Ok(json!({
                "clips": rows.into_iter().take(limit).map(|(_, v)| v).collect::<Vec<_>>(),
                "total": total,
                "truncated": total > limit,
            }))
        }
        "timeline.preview" => preview(args, env),
        "timeline.apply_plan" => {
            let token = s(args, "plan_token").unwrap_or_default();
            let Some(info) = env.state.plans.get(&token).cloned() else {
                return Err(te(
                    ToolErrorCode::InvalidArguments,
                    "unknown plan_token for this task: call timeline.preview first",
                ));
            };
            let r = env.ctx.engine.apply(&env.ctx.actor, &token).map_err(fail)?;
            env.state.applied.push(token);
            Ok(json!({
                "applied": true,
                "label": info.label,
                "operations": info.op_count,
                "revision": r["revision"],
                "undo": "the user can undo this with Ctrl+Z (one history entry)",
            }))
        }
        "assets.search" => {
            let q = s(args, "query").map(|q| q.to_lowercase());
            let kind = s(args, "kind");
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(30)
                .clamp(1, 100) as usize;
            let rows: Vec<Value> = assets(env.ctx)?
                .into_iter()
                .filter(|a| {
                    q.as_deref().is_none_or(|q| {
                        a["name"]
                            .as_str()
                            .is_some_and(|n| n.to_lowercase().contains(q))
                    }) && kind
                        .as_deref()
                        .is_none_or(|k| a["kind"].as_str() == Some(k))
                })
                .map(|a| strip_path(&a))
                .collect();
            let total = rows.len();
            Ok(json!({"assets": rows.into_iter().take(limit).collect::<Vec<_>>(), "total": total}))
        }
        "assets.get" => {
            let id = s(args, "asset").unwrap_or_default();
            assets(env.ctx)?
                .into_iter()
                .find(|a| a["id"].as_str() == Some(&id))
                .map(|a| strip_path(&a))
                .ok_or_else(|| {
                    te(
                        ToolErrorCode::InvalidArguments,
                        format!("asset `{id}` does not exist"),
                    )
                })
        }
        "assets.get_transcript" => {
            let id = s(args, "asset").unwrap_or_default();
            known_asset(env.ctx, &id)?;
            let rec = latest_transcript(env.ctx, &id).map_err(fail)?;
            let Some(rec) = rec else {
                return Ok(json!({"available": false, "hint": "call media.transcribe first"}));
            };
            let from = args
                .get("from_s")
                .and_then(Value::as_f64)
                .map_or(0, |v| (v * 1e6) as i64);
            let to = args
                .get("to_s")
                .and_then(Value::as_f64)
                .map_or(i64::MAX, |v| (v * 1e6) as i64);
            let segs: Vec<Value> = rec
                .transcript
                .segments
                .iter()
                .filter(|sg| sg.end_us > from && sg.start_us < to)
                .map(|sg| json!({"start_s": sg.start_us as f64 / 1e6, "end_s": sg.end_us as f64 / 1e6, "text": sg.text}))
                .collect();
            Ok(json!({"available": true, "language": rec.transcript.language, "segments": segs}))
        }
        "media.detect_scenes" => {
            let id = s(args, "asset").unwrap_or_default();
            let b = scan_scenes(env, &id).await?;
            Ok(
                json!({"count": b.len(), "boundaries": b.iter().map(|x| json!({
                "at_s": x.frame as f64 / 25.0, "kind": x.kind, "span_frames": x.span})).collect::<Vec<_>>()}),
            )
        }
        "media.analyze" => {
            let id = s(args, "asset").unwrap_or_default();
            let file = asset_file(env.ctx, &id).map_err(fail)?;
            let b = if file.has_video {
                scan_scenes(env, &id).await?
            } else {
                Vec::new()
            };
            let mut out = json!({
                "duration_s": to_s(file.duration.0),
                "has_video": file.has_video, "has_audio": file.has_audio,
                "scene_count": b.len(),
            });
            if file.has_audio {
                let (ctx2, id2, cancel, dur) = (
                    env.ctx.clone(),
                    id.clone(),
                    env.task.cancel.clone(),
                    file.duration,
                );
                let db = tokio::task::spawn_blocking(move || {
                    silence::measure_asset(&ctx2, &id2, Ticks::ZERO, dur, &cancel)
                })
                .await
                .map_err(|e| te(ToolErrorCode::Failed, e.to_string()))?
                .map_err(fail)?;
                let sil = silence::find_silences(&db, &SilenceParams::default());
                out["silence_regions"] = json!(
                    sil.iter()
                        .take(50)
                        .map(|r| json!({
                    "start_s": r.start_us as f64 / 1e6, "end_s": r.end_us as f64 / 1e6}))
                        .collect::<Vec<_>>()
                );
                out["silence_total_s"] =
                    json!(sil.iter().map(|r| r.end_us - r.start_us).sum::<i64>() as f64 / 1e6);
            }
            Ok(out)
        }
        "media.transcribe" => {
            let id = s(args, "asset").unwrap_or_default();
            known_asset(env.ctx, &id)?;
            let mut p = TranscribeParams::new(&id);
            p.language = s(args, "language");
            if let Some(w) = args.get("word_timestamps").and_then(Value::as_bool) {
                p.word_timestamps = w;
            }
            let rec = transcribe_asset(env.ctx, env.task, &p, &|_, _| {})
                .await
                .map_err(fail)?;
            let text = rec.transcript.text();
            Ok(json!({
                "language": rec.transcript.language,
                "segments": rec.transcript.segments.len(),
                "from_cache": rec.from_cache,
                "local": rec.local,
                "cost_micros": rec.cost_micros,
                "text": text.chars().take(4_000).collect::<String>(),
                "text_truncated": text.chars().count() > 4_000,
            }))
        }
        "reference.analyze" => {
            let id = s(args, "asset").unwrap_or_default();
            known_asset(env.ctx, &id)?;
            let (g, cached) = reference::analyze_reference(
                env.ctx,
                env.task,
                &id,
                &ReferenceOptions::default(),
                &|_, _, _| {},
            )
            .await
            .map_err(fail)?;
            Ok(json!({"cached": cached, "grammar": grammar_summary(&g)}))
        }
        "reference.get_grammar" => {
            let id = s(args, "asset").unwrap_or_default();
            known_asset(env.ctx, &id)?;
            let rows = env
                .ctx
                .records()
                .and_then(|r| Ok(r.store().list(KIND_REFERENCE, Some(&id), true, 1)?))
                .map_err(fail)?;
            match rows.into_iter().next() {
                Some(r) => {
                    let g: reference::ReferenceGrammar = serde_json::from_value(r.json)
                        .map_err(|_| te(ToolErrorCode::Failed, "stored grammar is unreadable"))?;
                    Ok(json!({"available": true, "grammar": grammar_summary(&g)}))
                }
                None => Ok(json!({"available": false, "hint": "call reference.analyze first"})),
            }
        }
        "captions.build_commands" => {
            let seq = s(args, "sequence").unwrap_or_default();
            let asset = s(args, "asset").unwrap_or_default();
            sequence(env.ctx, &seq)?;
            known_asset(env.ctx, &asset)?;
            let Some(rec) = latest_transcript(env.ctx, &asset).map_err(fail)? else {
                return Err(te(
                    ToolErrorCode::Failed,
                    "no transcript yet: call media.transcribe first",
                ));
            };
            let mut o = CaptionOptions::default();
            if let Some(m) = args.get("max_chars_per_block").and_then(Value::as_u64) {
                o.max_chars = m as usize;
            }
            let task_id = format!("{}#{}", env.task.task_id, env.step);
            let plan = captions::plan_captions(env.ctx, &rec, &seq, &o, &task_id).map_err(fail)?;
            register_plan(env, &plan.preview, "Legendas automáticas")?;
            Ok(json!({
                "cue_count": plan.cue_count,
                "plan_token": plan.plan_token,
                "next": "call timeline.apply_plan with this plan_token to create the caption clips",
            }))
        }
        "silence.propose_cuts" => {
            let seq_id = s(args, "sequence").unwrap_or_default();
            let clip_id = s(args, "clip").unwrap_or_default();
            let seq = sequence(env.ctx, &seq_id)?;
            let clip = seq["clips"].get(&clip_id).ok_or_else(|| {
                te(
                    ToolErrorCode::InvalidArguments,
                    format!("clip `{clip_id}` does not exist"),
                )
            })?;
            let asset = clip["content"]["asset"]
                .as_str()
                .ok_or_else(|| {
                    te(
                        ToolErrorCode::InvalidArguments,
                        "the clip is not a media clip",
                    )
                })?
                .to_owned();
            let mut p = SilenceParams::default();
            if let Some(v) = args.get("min_silence_ms").and_then(Value::as_i64) {
                p.min_silence_us = v * 1_000;
            }
            if let Some(v) = args.get("threshold_db").and_then(Value::as_f64) {
                p.threshold_db = v as f32;
            }
            if let Some(v) = args.get("padding_ms").and_then(Value::as_i64) {
                p.pad_us = v * 1_000;
            }
            let task_id = format!("{}#{}", env.task.task_id, env.step);
            let (ctx2, cancel) = (env.ctx.clone(), env.task.cancel.clone());
            let (seq2, asset2, clip2) = (seq_id.clone(), asset.clone(), clip_id.clone());
            let plan = tokio::task::spawn_blocking(move || {
                silence::plan_silence_cut(
                    &ctx2,
                    &asset2,
                    &seq2,
                    &p,
                    "track",
                    &task_id,
                    Some(&clip2),
                    &cancel,
                )
            })
            .await
            .map_err(|e| te(ToolErrorCode::Failed, e.to_string()))?
            .map_err(fail)?;
            register_plan(env, &plan.preview, "Remover silêncios")?;
            Ok(json!({
                "cuts": plan.cut_count,
                "removed_s": plan.removed_us as f64 / 1e6,
                "plan_token": plan.plan_token,
                "next": "call timeline.apply_plan with this plan_token to ripple-delete the silences",
            }))
        }
        "media.sample_frames" => sample_frames(args, env),
        "project.get_demand_spec" => {
            let rows = env
                .ctx
                .records()
                .and_then(|r| Ok(r.store().list(KIND_DEMAND, None, true, 1)?))
                .map_err(fail)?;
            Ok(match rows.into_iter().next() {
                Some(r) => json!({"available": true, "demand_spec": r.json}),
                None => json!({"available": false}),
            })
        }
        other => Err(te(
            ToolErrorCode::UnknownTool,
            format!("tool `{other}` has no executor"),
        )),
    }
}

fn strip_path(a: &Value) -> Value {
    // o caminho do arquivo é dado do usuário e não serve ao modelo
    let mut v = a.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("path");
    }
    v
}

fn known_asset(ctx: &IntelCtx, id: &str) -> Result<(), ToolError> {
    if assets(ctx)?.iter().any(|a| a["id"].as_str() == Some(id)) {
        Ok(())
    } else {
        Err(te(
            ToolErrorCode::InvalidArguments,
            format!("asset `{id}` does not exist in this project"),
        ))
    }
}

fn latest_transcript(ctx: &IntelCtx, asset: &str) -> Result<Option<TranscriptRecord>, IntelError> {
    let rows = ctx
        .records()?
        .store()
        .list(KIND_TRANSCRIPT, Some(asset), true, 1)?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|r| serde_json::from_value(r.json).ok()))
}

async fn scan_scenes(env: &Env<'_>, asset: &str) -> Result<Vec<scenes::Boundary>, ToolError> {
    let file = asset_file(env.ctx, asset).map_err(fail)?;
    if !file.has_video {
        return Err(te(ToolErrorCode::Failed, "the asset has no video"));
    }
    let tc = env
        .ctx
        .engine
        .toolchain()
        .ok_or_else(|| te(ToolErrorCode::Failed, "ffmpeg is not available"))?;
    let cancel = env.task.cancel.clone();
    tokio::task::spawn_blocking(move || {
        scenes::detect_media(
            &tc,
            &file.path,
            0,
            (25, 1),
            &SceneParams::default(),
            Duration::from_secs(1800),
            &move || cancel.is_cancelled(),
        )
    })
    .await
    .map_err(|e| te(ToolErrorCode::Failed, e.to_string()))?
    .map_err(|e| fail(e.into()))
}

fn grammar_summary(g: &reference::ReferenceGrammar) -> Value {
    json!({
        "duration_s": g.duration_us as f64 / 1e6,
        "shots": g.cut_rhythm.shot_count,
        "cuts_per_minute": g.cut_rhythm.cuts_per_minute,
        "median_shot_s": g.cut_rhythm.median_shot_us as f64 / 1e6,
        "transitions": g.transitions,
        "structure": g.structure,
        "audio": g.audio.as_ref().map(|a| json!({
            "speech_ratio_permille": a.speech_ratio_permille, "mean_db": a.mean_db,
            "dynamic_range_db": a.dynamic_range_db})),
        "speech": g.speech.as_ref().map(|sp| json!({
            "language": sp.language, "words_per_minute": sp.words_per_minute,
            "hook_text": sp.hook_text})),
        "notes": g.provenance.notes,
    })
}

fn register_plan(env: &mut Env<'_>, preview: &Value, label: &str) -> Result<(), ToolError> {
    if let Some(tok) = preview["plan_token"].as_str() {
        env.state.plans.insert(
            tok.to_owned(),
            PlanInfo {
                label: label.to_owned(),
                op_count: preview["op_count"].as_u64().unwrap_or(0),
                summary: json!({"base_revision": preview["base_revision"], "refs": preview["refs"]}),
            },
        );
    }
    Ok(())
}

fn preview(args: &Value, env: &mut Env<'_>) -> Result<Value, ToolError> {
    let label = s(args, "label").unwrap_or_else(|| "Edição do assistente".into());
    let cmds = args
        .get("commands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            te(
                ToolErrorCode::InvalidArguments,
                "`commands` must be an array",
            )
        })?;
    let mut out: Vec<Value> = Vec::with_capacity(cmds.len());
    for (i, c) in cmds.iter().enumerate() {
        let ty = c.get("type").and_then(Value::as_str).unwrap_or("");
        if !WRITABLE_COMMANDS.contains(&ty) {
            return Err(te(
                ToolErrorCode::PermissionDenied,
                format!("command `{ty}` is not available to the assistant"),
            ));
        }
        let mut c = c.clone();
        // o `operation_id` NUNCA vem do modelo: é derivado de tarefa+passo+índice (retry não duplica)
        c["operation_id"] = json!(operation_id(
            &env.task.task_id,
            env.step,
            u32::try_from(i).unwrap_or(u32::MAX)
        ));
        out.push(c);
    }
    let pv = env
        .ctx
        .engine
        .preview(&env.ctx.actor, &label, Value::Array(out))
        .map_err(|e| {
            te(
                ToolErrorCode::Failed,
                format!("the engine rejected the plan ({}): {}", e.code, e.message),
            )
        })?;
    register_plan(env, &pv, &label)?;
    let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
    for c in cmds {
        *kinds
            .entry(c.get("type").and_then(Value::as_str).unwrap_or("?"))
            .or_default() += 1;
    }
    Ok(json!({
        "plan_token": pv["plan_token"],
        "already_applied": pv["already_applied"],
        "operations": pv["op_count"],
        "commands": kinds,
        "base_revision": pv["base_revision"],
        "next": "call timeline.apply_plan with the plan_token (the user may need to approve)",
    }))
}

fn sample_frames(args: &Value, env: &mut Env<'_>) -> Result<Value, ToolError> {
    if !env.state.vision_ok {
        return Err(te(
            ToolErrorCode::PrivacyBlocked,
            "no vision-capable model is available (or privacy forbids sending frames)",
        ));
    }
    let id = s(args, "asset").unwrap_or_default();
    let file = asset_file(env.ctx, &id).map_err(fail)?;
    if !file.has_video {
        return Err(te(ToolErrorCode::Failed, "the asset has no video"));
    }
    let tc = env
        .ctx
        .engine
        .toolchain()
        .ok_or_else(|| te(ToolErrorCode::Failed, "ffmpeg is not available"))?;
    let count = args
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or(4)
        .clamp(1, 16);
    let width = args
        .get("width")
        .and_then(Value::as_u64)
        .unwrap_or(384)
        .clamp(64, 512) as u32;
    let total = us_from_ticks(file.duration);
    let from = args
        .get("from_s")
        .and_then(Value::as_f64)
        .map_or(0, |v| (v * 1e6) as i64)
        .clamp(0, total);
    let to = args
        .get("to_s")
        .and_then(Value::as_f64)
        .map_or(total, |v| (v * 1e6) as i64)
        .clamp(from, total);
    let mut times = Vec::new();
    for k in 0..count {
        // pontos centrais de `count` fatias iguais: determinístico
        let t = from + (to - from) * (2 * k as i64 + 1) / (2 * count as i64);
        let png = capia_media::extract_frame_png(
            &tc,
            &file.path,
            capia_media::ThumbnailRequest {
                at: ticks_from_us(t),
                max_dim: width,
            },
        )
        .map_err(|e| fail(e.into()))?;
        env.state.pending_images.push(Part::Image {
            mime: "image/png".into(),
            data_b64: base64_encode(&png),
        });
        times.push(t as f64 / 1e6);
    }
    Ok(
        json!({"frames": count, "at_s": times, "note": "the frames are attached to the next message"}),
    )
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(b)
}
