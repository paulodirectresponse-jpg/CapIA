//! API do editor: fachada **JSON** transport-agnóstica sobre `capia-project` (ARCHITECTURE §5/§7).
//! O shell Tauri e o servidor de desenvolvimento/E2E (`capia-devserver`) são adaptadores finos que
//! chamam [`Session::call`]; nenhuma regra de edição mora aqui e **toda mutação do documento passa
//! pelo Command Engine** (`command.execute`/`undo`/`redo`, import via `pump`).
//!
//! Contrato: `call(método, params) -> Reply`. Mutações devolvem o *change set* (`patches` = ops
//! primitivas com `new`, resumos de sequences e flags de undo/redo) para a réplica da UI.
//! Eventos assíncronos (import, export) ficam numa fila drenada por `events.poll`.

mod error;
mod export;
mod model;

pub use error::ApiError;

use capia_assets::ScanOptions;
use capia_commands::{Actor, Transaction};
use capia_media::{FfprobeBackend, MediaConfig, MediaToolchain};
use capia_model::{AssetId, SequenceId};
use capia_project::{
    PipelineOptions, Project, PumpEvent, RenderServices, RenderSettings, engine_info,
    parse_transaction,
};
use capia_store::StoreOptions;
use capia_time::Ticks;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Resultado da fase 1 de [`Session::begin`].
#[derive(Debug)]
pub enum Outcome {
    /// Pronto (resposta pequena, calculada sob o lock).
    Done(Reply),
    /// Trabalho pesado já preparado: rode com [`Job::run`] **sem** o lock da sessão.
    Later(Job),
}

/// Trabalho independente da sessão (ex.: renderizar um quadro).
pub struct Job(Box<dyn FnOnce() -> Result<Reply, ApiError> + Send>);

impl Job {
    pub fn run(self) -> Result<Reply, ApiError> {
        (self.0)()
    }
}

impl core::fmt::Debug for Job {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Job(..)")
    }
}

/// Resposta de uma chamada: JSON ou bytes (quadros, miniaturas) com metadados.
#[derive(Debug)]
pub enum Reply {
    Json(Value),
    Binary {
        mime: &'static str,
        bytes: Vec<u8>,
        meta: Value,
    },
}

/// Configuração da sessão (testes injetam caminhos explícitos do FFmpeg).
#[derive(Clone, Debug, Default)]
pub struct SessionConfig {
    pub media: MediaConfig,
    pub store: StoreOptions,
}

pub(crate) type EventQueue = Arc<Mutex<VecDeque<Value>>>;

struct Open {
    project: Project,
    /// Maior id de entrada de histórico já enviado à UI (para o `pump` do import).
    last_entry: u64,
}

/// Uma sessão de editor: no máximo um projeto aberto.
pub struct Session {
    cfg: SessionConfig,
    open: Option<Open>,
    toolchain: Option<MediaToolchain>,
    services: Option<Arc<RenderServices>>,
    events: EventQueue,
    exports: export::Exports,
    actor: Actor,
}

impl core::fmt::Debug for Session {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Session")
            .field(
                "open",
                &self.open.as_ref().map(|o| o.project.path().to_path_buf()),
            )
            .finish_non_exhaustive()
    }
}

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, ApiError> {
    Ok(serde_json::from_value(v)?)
}

#[derive(Deserialize)]
struct PathParam {
    path: String,
}

#[derive(Deserialize)]
struct SeqParam {
    sequence: SequenceId,
}

#[derive(Deserialize)]
struct ExecuteParams {
    #[serde(default = "default_label")]
    label: String,
    commands: Value,
}

fn default_label() -> String {
    "edit".to_owned()
}

#[derive(Deserialize)]
struct ImportParams {
    paths: Vec<String>,
}

#[derive(Deserialize)]
struct AssetParam {
    asset: AssetId,
}

#[derive(Deserialize)]
struct ThumbParams {
    asset: AssetId,
    #[serde(default)]
    at: i64,
    #[serde(default = "default_thumb")]
    max_dim: u32,
}

fn default_thumb() -> u32 {
    240
}

#[derive(Deserialize)]
struct SelectiveUndoParams {
    #[serde(default)]
    entries: Option<Vec<u64>>,
    /// Todas as entradas aplicadas deste ator (por exemplo `run:<id>`).
    #[serde(default)]
    actor_id: Option<String>,
    #[serde(default)]
    mode: Option<capia_commands::SelectiveUndoMode>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Deserialize)]
struct AudioParams {
    sequence: SequenceId,
    from: i64,
    /// Duração do trecho em Ticks.
    duration: i64,
    #[serde(default)]
    sample_rate: Option<u32>,
}

#[derive(Deserialize)]
struct FrameParams {
    sequence: SequenceId,
    at: i64,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct PeaksParams {
    asset: AssetId,
    #[serde(default)]
    from: i64,
    /// `None` = até o fim do asset.
    #[serde(default)]
    to: Option<i64>,
    buckets: usize,
}

#[derive(Deserialize)]
struct RelinkParams {
    asset: AssetId,
    path: String,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize)]
struct FolderParam {
    folder: String,
}

impl Session {
    pub fn new(cfg: SessionConfig) -> Self {
        Self {
            toolchain: MediaToolchain::locate(&cfg.media).ok(),
            cfg,
            open: None,
            services: None,
            events: Arc::new(Mutex::new(VecDeque::new())),
            exports: export::Exports::default(),
            actor: Actor::user("editor"),
        }
    }

    pub(crate) fn push_event(events: &EventQueue, ev: Value) {
        if let Ok(mut q) = events.lock() {
            q.push_back(ev);
        }
    }

    /// Avisa os **outros** clientes (UI, CLI, futura IA) de que o documento mudou: só a revisão —
    /// quem tiver uma réplica mais antiga ressincroniza; quem emitiu o comando já a tem (a UI ignora
    /// o evento quando a revisão não é mais nova que a sua).
    fn announce_revision(events: &EventQueue, reply: &Value) {
        if let Some(rev) = reply.get("revision").and_then(Value::as_u64) {
            Self::push_event(
                events,
                json!({ "kind": "revision_changed", "revision": rev }),
            );
        }
    }

    fn open_ref(&self) -> Result<&Open, ApiError> {
        self.open.as_ref().ok_or_else(ApiError::no_project)
    }

    fn open_mut(&mut self) -> Result<&mut Open, ApiError> {
        self.open.as_mut().ok_or_else(ApiError::no_project)
    }

    fn render_services(&mut self) -> Result<Arc<RenderServices>, ApiError> {
        if let Some(s) = &self.services {
            return Ok(Arc::clone(s));
        }
        let tc = self.toolchain.clone().ok_or_else(|| {
            ApiError::new(
                "FFMPEG_NOT_FOUND",
                "ffmpeg/ffprobe were not found: import, preview and export are unavailable (editing still works)",
            )
        })?;
        let s = Arc::new(RenderServices::new(tc));
        self.services = Some(Arc::clone(&s));
        Ok(s)
    }

    fn start_project(&mut self, mut project: Project) -> Result<Value, ApiError> {
        if let Some(tc) = &self.toolchain {
            // sem pipeline o editor continua funcionando (sem import em background)
            let _ = project.start_pipeline(PipelineOptions::new(tc.clone()));
        }
        let last_entry = project.engine().history().last().map_or(0, |e| e.id);
        let assets = project.assets()?;
        let snap = model::snapshot(&project, &assets);
        self.open = Some(Open {
            project,
            last_entry,
        });
        Ok(snap)
    }

    fn commit_reply(o: &mut Open, entry_id: u64, inverse: bool, extra: Value) -> Value {
        let entry = o
            .project
            .engine()
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .cloned();
        let mut v = match &entry {
            Some(e) => model::change_set(&o.project, &[(e, inverse)]),
            None => model::engine_flags(o.project.engine()),
        };
        if let Some(e) = &entry {
            o.last_entry = o.last_entry.max(e.id);
        }
        if let Value::Object(m) = extra {
            for (k, val) in m {
                v[k] = val;
            }
        }
        v
    }

    // ---- entradas **só Rust** para clientes não-humanos (IA/API; ADR-029/030/085) -----------------
    //
    // Não existem como método de `call`/`begin`: a UI (JS) não alcança estas funções. Atores `Agent`/
    // `Api` jamais escrevem direto — só `preview → apply_plan`, com token HMAC preso ao ator.

    /// Fase 1 do gate: valida e devolve o diff + `plan_token` (nada é gravado).
    pub fn agent_preview(
        &mut self,
        actor: &Actor,
        label: &str,
        commands: Value,
    ) -> Result<Value, ApiError> {
        let tx: Transaction = parse_transaction(&commands.to_string(), label)
            .map_err(|e| ApiError::invalid(e.to_string()))?;
        let o = self.open_mut()?;
        let pv = o.project.preview(actor, tx)?;
        Ok(json!({
            "plan_token": pv.plan_token,
            "already_applied": pv.already_applied,
            "base_revision": pv.base_revision,
            "plan_digest": pv.plan_digest,
            "diff_digest": pv.diff_digest,
            "op_count": pv.ops.len(),
            "refs": pv.refs,
            "results": serde_json::to_value(&pv.results)?,
        }))
    }

    /// Fase 2 do gate: aplica **só** o plano revisado (rebase falha se o documento mudou).
    pub fn agent_apply(&mut self, actor: &Actor, token: &str) -> Result<Value, ApiError> {
        let o = self.open_mut()?;
        let r = o.project.apply_plan(actor, token)?;
        let extra = json!({
            "refs": r.refs,
            "results": serde_json::to_value(&r.results)?,
            "replayed": r.replayed,
        });
        let reply = Self::commit_reply(o, r.entry_id, false, extra);
        Self::announce_revision(&self.events, &reply);
        Ok(reply)
    }

    /// Importa um arquivo **já em staging** pelo caminho do sistema de assets (hash + probe +
    /// registro no documento e no catálogo na MESMA transação; ADR-046/087). Só o Asset Gateway
    /// chama isto, depois de validar tamanho/tipo/licença; não é método de `call`/`begin`.
    /// Devolve o ticket do import assíncrono.
    pub fn agent_import_begin(&mut self, path: &Path) -> Result<String, ApiError> {
        let o = self.open_mut()?;
        let t = o.project.import_asset_async(path)?;
        Ok(t.ticket_id)
    }

    /// Finaliza imports prontos (publica os eventos para a UI) e devolve o estado do ticket.
    pub fn agent_import_poll(&mut self, ticket_id: &str) -> Result<Value, ApiError> {
        self.pump_events()?;
        let o = self.open_ref()?;
        let row = o.project.ticket(ticket_id)?;
        Ok(match row {
            Some(r) => serde_json::to_value(r)?,
            None => json!({"state": "unknown"}),
        })
    }

    /// Cancela um import em staging (Gateway cancelado/abortado).
    pub fn agent_import_cancel(&mut self, ticket_id: &str) -> Result<bool, ApiError> {
        let o = self.open_mut()?;
        Ok(o.project.cancel_ticket(ticket_id)?)
    }

    /// Documento (somente leitura) — a IA enxerga a timeline por tools paginadas, nunca por aqui
    /// diretamente; hospedeiros Rust (capia-intelligence) usam isto para montar *digests*.
    pub fn document(&self) -> Result<&capia_model::Document, ApiError> {
        Ok(self.open_ref()?.project.document())
    }

    pub fn project_path(&self) -> Option<PathBuf> {
        self.open.as_ref().map(|o| o.project.path().to_path_buf())
    }

    pub fn media_toolchain(&self) -> Option<&MediaToolchain> {
        self.toolchain.as_ref()
    }

    /// Ponto único de entrada (síncrono): resolve e, se preciso, executa o trabalho pesado.
    pub fn call(&mut self, method: &str, p: Value) -> Result<Reply, ApiError> {
        match self.begin(method, p)? {
            Outcome::Done(r) => Ok(r),
            Outcome::Later(job) => job.run(),
        }
    }

    /// Entrada em **duas fases** para hospedeiros com lock: a fase 1 (esta, sob o lock da sessão)
    /// resolve tudo o que precisa do projeto; o que for caro (compositor/decode do preview) volta
    /// como [`Job`], que roda **fora** do lock — comandos e undo nunca esperam um quadro.
    pub fn begin(&mut self, method: &str, p: Value) -> Result<Outcome, ApiError> {
        if method == "render.frame" {
            let FrameParams {
                sequence,
                at,
                width,
                height,
            } = params(p)?;
            let services = self.render_services()?;
            let o = self.open_ref()?;
            let mut settings = RenderSettings::new(width, height);
            settings.strict_sources = false;
            settings.design_size = o
                .project
                .document()
                .sequence(&sequence)
                .map(|s| (s.header.width, s.header.height));
            let frame = o.project.prepare_frame(&services, &sequence)?;
            return Ok(Outcome::Later(Job(Box::new(move || {
                let f = frame.render(Ticks(at), &settings)?;
                Ok(Reply::Binary {
                    mime: "application/x-rgba",
                    meta: json!({
                        "width": f.image.width,
                        "height": f.image.height,
                        "warnings": f.warnings.iter().map(|w| json!({"code": w.code, "message": w.message})).collect::<Vec<_>>(),
                    }),
                    bytes: f.image.data,
                })
            }))));
        }
        if method == "render.audio" {
            let AudioParams {
                sequence,
                from,
                duration,
                sample_rate,
            } = params(p)?;
            let services = self.render_services()?;
            let o = self.open_ref()?;
            let frame = o.project.prepare_frame(&services, &sequence)?;
            let mut settings = RenderSettings::new(2, 2);
            settings.strict_sources = false;
            settings.audio_sample_rate = sample_rate.unwrap_or(48_000);
            settings.audio_channels = 2;
            let range = capia_time::TimeRange::new(Ticks(from), Ticks(duration));
            return Ok(Outcome::Later(Job(Box::new(move || {
                let (buf, warnings) = frame.render_audio(range, &settings)?;
                let frames = buf.samples.len() / buf.channels.max(1) as usize;
                let mut bytes = Vec::with_capacity(buf.samples.len() * 4);
                for s in &buf.samples {
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
                Ok(Reply::Binary {
                    mime: "application/x-f32le",
                    meta: json!({
                        "sample_rate": buf.sample_rate,
                        "channels": buf.channels,
                        "frames": frames,
                        "warnings": warnings.iter().map(|w| json!({"code": w.code, "message": w.message})).collect::<Vec<_>>(),
                    }),
                    bytes,
                })
            }))));
        }
        self.call_locked(method, p).map(Outcome::Done)
    }

    fn call_locked(&mut self, method: &str, p: Value) -> Result<Reply, ApiError> {
        match method {
            "engine.info" => {
                let mut v = serde_json::to_value(engine_info())?;
                v["media_available"] = json!(self.toolchain.is_some());
                v["media_version"] = json!(self.toolchain.as_ref().map(|t| t.version.clone()));
                Ok(Reply::Json(v))
            }
            "project.create" => {
                let PathParam { path } = params(p)?;
                let project = Project::create(Path::new(&path), &self.cfg.store)?;
                Ok(Reply::Json(self.start_project(project)?))
            }
            "project.open" => {
                let PathParam { path } = params(p)?;
                let project = Project::open(Path::new(&path), &self.cfg.store)?;
                Ok(Reply::Json(self.start_project(project)?))
            }
            "project.close" => {
                self.open = None;
                Ok(Reply::Json(json!({ "closed": true })))
            }
            "project.snapshot" => {
                let o = self.open_ref()?;
                let assets = o.project.assets()?;
                Ok(Reply::Json(model::snapshot(&o.project, &assets)))
            }
            "sequence.get" => {
                let SeqParam { sequence } = params(p)?;
                let o = self.open_ref()?;
                model::sequence_model(&o.project, &sequence)
                    .map(Reply::Json)
                    .ok_or_else(|| {
                        ApiError::new("NOT_FOUND", format!("sequence {sequence} does not exist"))
                    })
            }
            "command.execute" => {
                let ExecuteParams { label, commands } = params(p)?;
                let tx: Transaction = parse_transaction(&commands.to_string(), &label)
                    .map_err(|e| ApiError::invalid(e.to_string()))?;
                let actor = self.actor.clone();
                let o = self.open_mut()?;
                let r = o.project.execute(&actor, tx)?;
                let extra = json!({
                    "refs": r.refs,
                    "results": serde_json::to_value(&r.results)?,
                    "replayed": r.replayed,
                });
                let reply = Self::commit_reply(o, r.entry_id, false, extra);
                Self::announce_revision(&self.events, &reply);
                Ok(Reply::Json(reply))
            }
            "command.undo" => {
                let actor = self.actor.clone();
                let o = self.open_mut()?;
                let r = o.project.undo(&actor)?;
                let reply = Self::commit_reply(o, r.entry_id, true, json!({}));
                Self::announce_revision(&self.events, &reply);
                Ok(Reply::Json(reply))
            }
            "command.redo" => {
                let actor = self.actor.clone();
                let o = self.open_mut()?;
                let r = o.project.redo(&actor)?;
                let reply = Self::commit_reply(o, r.entry_id, false, json!({}));
                Self::announce_revision(&self.events, &reply);
                Ok(Reply::Json(reply))
            }
            "history.list" => Ok(Reply::Json(model::history(&self.open_ref()?.project))),
            "history.undo_report" | "history.undo_selective" => {
                let q: SelectiveUndoParams = params(p)?;
                let actor = self.actor.clone();
                let o = self.open_mut()?;
                let ids: Vec<u64> = match (&q.entries, &q.actor_id) {
                    (Some(e), None) => e.clone(),
                    (None, Some(a)) => o
                        .project
                        .engine()
                        .applied_history()
                        .iter()
                        .filter(|h| h.actor.id == *a)
                        .map(|h| h.id)
                        .collect(),
                    _ => {
                        return Err(ApiError::invalid(
                            "provide exactly one of `entries` or `actor_id`",
                        ));
                    }
                };
                if method == "history.undo_report" {
                    let rep = o.project.engine().selective_undo_report(&ids)?;
                    return Ok(Reply::Json(serde_json::to_value(rep)?));
                }
                let label = q.label.unwrap_or_else(|| "Selective undo".to_owned());
                let r = o.project.selective_undo(
                    &actor,
                    &ids,
                    q.mode.unwrap_or(capia_commands::SelectiveUndoMode::Safe),
                    &label,
                )?;
                let reply = Self::commit_reply(o, r.entry_id, false, json!({"undone": ids}));
                Self::announce_revision(&self.events, &reply);
                Ok(Reply::Json(reply))
            }
            "assets.list" => {
                let o = self.open_ref()?;
                Ok(Reply::Json(model::asset_rows(&o.project.assets()?)))
            }
            "assets.import" => {
                let ImportParams { paths } = params(p)?;
                let o = self.open_mut()?;
                let mut tickets = Vec::new();
                let mut errors = Vec::new();
                for path in &paths {
                    match o.project.import_asset_async(Path::new(path)) {
                        Ok(t) => tickets.push(serde_json::to_value(t)?),
                        Err(e) => errors
                            .push(json!({ "path": path, "error": ApiError::from(e).to_json() })),
                    }
                }
                Ok(Reply::Json(json!({ "tickets": tickets, "errors": errors })))
            }
            "assets.verify" => {
                let AssetParam { asset } = params(p)?;
                let o = self.open_mut()?;
                Ok(Reply::Json(serde_json::to_value(
                    o.project.verify_asset(&asset)?,
                )?))
            }
            "assets.relink" => {
                let RelinkParams {
                    asset,
                    path,
                    force,
                    dry_run,
                } = params(p)?;
                let actor = self.actor.clone();
                let tc = self.toolchain.clone();
                let o = self.open_mut()?;
                if force {
                    let tc = tc.ok_or_else(|| {
                        ApiError::new("FFMPEG_NOT_FOUND", "ffmpeg/ffprobe were not found")
                    })?;
                    let probe = FfprobeBackend::new(tc);
                    let r = o.project.force_relink_asset(
                        &actor,
                        &asset,
                        Path::new(&path),
                        &probe,
                        dry_run,
                    )?;
                    let extra = json!({ "force_relink": serde_json::to_value(&r)? });
                    let v = match &r.commit {
                        Some(c) => Self::commit_reply(o, c.entry_id, false, extra),
                        None => {
                            let mut v = model::engine_flags(o.project.engine());
                            v["force_relink"] = extra["force_relink"].clone();
                            v
                        }
                    };
                    return Ok(Reply::Json(v));
                }
                let r = o.project.relink_asset(&asset, Path::new(&path))?;
                let mut v = model::engine_flags(o.project.engine());
                v["relink"] = serde_json::to_value(&r)?;
                Ok(Reply::Json(v))
            }
            "assets.relink_folder" => {
                let FolderParam { folder } = params(p)?;
                let o = self.open_mut()?;
                let (report, applied, errors) = o.project.batch_relink_folder(
                    Path::new(&folder),
                    &ScanOptions::default(),
                    None,
                    &|| false,
                )?;
                let mut v = model::engine_flags(o.project.engine());
                v["report"] = serde_json::to_value(&report)?;
                v["applied"] = serde_json::to_value(&applied)?;
                v["errors"] = Value::Array(errors);
                Ok(Reply::Json(v))
            }
            "media.thumbnail" => {
                let ThumbParams { asset, at, max_dim } = params(p)?;
                let tc = self.toolchain.clone().ok_or_else(|| {
                    ApiError::new("FFMPEG_NOT_FOUND", "ffmpeg/ffprobe were not found")
                })?;
                let o = self.open_ref()?;
                let path = o
                    .project
                    .generate_thumbnail(&asset, Ticks(at), max_dim, &tc)?;
                let bytes = std::fs::read(&path).map_err(|e| {
                    ApiError::new("IO_ERROR", format!("cannot read the thumbnail: {e}"))
                })?;
                // o formato da miniatura é do `capia-assets`: anuncia o que realmente veio
                let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                    "image/png"
                } else {
                    "image/jpeg"
                };
                Ok(Reply::Binary {
                    mime,
                    bytes,
                    meta: json!({ "asset": asset }),
                })
            }
            "media.peaks" => {
                let PeaksParams {
                    asset,
                    from,
                    to,
                    buckets,
                } = params(p)?;
                let tc = self.toolchain.clone().ok_or_else(|| {
                    ApiError::new("FFMPEG_NOT_FOUND", "ffmpeg/ffprobe were not found")
                })?;
                if !(1..=16_384).contains(&buckets) {
                    return Err(ApiError::invalid("buckets must be within 1..=16384"));
                }
                let o = self.open_ref()?;
                let wf = o.project.waveform(&asset, &tc, &|| false)?;
                let end = to.map_or_else(|| wf.duration(), Ticks);
                let peaks = wf.query(Ticks(from), end, buckets);
                let flat: Vec<f32> = peaks.iter().flat_map(|p| [p.min, p.max]).collect();
                Ok(Reply::Json(
                    json!({ "asset": asset, "buckets": buckets, "peaks": flat }),
                ))
            }
            "export.encoders" => {
                let services = self.render_services()?;
                let caps = Project::detect_export_encoders(&services)?;
                Ok(Reply::Json(serde_json::to_value(caps)?))
            }
            "export.start" => {
                let services = self.render_services()?;
                let path = self.open_ref()?.project.path().to_path_buf();
                Ok(Reply::Json(self.exports.start(
                    p,
                    path,
                    self.cfg.store.clone(),
                    services,
                    Arc::clone(&self.events),
                )?))
            }
            "export.cancel" => {
                #[derive(Deserialize)]
                struct Id {
                    id: String,
                }
                let Id { id } = params(p)?;
                Ok(Reply::Json(
                    json!({ "cancelled": self.exports.cancel(&id) }),
                ))
            }
            "jobs.list" => {
                let o = self.open_ref()?;
                let jobs = o.project.jobs(None, 200)?;
                Ok(Reply::Json(serde_json::to_value(jobs)?))
            }
            "events.poll" => Ok(Reply::Json(self.poll()?)),
            other => Err(ApiError::new(
                "UNKNOWN_METHOD",
                format!("unknown method `{other}`"),
            )),
        }
    }

    /// Aplica o que os workers prepararam (`pump`) e drena a fila de eventos.
    fn poll(&mut self) -> Result<Value, ApiError> {
        self.pump_events()?;
        let drained: Vec<Value> = self
            .events
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();
        Ok(json!({ "events": drained }))
    }

    /// Metade "pump" do poll: finaliza imports/relinks prontos e **enfileira** os eventos para a UI,
    /// sem drená-los (quem drena é só o `events.poll` da UI).
    fn pump_events(&mut self) -> Result<(), ApiError> {
        let actor = self.actor.clone();
        if let Some(o) = self.open.as_mut() {
            let evs = o.project.pump(&actor)?;
            if !evs.is_empty() {
                let new: Vec<_> = o
                    .project
                    .engine()
                    .applied_history()
                    .iter()
                    .filter(|e| e.id > o.last_entry)
                    .cloned()
                    .collect();
                let refs: Vec<(&capia_commands::HistoryEntry, bool)> =
                    new.iter().map(|e| (e, false)).collect();
                if let Some(max) = new.iter().map(|e| e.id).max() {
                    o.last_entry = o.last_entry.max(max);
                }
                let changes = (!refs.is_empty()).then(|| model::change_set(&o.project, &refs));
                for ev in evs {
                    let mut v = serde_json::to_value(&ev)?;
                    if let PumpEvent::ImportFinalized { .. } = ev {
                        v["kind"] = json!("import_finalized");
                    }
                    Self::push_event(&self.events, v);
                }
                if let Some(c) = changes {
                    Self::push_event(
                        &self.events,
                        json!({ "kind": "document_changed", "change": c }),
                    );
                    if let Ok(assets) = o.project.assets() {
                        Self::push_event(
                            &self.events,
                            json!({ "kind": "assets_changed", "assets": model::asset_rows(&assets) }),
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

/// Caminho padrão de projetos de teste (`<dir>/<nome>.capia`).
pub fn project_path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.capia"))
}
