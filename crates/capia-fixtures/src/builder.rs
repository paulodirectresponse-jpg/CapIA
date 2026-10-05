//! Construtor do projeto grande. Escreve **somente** por `Project::execute` (Command Engine), pelo
//! `Catalog` público (registros sintéticos de mídia, `offline`) e pelo `AutonomyStore` público
//! (histórico de AI Runs). Nenhum arquivo de mídia é gerado: o catálogo aceita registros cujo
//! caminho não existe (disponibilidade `offline`) — o que é exatamente o estado de uma biblioteca
//! com discos desmontados e custa o mesmo para abrir/listar.

use crate::FRAME;
use crate::rng::Rng;
use crate::spec::{FixtureError, LargeSpec, LargeStats};
use capia_assets::testing::synthetic_info;
use capia_assets::{AssetKind, AssetLocation, AssetRecord, Availability, ContentHash};
use capia_commands::{Actor, Command, CommandEnvelope, NewClip, Transaction, document_digest};
use capia_media::{MediaConfig, MediaKind, MediaToolchain, ToolSource};
use capia_model::{
    Asset, AssetId, ClipContent, Document, SequenceId, TextStyle, TrackKind, TrackRole,
};
use capia_project::Project;
use capia_store::{
    AutonomyStore, Catalog, CatalogEventKind, CatalogOp, RunUpdate, StageRow, StoreOptions,
    Synchronous,
};
use capia_time::{FrameRate, Rational, TICKS_PER_SECOND, Ticks};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Um `MediaToolchain` que **nunca** é executado: serve a `RenderServices` quando o quadro só tem
/// camadas sólidas/texto/nested ou fontes offline (o preview não decodifica nada).
pub fn offline_toolchain() -> MediaToolchain {
    // `MediaConfig` não é usado; o tipo existe só para o import não ficar solto em builds sem uso
    let _ = MediaConfig::default();
    MediaToolchain {
        ffprobe: PathBuf::from("ffprobe-not-used-by-fixtures"),
        ffprobe_source: ToolSource::Configured,
        ffmpeg: None,
        version: "fixtures: no ffmpeg".to_owned(),
        probe_timeout: Duration::from_secs(1),
    }
}

fn ferr(what: &str, e: impl core::fmt::Debug) -> FixtureError {
    FixtureError::of(what, e)
}

struct AssetPlan {
    id: AssetId,
    kind: AssetKind,
    seconds: i64,
}

fn asset_plan(spec: &LargeSpec) -> Vec<AssetPlan> {
    (0..spec.assets)
        .map(|i| {
            let (kind, seconds) = match i % 5 {
                3 => (AssetKind::Audio, 20 + (i as i64 * 11) % 280),
                4 => (AssetKind::Image, 0),
                _ => (AssetKind::Video, 20 + (i as i64 * 7) % 100),
            };
            AssetPlan {
                id: AssetId::new(format!("ast_{:032x}", i as u128 + 1)),
                kind,
                seconds,
            }
        })
        .collect()
}

struct Batcher {
    project: Project,
    actor: Actor,
    ops: Vec<CommandEnvelope>,
    batch: usize,
    counter: u64,
    label: &'static str,
}

impl Batcher {
    fn push(&mut self, command: Command) -> Result<(), FixtureError> {
        self.counter += 1;
        self.ops.push(CommandEnvelope {
            operation_id: format!("lg-{}", self.counter),
            reference: None,
            command,
        });
        if self.ops.len() >= self.batch {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), FixtureError> {
        if self.ops.is_empty() {
            return Ok(());
        }
        let commands = std::mem::take(&mut self.ops);
        let tx = Transaction {
            transaction_id: None,
            label: self.label.to_owned(),
            base_revision: None,
            commands,
            max_ops: Some(100_000),
        };
        self.project
            .execute(&self.actor, tx)
            .map(|_| ())
            .map_err(|e| ferr("command failed", e))
    }
}

#[derive(Default)]
struct Counts {
    nested: usize,
    text: usize,
    captions: usize,
    audio: usize,
    media: usize,
    keyframed: usize,
    markers: usize,
}

const WORDS: [&str; 16] = [
    "agora",
    "descubra",
    "oferta",
    "limitada",
    "resultado",
    "garantido",
    "clique",
    "aqui",
    "hoje",
    "sem",
    "risco",
    "transforme",
    "seu",
    "negócio",
    "veja",
    "como",
];

fn words(rng: &mut Rng, n: u64) -> String {
    (0..n)
        .map(|_| *rng.pick(&WORDS))
        .collect::<Vec<_>>()
        .join(" ")
}

fn new_clip(
    id: String,
    name: String,
    frames: i64,
    content: ClipContent,
    source_in_frames: i64,
) -> NewClip {
    NewClip {
        id: Some(id.as_str().into()),
        name,
        duration: Ticks(frames * FRAME),
        content,
        source_in: Ticks(source_in_frames * FRAME),
        speed: Rational::ONE,
        reversed: false,
        properties: Default::default(),
    }
}

/// Quantos clips cada sequence recebe: a main fica com 30 %, o resto com pesos determinísticos.
fn allocate(spec: &LargeSpec, rng: &mut Rng) -> Vec<usize> {
    let main = spec.clips * 30 / 100;
    let rest = spec.clips - main;
    let others = spec.sequences - 1;
    let weights: Vec<u64> = (0..others).map(|_| rng.range(60, 140)).collect();
    let total: u64 = weights.iter().sum();
    let mut out = vec![main];
    let mut used = 0usize;
    for (i, w) in weights.iter().enumerate() {
        let n = if i + 1 == others {
            rest - used
        } else {
            (rest as u64 * w / total) as usize
        };
        used += n;
        out.push(n);
    }
    out
}

fn nested_targets(spec: &LargeSpec, i: usize, rng: &mut Rng) -> Vec<usize> {
    let s = spec.sequences;
    // grupo A = 1..=a (compõem B); grupo B = a+1..s (folhas). Profundidade total 3 (≤ 16).
    let a = ((s - 1) / 3).max(1);
    let b: Vec<usize> = (a + 1..s).collect();
    if i == 0 {
        let pool: Vec<usize> = (1..s).collect();
        let k = pool.len().min(12);
        (0..k).map(|j| pool[(j * pool.len()) / k]).collect()
    } else if i <= a && !b.is_empty() {
        (0..3).map(|_| *rng.pick(&b)).collect()
    } else {
        Vec::new()
    }
}

#[allow(clippy::too_many_lines)]
fn build_sequences(
    bt: &mut Batcher,
    spec: &LargeSpec,
    assets: &[AssetPlan],
    rng: &mut Rng,
) -> Result<(Counts, Vec<String>), FixtureError> {
    let video: Vec<&AssetPlan> = assets
        .iter()
        .filter(|a| a.kind == AssetKind::Video)
        .collect();
    let audio: Vec<&AssetPlan> = assets
        .iter()
        .filter(|a| a.kind == AssetKind::Audio)
        .collect();
    let image: Vec<&AssetPlan> = assets
        .iter()
        .filter(|a| a.kind == AssetKind::Image)
        .collect();
    let alloc = allocate(spec, rng);
    let mut counts = Counts::default();
    let mut main_clips = Vec::new();
    // filhas primeiro: o pai só pode compor sequences que já existem e têm conteúdo
    for idx in (0..spec.sequences).rev() {
        let sid = format!("seq_{idx:03}");
        let n = alloc[idx];
        let targets = nested_targets(spec, idx, rng);
        let k = targets.len();
        let rest = n - k;
        let (v1, v2, t1, c1, a1) = (
            rest * 30 / 100,
            rest * 15 / 100,
            rest * 8 / 100,
            rest * 25 / 100,
            rest * 12 / 100,
        );
        let a2 = rest - v1 - v2 - t1 - c1 - a1;
        bt.push(Command::CreateSequence {
            id: Some(sid.as_str().into()),
            name: format!("Sequence {idx:03}"),
            frame_rate: FrameRate::FPS_30,
            sample_rate: None,
            width: None,
            height: None,
            folder: None,
        })?;
        let tracks = [
            ("V1", TrackKind::Visual, TrackRole::Main),
            ("V2", TrackKind::Visual, TrackRole::Overlay),
            ("T1", TrackKind::Visual, TrackRole::Text),
            ("C1", TrackKind::Visual, TrackRole::Captions),
            ("A1", TrackKind::Audio, TrackRole::Voice),
            ("A2", TrackKind::Audio, TrackRole::Music),
        ];
        for (name, kind, role) in tracks {
            bt.push(Command::AddTrack {
                sequence: SequenceId::from(sid.as_str()),
                id: Some(format!("{sid}.{name}").as_str().into()),
                kind,
                name: None,
                role: Some(role),
                magnetic: false,
                index: None,
            })?;
        }
        // V1: mídia (vídeo) com propriedades
        let mut cursor = 0i64;
        for j in 0..v1 {
            let a = video[usize::try_from(rng.range(0, video.len() as u64 - 1)).unwrap_or(0)];
            let dur = rng.range(30, 150) as i64;
            let asset_frames = a.seconds * 30;
            let src = rng.range(0, (asset_frames - dur).max(0) as u64) as i64;
            let id = format!("c{idx}_v1_{j}");
            bt.push(Command::InsertClip {
                track: format!("{sid}.V1").as_str().into(),
                start: Ticks(cursor * FRAME),
                clip: new_clip(
                    id.clone(),
                    format!("Take {j}"),
                    dur,
                    ClipContent::Media {
                        asset: a.id.clone(),
                        has_video: true,
                        has_audio: j % 2 == 0,
                    },
                    src,
                ),
                split_at_insert: false,
                split_new_id: None,
            })?;
            counts.media += 1;
            if j % 9 == 0 {
                bt.push(Command::SetProperty {
                    clip: id.as_str().into(),
                    prop: "opacity".into(),
                    value: 0.85,
                })?;
            }
            if idx == 0 {
                main_clips.push(id);
            }
            cursor += dur + rng.range(0, 2) as i64;
        }
        // V2: sólidos/imagens com keyframes + nested (compose) espalhados
        let mut order: Vec<Option<usize>> = vec![None; v2];
        for (j, t) in targets.iter().enumerate() {
            let pos = if v2 == 0 {
                0
            } else {
                (j * (v2 + k)) / k.max(1)
            };
            order.insert(pos.min(order.len()), Some(*t));
        }
        let mut cursor = 0i64;
        for (j, slot) in order.into_iter().enumerate() {
            let track: capia_model::TrackId = format!("{sid}.V2").as_str().into();
            match slot {
                Some(child_idx) => {
                    let child = format!("seq_{child_idx:03}");
                    let child_frames = bt
                        .project
                        .document()
                        .sequence(&SequenceId::from(child.as_str()))
                        .map_or(0, |s| s.duration().0 / FRAME);
                    if child_frames == 0 {
                        return Err(FixtureError(format!("{child} is empty when nested")));
                    }
                    let dur = child_frames.min(90);
                    bt.push(Command::InsertNested {
                        track,
                        start: Ticks(cursor * FRAME),
                        sequence: child.as_str().into(),
                        id: Some(format!("c{idx}_n_{j}").as_str().into()),
                        name: format!("Nested {child}"),
                        duration: Some(Ticks(dur * FRAME)),
                        source_in: Ticks::ZERO,
                        follow_length: false,
                        split_at_insert: false,
                        split_new_id: None,
                    })?;
                    counts.nested += 1;
                    cursor += dur;
                }
                None => {
                    let dur = rng.range(20, 90) as i64;
                    let id = format!("c{idx}_v2_{j}");
                    let content = if j % 5 == 4 && !image.is_empty() {
                        ClipContent::Image {
                            asset: image[usize::try_from(rng.range(0, image.len() as u64 - 1))
                                .unwrap_or(0)]
                            .id
                            .clone(),
                        }
                    } else {
                        ClipContent::Solid {
                            color: format!("#{:06X}", rng.range(0, 0xFF_FFFF)),
                        }
                    };
                    let solid = matches!(content, ClipContent::Solid { .. });
                    bt.push(Command::InsertClip {
                        track,
                        start: Ticks(cursor * FRAME),
                        clip: new_clip(id.clone(), format!("Overlay {j}"), dur, content, 0),
                        split_at_insert: false,
                        split_new_id: None,
                    })?;
                    if solid && j % 4 == 0 && dur >= 4 {
                        for (at, value) in [(cursor, 0.2), (cursor + dur / 2, 1.0)] {
                            bt.push(Command::AddKeyframe {
                                clip: id.as_str().into(),
                                prop: "opacity".into(),
                                at: Ticks(at * FRAME),
                                value,
                                interp: None,
                            })?;
                        }
                        counts.keyframed += 1;
                    }
                    cursor += dur + rng.range(0, 2) as i64;
                }
            }
        }
        // T1: texto; C1: legendas curtas
        for (track, count, lo, hi, caption) in
            [("T1", t1, 60, 150, false), ("C1", c1, 15, 45, true)]
        {
            let mut cursor = 0i64;
            for j in 0..count {
                let dur = rng.range(lo, hi) as i64;
                let text = words(rng, if caption { 4 } else { 6 });
                bt.push(Command::InsertClip {
                    track: format!("{sid}.{track}").as_str().into(),
                    start: Ticks(cursor * FRAME),
                    clip: new_clip(
                        format!("c{idx}_{track}_{j}"),
                        format!("{track} {j}"),
                        dur,
                        ClipContent::Text {
                            text,
                            style: TextStyle::default(),
                        },
                        0,
                    ),
                    split_at_insert: false,
                    split_new_id: None,
                })?;
                if caption {
                    counts.captions += 1;
                } else {
                    counts.text += 1;
                }
                cursor += dur + rng.range(0, 2) as i64;
            }
        }
        // A1/A2: áudio
        for (track, count) in [("A1", a1), ("A2", a2)] {
            let mut cursor = 0i64;
            for j in 0..count {
                let a = audio[usize::try_from(rng.range(0, audio.len() as u64 - 1)).unwrap_or(0)];
                let dur = rng.range(60, 300) as i64;
                let src = rng.range(0, (a.seconds * 30 - dur).max(0) as u64) as i64;
                bt.push(Command::InsertClip {
                    track: format!("{sid}.{track}").as_str().into(),
                    start: Ticks(cursor * FRAME),
                    clip: new_clip(
                        format!("c{idx}_{track}_{j}"),
                        format!("{track} {j}"),
                        dur,
                        ClipContent::Media {
                            asset: a.id.clone(),
                            has_video: false,
                            has_audio: true,
                        },
                        src,
                    ),
                    split_at_insert: false,
                    split_new_id: None,
                })?;
                counts.audio += 1;
                cursor += dur + rng.range(0, 3) as i64;
            }
        }
        for m in 0..5i64 {
            bt.push(Command::AddMarker {
                sequence: SequenceId::from(sid.as_str()),
                id: Some(format!("{sid}.m{m}").as_str().into()),
                time: Ticks(m * 300 * FRAME),
                label: format!("beat {m}"),
            })?;
            counts.markers += 1;
        }
        bt.flush()?;
    }
    Ok((counts, main_clips))
}

fn catalog_records(spec: &LargeSpec, assets: &[AssetPlan], dir: &Path) -> Vec<CatalogOp> {
    assets
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let media_kind = match a.kind {
                AssetKind::Video => MediaKind::Video,
                AssetKind::Audio => MediaKind::Audio,
                AssetKind::Image => MediaKind::Image,
            };
            let ext = match a.kind {
                AssetKind::Video => "mp4",
                AssetKind::Audio => "wav",
                AssetKind::Image => "png",
            };
            let path = dir.join("media").join(format!("m{i:05}.{ext}"));
            let hash = format!(
                "sha256:{:064x}",
                ((u128::from(spec.seed)) << 64 | i as u128) + 1
            );
            CatalogOp {
                record: AssetRecord {
                    asset_id: a.id.clone(),
                    kind: a.kind,
                    content_hash: ContentHash::parse(&hash).unwrap_or_else(|| {
                        // nunca acontece: o formato acima é o do contrato (`sha256:` + 64 hex)
                        unreachable!("fixture hash is well-formed")
                    }),
                    size_bytes: 1_000_000 + i as u64 * 1_237,
                    display_name: format!("m{i:05}.{ext}"),
                    location: AssetLocation::from_path(&path, Some(dir)),
                    known_paths: if i % 7 == 0 {
                        vec![format!("/old-drive/m{i:05}.{ext}")]
                    } else {
                        Vec::new()
                    },
                    media: synthetic_info(media_kind, a.seconds),
                    fingerprint: Some(format!("fp1:{:016x}", i as u64 ^ spec.seed)),
                    status: Availability::Offline,
                    status_checked_ms: 1,
                    imported_ms: 1_700_000_000_000 + i as u64,
                },
                event: CatalogEventKind::Import,
                detail: json!({ "fixture": true }),
                at_ms: 1_700_000_000_000 + i as u64,
            }
        })
        .collect()
}

const STAGES: [&str; 8] = [
    "understand",
    "research",
    "plan",
    "validate_plan",
    "edit",
    "review",
    "correct",
    "deliver",
];

fn plan_json(rng: &mut Rng, run: usize) -> Value {
    let ops: Vec<Value> = (0..24)
        .map(|k| {
            json!({
                "op_index": k,
                "type": "insert_clip",
                "track": format!("seq_{:03}.V1", run % 30),
                "start_frames": rng.range(0, 5_000),
                "frames": rng.range(10, 120),
                "why": words(rng, 8),
            })
        })
        .collect();
    json!({ "plan": ops, "digest": format!("{:016x}", rng.next_u64()) })
}

/// Histórico de AI Runs: runs, stages, eventos, efeitos, orçamento, proveniência e memória.
/// Devolve `(stages, eventos, efeitos)` gravados.
fn populate_runs(
    path: &Path,
    spec: &LargeSpec,
    rng: &mut Rng,
) -> Result<(usize, usize, usize), FixtureError> {
    if spec.runs == 0 {
        return Ok((0, 0, 0));
    }
    let store =
        AutonomyStore::open(path, Duration::from_secs(5)).map_err(|e| ferr("autonomy", e))?;
    let base = 1_700_100_000_000u64;
    let (mut stages, mut events, mut effects) = (0usize, 0usize, 0usize);
    for r in 0..spec.runs {
        let run_id = format!("run_{r:04}");
        let t0 = base + r as u64 * 90_000;
        let group = (r % 5 == 0).then(|| format!("grp_{}", r / 5));
        store
            .create_run(
                &run_id,
                "pending",
                "understand",
                None,
                group.as_deref(),
                &json!({ "goal": words(rng, 12), "brief": words(rng, 40) }),
                t0,
            )
            .map_err(|e| ferr("create_run", e))?;
        let terminal = match r % 10 {
            7 => "failed",
            8 => "paused",
            9 => "cancelled",
            _ => "completed",
        };
        let n_stages = if terminal == "completed" {
            STAGES.len()
        } else {
            5
        };
        let mut rev = 0u64;
        for (s, stage) in STAGES.iter().take(n_stages).enumerate() {
            let ts = t0 + (s as u64 + 1) * 5_000;
            let last = s + 1 == n_stages;
            let key = format!("{run_id}:{stage}:0");
            let row = StageRow {
                run_id: run_id.clone(),
                seq: s as u64 + 1,
                stage: (*stage).to_owned(),
                attempt: 0,
                idem_key: key,
                input_digest: format!("in{:016x}", rng.next_u64()),
                output_digest: Some(format!("out{:016x}", rng.next_u64())),
                status: "completed".into(),
                started_ms: ts,
                ended_ms: Some(ts + 4_000),
                json: plan_json(rng, r),
            };
            let evs: Vec<(String, Value)> = (0..3)
                .map(|e| {
                    (
                        ["stage_started", "tool_result", "stage_completed"][e].to_owned(),
                        json!({ "stage": stage, "note": words(rng, 14), "n": e }),
                    )
                })
                .collect();
            events += evs.len();
            stages += 1;
            let status = if last { terminal } else { "running" };
            let next_stage = if last {
                *stage
            } else {
                STAGES[(s + 1).min(STAGES.len() - 1)]
            };
            rev = store
                .advance(
                    &run_id,
                    rev,
                    &RunUpdate {
                        status: status.to_owned(),
                        stage: next_stage.to_owned(),
                        json: json!({ "cursor": s, "usage": { "micros": rng.range(1_000, 90_000) } }),
                    },
                    Some(&row),
                    &evs,
                    ts + 4_500,
                )
                .map_err(|e| ferr("advance", e))?;
        }
        for k in 0..3 {
            let key = format!("llm:{run_id}:{k}");
            store
                .claim_effect(&key, &run_id, "llm", &json!({ "model": "replay" }), t0 + k)
                .map_err(|e| ferr("claim_effect", e))?;
            store
                .update_effect(
                    &key,
                    "done",
                    Some(&format!("ext_{r}_{k}")),
                    None,
                    t0 + k + 10,
                )
                .map_err(|e| ferr("update_effect", e))?;
            let rid = format!("res_{k}");
            store
                .ledger_reserve(&run_id, &rid, 50_000, None, &json!({}), t0 + k)
                .map_err(|e| ferr("ledger_reserve", e))?;
            store
                .ledger_settle(&run_id, &rid, 31_000 + k * 100, &json!({}), t0 + k + 20)
                .map_err(|e| ferr("ledger_settle", e))?;
            effects += 1;
        }
        if r % 4 == 0 {
            store
                .put_provenance(
                    &format!("ast_{:032x}", r as u128 + 1),
                    Some(&run_id),
                    "generated",
                    &format!("sha256:{:064x}", r as u128 + 1),
                    &json!({ "license": "generated" }),
                    t0,
                )
                .map_err(|e| ferr("put_provenance", e))?;
        }
        if r % 5 == 0 {
            let mid = format!("mem_{r:04}");
            store
                .put_memory(&mid, "active", &json!({ "text": words(rng, 10) }), t0)
                .map_err(|e| ferr("put_memory", e))?;
            store
                .log_memory(&mid, "proposed", "run", Some(&run_id), &json!({}), t0)
                .map_err(|e| ferr("log_memory", e))?;
        }
    }
    Ok((stages, events, effects))
}

/// Constrói o projeto grande em `path` (falha se existir). Determinístico por `spec.seed`: o
/// `document_digest` do resultado depende só da `LargeSpec`.
pub fn build_large_project(path: &Path, spec: &LargeSpec) -> Result<LargeStats, FixtureError> {
    if spec.sequences < 2 {
        return Err(FixtureError("sequences must be >= 2".into()));
    }
    if spec.assets < 10 {
        return Err(FixtureError("assets must be >= 10".into()));
    }
    if spec.clips < spec.sequences * 20 {
        return Err(FixtureError(
            "clips must be >= 20 per sequence (keeps every track populated)".into(),
        ));
    }
    let t0 = Instant::now();
    let mut rng = Rng::new(spec.seed);
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    // NORMAL na construção (o formato do arquivo é idêntico; só evita 1 fsync por commit)
    let opts = StoreOptions {
        synchronous: Synchronous::Normal,
        ..StoreOptions::default()
    };
    let project = Project::create(path, &opts).map_err(|e| ferr("create", e))?;
    let mut bt = Batcher {
        project,
        actor: Actor::user("fixture"),
        ops: Vec::new(),
        batch: spec.batch.max(1),
        counter: 0,
        label: "fixture build",
    };
    let assets = asset_plan(spec);
    for a in &assets {
        bt.push(Command::RegisterAsset {
            asset: Asset {
                id: a.id.clone(),
                name: format!("asset {}", a.id),
                duration: (a.seconds > 0).then_some(Ticks(a.seconds * TICKS_PER_SECOND)),
                has_video: a.kind != AssetKind::Audio,
                has_audio: a.kind == AssetKind::Audio,
                offline: true,
            },
        })?;
    }
    bt.flush()?;
    let (counts, main_clips) = build_sequences(&mut bt, spec, &assets, &mut rng)?;
    // cauda: transações pequenas, cada uma um commit próprio (histórico realista)
    bt.label = "fixture tail";
    bt.batch = 1;
    for k in 0..spec.history_tail {
        let clip = main_clips
            [usize::try_from(rng.range(0, main_clips.len() as u64 - 1)).unwrap_or(0)]
        .clone();
        if k % 3 == 0 {
            bt.push(Command::RenameClip {
                clip: clip.as_str().into(),
                name: format!("Take (rev {k})"),
            })?;
        } else {
            bt.push(Command::SetProperty {
                clip: clip.as_str().into(),
                prop: "opacity".into(),
                value: 0.5 + (k % 5) as f64 / 10.0,
            })?;
        }
    }
    bt.flush()?;
    drop(bt);

    {
        let mut catalog =
            Catalog::open(path, Duration::from_secs(5)).map_err(|e| ferr("catalog", e))?;
        catalog
            .apply_batch(&catalog_records(spec, &assets, &dir))
            .map_err(|e| ferr("catalog batch", e))?;
    }
    let (run_stages, run_events, run_effects) = populate_runs(path, spec, &mut rng)?;

    // estatística medida no arquivo final
    let project = Project::open(path, &opts).map_err(|e| ferr("reopen", e))?;
    let doc: &Document = project.document();
    let (big_id, big) = doc
        .sequences()
        .max_by_key(|(id, s)| (s.clip_count(), std::cmp::Reverse((*id).clone())))
        .map(|(id, s)| (id.to_string(), s))
        .ok_or_else(|| FixtureError("no sequences".into()))?;
    let clips: usize = doc.sequences().map(|(_, s)| s.clip_count()).sum();
    let stats = LargeStats {
        seed: spec.seed,
        sequences: doc.sequences().count(),
        clips,
        nested_clips: counts.nested,
        text_clips: counts.text,
        caption_clips: counts.captions,
        audio_clips: counts.audio,
        media_clips: counts.media,
        keyframed_clips: counts.keyframed,
        markers: counts.markers,
        document_assets: doc.assets().count(),
        catalog_assets: project
            .assets()
            .map_err(|e| ferr("assets", e))?
            .iter()
            .filter(|v| v.catalog.is_some())
            .count() as u64,
        history_entries: project.engine().history().len() as u64,
        revision: project.engine().revision(),
        runs: spec.runs,
        run_stages,
        run_events,
        run_effects,
        biggest_sequence: big_id,
        biggest_sequence_clips: big.clip_count(),
        biggest_sequence_duration_ticks: big.duration().0,
        file_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        build_ms: u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        document_digest: document_digest(doc),
    };
    Ok(stats)
}
