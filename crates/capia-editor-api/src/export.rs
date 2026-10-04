//! Export em segundo plano (um ou vários itens, em sequência). Cada lote abre **sua própria**
//! instância somente-leitura do projeto: o export renderiza um snapshot fixo e nunca bloqueia nem
//! vê a edição em curso (ARCHITECTURE §6). Escrita atômica/validação por ffprobe são do
//! `capia-project` (ADR-068); aqui só orquestramos, reportamos progresso e cancelamos.

use crate::{ApiError, EventQueue, Session};
use capia_media::ExportCodec;
use capia_model::SequenceId;
use capia_project::{ExportOptions, Mp4Options, Project, RenderServices, RenderSettings};
use capia_store::StoreOptions;
use capia_time::{Ticks, TimeRange};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(crate) struct Exports {
    next: u64,
    cancels: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
}

#[derive(Clone, Debug, Deserialize)]
struct Item {
    id: String,
    sequence: SequenceId,
    #[serde(default = "default_preset")]
    preset: String,
    path: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    encoder: Option<String>,
}

fn default_preset() -> String {
    "h264-mp4".to_owned()
}

#[derive(Deserialize)]
struct StartParams {
    items: Vec<Item>,
}

fn run_item(
    project: &Project,
    services: &Arc<RenderServices>,
    item: &Item,
    cancel: &AtomicBool,
    batch: &str,
    events: &EventQueue,
) -> Result<Value, ApiError> {
    let graph = project.render_graph(&item.sequence)?;
    let gs = graph
        .sequence(&item.sequence)
        .map_err(|e| ApiError::new(e.code, e.message))?;
    let (sw, sh) = project
        .document()
        .sequence(&item.sequence)
        .map_or((1920, 1080), |s| (s.header.width, s.header.height));
    let mut w = item.width.unwrap_or(sw);
    let mut h = item.height.unwrap_or(sh);
    let mp4 = item.preset != "intermediate";
    if mp4 {
        w -= w % 2;
        h -= h % 2;
    }
    let fd = gs.frame_rate.frame_duration().0.max(1);
    let total_frames = u64::try_from((gs.duration.0 + fd - 1) / fd)
        .unwrap_or(0)
        .max(1);
    let range = TimeRange::new(Ticks(0), gs.duration);
    let settings = RenderSettings::new(w, h);
    let done = AtomicU64::new(0);
    let last = Mutex::new(Instant::now());
    let id = item.id.clone();
    let progress = |frames: u64| {
        let due = last
            .lock()
            .map(|mut l| {
                let due = l.elapsed() >= Duration::from_millis(250);
                if due {
                    *l = Instant::now();
                }
                due
            })
            .unwrap_or(false);
        if due {
            Session::push_event(
                events,
                json!({
                    "kind": "export_progress", "batch": batch, "id": id,
                    "done_frames": frames.min(total_frames), "total_frames": total_frames,
                }),
            );
        }
    };
    // o callback de cancelamento é chamado uma vez por quadro: serve também de contador
    let cancel_fn = || {
        progress(done.fetch_add(1, Ordering::Relaxed) + 1);
        cancel.load(Ordering::SeqCst)
    };
    let out = Path::new(&item.path);
    if mp4 {
        let opts = Mp4Options {
            overwrite: item.overwrite,
            codec: ExportCodec::H264,
            encoder: item.encoder.clone(),
            ..Mp4Options::default()
        };
        let r = project.export_mp4(
            services,
            &item.sequence,
            range,
            &settings,
            out,
            &opts,
            &cancel_fn,
        )?;
        Ok(json!({
            "path": r.path.display().to_string(),
            "encoder": r.encoder, "codec": r.codec, "hardware": r.hardware,
            "frames": r.frames, "width": r.width, "height": r.height,
            "audio_frames": r.audio_frames, "av_drift_ticks": r.av_drift_ticks,
            "warnings": r.warnings,
        }))
    } else {
        let r = project.export_intermediate(
            services,
            &item.sequence,
            range,
            &settings,
            out,
            &ExportOptions {
                overwrite: item.overwrite,
            },
            &cancel_fn,
        )?;
        Ok(json!({
            "path": r.path.display().to_string(),
            "frames": r.frames, "width": r.width, "height": r.height,
            "audio_frames": r.audio_frames, "warnings": r.warnings,
        }))
    }
}

impl Exports {
    pub(crate) fn start(
        &mut self,
        p: Value,
        project_path: PathBuf,
        store: StoreOptions,
        services: Arc<RenderServices>,
        events: EventQueue,
    ) -> Result<Value, ApiError> {
        let StartParams { items } = serde_json::from_value(p)?;
        if items.is_empty() {
            return Err(ApiError::invalid("export.start needs at least one item"));
        }
        self.next += 1;
        let batch = format!("batch-{}", self.next);
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut m) = self.cancels.lock() {
            m.insert(batch.clone(), Arc::clone(&cancel));
        }
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let b = batch.clone();
        std::thread::spawn(move || {
            // uma instância própria do projeto: snapshot fixo, sem pipeline nem escrita
            let project = match Project::open(&project_path, &store) {
                Ok(p) => p,
                Err(e) => {
                    for it in &items {
                        Session::push_event(
                            &events,
                            json!({ "kind": "export_item_finished", "batch": b, "id": it.id,
                                    "ok": false, "error": ApiError::from(e.clone()).to_json() }),
                        );
                    }
                    Session::push_event(
                        &events,
                        json!({ "kind": "export_batch_finished", "batch": b, "cancelled": false }),
                    );
                    return;
                }
            };
            let mut cancelled = false;
            for it in &items {
                if cancel.load(Ordering::SeqCst) {
                    cancelled = true;
                    Session::push_event(
                        &events,
                        json!({ "kind": "export_item_finished", "batch": b, "id": it.id, "ok": false,
                                "cancelled": true }),
                    );
                    continue;
                }
                Session::push_event(
                    &events,
                    json!({ "kind": "export_item_started", "batch": b, "id": it.id }),
                );
                let ev = match run_item(&project, &services, it, &cancel, &b, &events) {
                    Ok(report) => json!({ "kind": "export_item_finished", "batch": b, "id": it.id,
                                          "ok": true, "report": report }),
                    Err(e) if cancel.load(Ordering::SeqCst) => {
                        cancelled = true;
                        json!({ "kind": "export_item_finished", "batch": b, "id": it.id, "ok": false,
                                "cancelled": true, "error": e.to_json() })
                    }
                    Err(e) => json!({ "kind": "export_item_finished", "batch": b, "id": it.id,
                                      "ok": false, "error": e.to_json() }),
                };
                Session::push_event(&events, ev);
            }
            Session::push_event(
                &events,
                json!({ "kind": "export_batch_finished", "batch": b, "cancelled": cancelled }),
            );
        });
        Ok(json!({ "batch": batch, "items": ids }))
    }

    pub(crate) fn cancel(&self, batch: &str) -> bool {
        self.cancels
            .lock()
            .ok()
            .and_then(|m| m.get(batch).cloned())
            .map(|c| c.store(true, Ordering::SeqCst))
            .is_some()
    }
}
