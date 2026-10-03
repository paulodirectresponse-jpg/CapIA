//! Pipeline de mídia em background (ADR-052..058): jobs de hash/índice/waveform/proxy/miniatura,
//! import **não bloqueante** (`ImportTicket`) e relink em lote.
//!
//! Regras:
//! * os **workers** só calculam (hash, probe, ffmpeg, cache): nunca escrevem no documento nem no
//!   catálogo; quem grava é a thread do projeto, em [`Project::pump`] (única porta de escrita
//!   continua sendo o engine/catálogo);
//! * o estado dos jobs e dos tickets é persistido (`JobStore`); reabrir marca o que estava em
//!   andamento como `interrupted` (nunca `completed`) e permite repetir;
//! * só **um processo** é dono do executor de um projeto (lock de arquivo do SO, liberado
//!   automaticamente se o processo morrer).

use crate::derive::{self, Env, FrameSource};
use crate::error::ProjectError;
use crate::project::{Project, now_ms};
use capia_assets::{
    AssetError, AssetErrorCode, AssetRecord, Availability, BatchRelinkReport, CacheDir,
    ContentHash, RelinkTarget, ScanOptions, fingerprint_file, match_candidates, prepare_import_job,
    quick_status, scan_folder,
};
use capia_commands::Actor;
use capia_jobs::{
    Executor, ExecutorConfig, JobCtx, JobError, JobHandle, JobId, JobKind, JobSink, JobSnapshot,
    JobSpec, JobState, Priority, SubmitError, Submitted,
};
use capia_media::{MediaToolchain, ProxyProfileV1};
use capia_model::AssetId;
use capia_store::{Recovery, TicketRow, TicketState};
use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::assets::{ImportResult, RelinkResult};

#[derive(Clone, Debug)]
pub struct PipelineOptions {
    pub toolchain: MediaToolchain,
    pub executor: ExecutorConfig,
}

impl PipelineOptions {
    pub fn new(toolchain: MediaToolchain) -> Self {
        Self {
            toolchain,
            executor: ExecutorConfig::default(),
        }
    }
}

/// Recibo de um import assíncrono: o arquivo foi aceito (existe, é regular, não está vazio),
/// tem impressão rápida e um job de hash+probe na fila. O asset **ainda não existe**.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportTicket {
    pub ticket_id: String,
    pub job_id: String,
    pub path: String,
    pub size_bytes: u64,
    pub fingerprint: String,
    pub state: TicketState,
    /// Asset já conhecido com a MESMA impressão rápida (dica; só o SHA-256 confirma).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possible_duplicate_of: Option<String>,
}

/// Algo que a thread do projeto aplicou em [`Project::pump`].
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PumpEvent {
    ImportFinalized {
        ticket_id: String,
        result: Box<ImportResult>,
    },
    ImportFailed {
        ticket_id: String,
        error: JobError,
    },
    ImportCancelled {
        ticket_id: String,
    },
    ImportInterrupted {
        ticket_id: String,
    },
    BatchRelinkApplied {
        job_id: String,
        report: Box<BatchRelinkReport>,
        applied: Vec<RelinkResult>,
        apply_errors: Vec<Value>,
    },
    BatchRelinkFailed {
        job_id: String,
        error: JobError,
    },
}

#[derive(Debug)]
enum Tracked {
    Import {
        ticket_id: String,
        handle: JobHandle,
    },
    Relink {
        handle: JobHandle,
    },
}

#[derive(Debug)]
pub(crate) struct Pipeline {
    pub(crate) executor: Executor,
    pub(crate) toolchain: MediaToolchain,
    _owner: std::fs::File,
    stop: Arc<AtomicBool>,
    watcher: Option<JoinHandle<()>>,
    tracked: Vec<Tracked>,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.watcher.take() {
            let _ = w.join();
        }
        self.executor.shutdown();
    }
}

fn lock_path(project: &Path) -> PathBuf {
    let mut name = project.file_name().unwrap_or_default().to_os_string();
    name.push(".jobs-lock");
    project.with_file_name(name)
}

/// Tenta tornar este processo o dono do executor (lock exclusivo do SO).
fn try_own(project: &Path) -> Result<Option<std::fs::File>, ProjectError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(project))
        .map_err(|e| {
            ProjectError::invalid("JOBS_LOCK_IO", format!("cannot open the jobs lock: {e}"))
        })?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(ProjectError::invalid(
            "JOBS_LOCK_IO",
            format!("cannot lock the jobs lock: {e}"),
        )),
    }
}

fn job_error(e: AssetError) -> JobError {
    if e.code == AssetErrorCode::AssetCancelled {
        return JobError::cancelled();
    }
    JobError {
        code: e.code.as_str().to_owned(),
        message: e.message,
        details: e.details,
    }
}

fn submit_error(e: SubmitError) -> ProjectError {
    match e {
        SubmitError::QueueFull { .. } => ProjectError::invalid("JOB_QUEUE_FULL", e.to_string()),
        SubmitError::ShuttingDown => ProjectError::invalid("JOB_EXECUTOR_SHUTDOWN", e.to_string()),
    }
}

static IDS: AtomicU64 = AtomicU64::new(0);

fn new_ticket_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "tk_{nanos:x}{:x}{:x}",
        std::process::id(),
        IDS.fetch_add(1, Ordering::SeqCst)
    )
}

fn derived_result(kind: &str, p: &capia_assets::Produced) -> Value {
    json!({ "kind": kind, "path": p.path.display().to_string(), "hit": p.hit })
}

impl Project {
    // ---- ciclo de vida ----------------------------------------------------------------------

    /// Inicia o executor deste projeto. Falha com `JOBS_OWNED_BY_ANOTHER_PROCESS` se outro
    /// processo já o possui. Recupera o que a execução anterior deixou incompleto (`interrupted`).
    pub fn start_pipeline(&mut self, opts: PipelineOptions) -> Result<Recovery, ProjectError> {
        if self.pipeline.is_some() {
            return Err(ProjectError::invalid(
                "PIPELINE_ALREADY_STARTED",
                "the media pipeline is already running",
            ));
        }
        let Some(owner) = try_own(self.path())? else {
            return Err(ProjectError::invalid(
                "JOBS_OWNED_BY_ANOTHER_PROCESS",
                "another process is already running this project's media jobs",
            ));
        };
        let recovery = self.job_store().recover(now_ms())?;
        let sink: Arc<dyn JobSink> = self.job_store().clone();
        let executor = Executor::new(opts.executor, Some(sink));
        let stop = Arc::new(AtomicBool::new(false));
        // cancelamentos pedidos por OUTROS processos (CLI) chegam pela flag no banco
        let watcher = {
            let store = self.job_store().clone();
            let stop = stop.clone();
            let exec = executor.canceller();
            std::thread::Builder::new()
                .name("capia-job-cancel-watch".into())
                .spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        if let Ok(ids) = store.pending_cancels() {
                            for id in ids {
                                exec.cancel(&id);
                            }
                        }
                        for _ in 0..8 {
                            if stop.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                    }
                })
                .ok()
        };
        self.pipeline = Some(Pipeline {
            executor,
            toolchain: opts.toolchain,
            _owner: owner,
            stop,
            watcher,
            tracked: Vec::new(),
        });
        Ok(recovery)
    }

    pub fn pipeline_running(&self) -> bool {
        self.pipeline.is_some()
    }

    /// Encerra o executor (jobs em andamento viram `interrupted`) e libera o lock.
    pub fn stop_pipeline(&mut self) {
        self.pipeline = None;
    }

    /// Para uso **sem** executor (CLI `job list`): se ninguém é dono do executor, converte em
    /// `interrupted` o que ficou `queued/running` de uma execução que morreu.
    pub fn recover_jobs(&self) -> Result<Recovery, ProjectError> {
        if self.pipeline.is_some() {
            return Ok(Recovery::default());
        }
        match try_own(self.path())? {
            Some(_lock) => Ok(self.job_store().recover(now_ms())?),
            None => Ok(Recovery::default()),
        }
    }

    fn pipeline_ref(&self) -> Result<&Pipeline, ProjectError> {
        self.pipeline.as_ref().ok_or_else(|| {
            ProjectError::invalid(
                "PIPELINE_NOT_STARTED",
                "start the media pipeline (start_pipeline) before submitting jobs",
            )
        })
    }

    // ---- consulta e cancelamento -------------------------------------------------------------

    pub fn jobs(
        &self,
        state: Option<JobState>,
        limit: u32,
    ) -> Result<Vec<JobSnapshot>, ProjectError> {
        let mut out = self.job_store().list(state, limit)?;
        // jobs vivos neste processo têm progresso mais novo que o banco
        if let Some(p) = &self.pipeline {
            for s in &mut out {
                if let Some(live) = p.executor.get(&s.id) {
                    *s = live;
                }
            }
        }
        Ok(out)
    }

    pub fn job(&self, id: &JobId) -> Result<Option<JobSnapshot>, ProjectError> {
        if let Some(p) = &self.pipeline
            && let Some(live) = p.executor.get(id)
        {
            return Ok(Some(live));
        }
        Ok(self.job_store().get(id)?)
    }

    /// Cancela um job: direto no executor local ou, de outro processo, pela flag no banco (o dono
    /// observa em ≤ 250 ms e mata o ffmpeg). Um job `interrupted` (sem dono) vira `cancelled`.
    pub fn cancel_job(&self, id: &JobId) -> Result<bool, ProjectError> {
        if let Some(p) = &self.pipeline
            && p.executor.cancel(id)
        {
            return Ok(true);
        }
        let Some(snap) = self.job_store().get(id)? else {
            return Ok(false);
        };
        match snap.state {
            JobState::Queued | JobState::Running => {
                if self.pipeline.is_none() {
                    // sem dono vivo? então não há quem cancele: trata como interrompido→cancelado
                    if let Some(_lock) = try_own(self.path())? {
                        self.job_store().recover(now_ms())?;
                        return Ok(self.job_store().force_state(
                            id,
                            JobState::Cancelled,
                            now_ms(),
                        )?);
                    }
                }
                Ok(self.job_store().request_cancel(id)?)
            }
            JobState::Interrupted => {
                Ok(self
                    .job_store()
                    .force_state(id, JobState::Cancelled, now_ms())?)
            }
            _ => Ok(false),
        }
    }

    pub fn wait_job(
        &self,
        id: &JobId,
        timeout: Duration,
    ) -> Result<Option<JobSnapshot>, ProjectError> {
        let p = self.pipeline_ref()?;
        let Some(h) = p.executor.handle(id) else {
            return Ok(self.job_store().get(id)?);
        };
        Ok(h.wait_timeout(timeout))
    }

    pub fn tickets(&self, state: Option<TicketState>) -> Result<Vec<TicketRow>, ProjectError> {
        Ok(self.job_store().list_tickets(state)?)
    }

    pub fn ticket(&self, id: &str) -> Result<Option<TicketRow>, ProjectError> {
        Ok(self.job_store().get_ticket(id)?)
    }

    // ---- import assíncrono -------------------------------------------------------------------

    fn spawn_import_job(&self, abs: PathBuf) -> Result<Submitted, ProjectError> {
        let p = self.pipeline_ref()?;
        let toolchain = p.toolchain.clone();
        let dir = self.project_dir();
        let spec = JobSpec::new(JobKind::AssetHash, Priority::Interactive)
            .label(format!("import {}", abs.display()))
            .params(json!({ "path": abs.display().to_string() }));
        p.executor
            .submit(spec, move |ctx: &JobCtx| {
                let probe = capia_media::FfprobeBackend::new(toolchain);
                let cancel = || ctx.is_cancelled();
                let prepared = prepare_import_job(
                    &abs,
                    dir.as_deref(),
                    &probe,
                    now_ms(),
                    &cancel,
                    &mut |d, t| ctx.set_progress(d, t),
                )
                .map_err(job_error)?;
                serde_json::to_value(&prepared.record)
                    .map_err(|e| JobError::new("JOB_RESULT_INVALID", e.to_string()))
            })
            .map_err(submit_error)
    }

    /// Import **não bloqueante**: valida o arquivo, calcula a impressão rápida (lê ≲ 1 MiB),
    /// enfileira hash completo + probe e devolve um [`ImportTicket`] imediatamente. O asset só
    /// passa a existir quando [`Project::pump`] finaliza o ticket. Nenhuma edição depende disso.
    pub fn import_asset_async(&mut self, path: &Path) -> Result<ImportTicket, ProjectError> {
        self.pipeline_ref()?;
        let abs = capia_assets::absolute_checked(path)?;
        let meta = std::fs::metadata(&abs).map_err(|e| {
            AssetError::new(
                if e.kind() == std::io::ErrorKind::NotFound {
                    AssetErrorCode::AssetFileNotFound
                } else {
                    AssetErrorCode::AssetIo
                },
                format!("cannot access `{}`: {e}", abs.display()),
            )
        })?;
        if !meta.is_file() {
            return Err(AssetError::new(
                AssetErrorCode::AssetNotRegularFile,
                format!("`{}` is not a regular file", abs.display()),
            )
            .into());
        }
        if meta.len() == 0 {
            return Err(AssetError::new(
                AssetErrorCode::AssetEmptyFile,
                format!("`{}` is empty", abs.display()),
            )
            .into());
        }
        let fp = fingerprint_file(&abs)?.to_text();
        let possible_duplicate_of = self
            .catalog()
            .find_by_fingerprint(&fp)?
            .into_iter()
            .find(|r| r.size_bytes == meta.len())
            .map(|r| r.asset_id.to_string());
        let submitted = self.spawn_import_job(abs.clone())?;
        let job_id = submitted.handle.id().0;
        let now = now_ms();
        let ticket_id = new_ticket_id();
        self.job_store().put_ticket(&TicketRow {
            ticket_id: ticket_id.clone(),
            path: abs.display().to_string(),
            size_bytes: meta.len(),
            fingerprint: Some(fp.clone()),
            state: TicketState::Pending,
            job_id: Some(job_id.clone()),
            asset_id: None,
            outcome: None,
            error: None,
            created_ms: now,
            updated_ms: now,
        })?;
        if let Some(p) = &mut self.pipeline {
            p.tracked.push(Tracked::Import {
                ticket_id: ticket_id.clone(),
                handle: submitted.handle,
            });
        }
        Ok(ImportTicket {
            ticket_id,
            job_id,
            path: abs.display().to_string(),
            size_bytes: meta.len(),
            fingerprint: fp,
            state: TicketState::Pending,
            possible_duplicate_of,
        })
    }

    /// Retoma um ticket `interrupted` (o processo anterior morreu no meio): reenvia o hash.
    pub fn resume_ticket(&mut self, ticket_id: &str) -> Result<ImportTicket, ProjectError> {
        self.pipeline_ref()?;
        let Some(mut row) = self.job_store().get_ticket(ticket_id)? else {
            return Err(ProjectError::invalid(
                "TICKET_NOT_FOUND",
                format!("ticket {ticket_id} does not exist"),
            ));
        };
        if row.state != TicketState::Interrupted {
            return Err(ProjectError::invalid(
                "TICKET_NOT_RESUMABLE",
                format!(
                    "ticket {ticket_id} is {}, only interrupted tickets can resume",
                    row.state.as_str()
                ),
            ));
        }
        let submitted = self.spawn_import_job(PathBuf::from(&row.path))?;
        let job_id = submitted.handle.id().0;
        row.state = TicketState::Pending;
        row.job_id = Some(job_id.clone());
        row.error = None;
        row.updated_ms = now_ms();
        self.job_store().put_ticket(&row)?;
        if let Some(p) = &mut self.pipeline {
            p.tracked.push(Tracked::Import {
                ticket_id: ticket_id.to_owned(),
                handle: submitted.handle,
            });
        }
        Ok(ImportTicket {
            ticket_id: row.ticket_id,
            job_id,
            path: row.path,
            size_bytes: row.size_bytes,
            fingerprint: row.fingerprint.unwrap_or_default(),
            state: TicketState::Pending,
            possible_duplicate_of: None,
        })
    }

    /// Cancela o ticket (cancela o job de hash; o hash para entre blocos).
    pub fn cancel_ticket(&self, ticket_id: &str) -> Result<bool, ProjectError> {
        let Some(row) = self.job_store().get_ticket(ticket_id)? else {
            return Ok(false);
        };
        match row.job_id {
            Some(j) => self.cancel_job(&JobId(j)),
            None => Ok(false),
        }
    }

    // ---- pump: a thread do projeto aplica o que os workers produziram --------------------------

    /// Finaliza o que terminou: tickets de import (registro no catálogo/documento via engine) e
    /// relinks em lote. Não bloqueia. Chame periodicamente (a UI/CLI decide o ritmo).
    pub fn pump(&mut self, actor: &Actor) -> Result<Vec<PumpEvent>, ProjectError> {
        let Some(p) = &mut self.pipeline else {
            return Ok(Vec::new());
        };
        let tracked = std::mem::take(&mut p.tracked);
        let mut keep = Vec::new();
        let mut events = Vec::new();
        let mut first_err: Option<ProjectError> = None;
        for t in tracked {
            let done = match &t {
                Tracked::Import { handle, .. } | Tracked::Relink { handle } => handle.is_done(),
            };
            // job vivo — ou um erro anterior neste pump: o item volta para a fila (nada se perde)
            if !done || first_err.is_some() {
                keep.push(t);
                continue;
            }
            let result = match &t {
                Tracked::Import { ticket_id, handle } => {
                    self.finish_import(actor, ticket_id, &handle.snapshot())
                }
                Tracked::Relink { handle } => self.finish_relink(&handle.snapshot()),
            };
            match result {
                Ok(ev) => events.push(ev),
                Err(e) => {
                    // falha ao gravar (ex.: banco ocupado): o ticket continua `pending` e será
                    // finalizado no próximo `pump`
                    first_err = Some(e);
                    keep.push(t);
                }
            }
        }
        if let Some(p) = &mut self.pipeline {
            keep.append(&mut p.tracked);
            p.tracked = keep;
        }
        match first_err {
            Some(e) if events.is_empty() => Err(e),
            _ => Ok(events),
        }
    }

    fn finish_import(
        &mut self,
        actor: &Actor,
        ticket_id: &str,
        snap: &JobSnapshot,
    ) -> Result<PumpEvent, ProjectError> {
        let mut row = self.job_store().get_ticket(ticket_id)?.ok_or_else(|| {
            ProjectError::invalid("TICKET_NOT_FOUND", format!("ticket {ticket_id} vanished"))
        })?;
        row.updated_ms = now_ms();
        let event = match snap.state {
            JobState::Completed => {
                let record = snap
                    .result
                    .clone()
                    .map(serde_json::from_value::<AssetRecord>)
                    .transpose()
                    .ok()
                    .flatten();
                match record {
                    None => {
                        let err =
                            JobError::new("JOB_RESULT_INVALID", "the hash job returned no record");
                        row.state = TicketState::Failed;
                        row.error = serde_json::to_value(&err).ok();
                        PumpEvent::ImportFailed {
                            ticket_id: ticket_id.to_owned(),
                            error: err,
                        }
                    }
                    Some(rec) => match self.commit_prepared(actor, rec, now_ms()) {
                        Ok(result) => {
                            row.state = TicketState::Finalized;
                            row.asset_id = Some(result.asset_id.to_string());
                            row.outcome = Some(json!({
                                "asset_id": result.asset_id.as_str(),
                                "outcome": result.outcome,
                            }));
                            PumpEvent::ImportFinalized {
                                ticket_id: ticket_id.to_owned(),
                                result: Box::new(result),
                            }
                        }
                        Err(e) => {
                            let err = JobError {
                                code: e.code(),
                                message: e.to_string(),
                                details: None,
                            };
                            row.state = TicketState::Failed;
                            row.error = serde_json::to_value(&err).ok();
                            PumpEvent::ImportFailed {
                                ticket_id: ticket_id.to_owned(),
                                error: err,
                            }
                        }
                    },
                }
            }
            JobState::Cancelled => {
                row.state = TicketState::Cancelled;
                PumpEvent::ImportCancelled {
                    ticket_id: ticket_id.to_owned(),
                }
            }
            JobState::Interrupted => {
                row.state = TicketState::Interrupted;
                PumpEvent::ImportInterrupted {
                    ticket_id: ticket_id.to_owned(),
                }
            }
            _ => {
                let err = snap
                    .error
                    .clone()
                    .unwrap_or_else(|| JobError::new("JOB_FAILED", "the hash job failed"));
                row.state = TicketState::Failed;
                row.error = serde_json::to_value(&err).ok();
                PumpEvent::ImportFailed {
                    ticket_id: ticket_id.to_owned(),
                    error: err,
                }
            }
        };
        self.job_store().put_ticket(&row)?;
        Ok(event)
    }

    // ---- derivados em background ----------------------------------------------------------------

    fn online_source(&self, id: &AssetId) -> Result<(AssetRecord, PathBuf), ProjectError> {
        let rec = self.managed(id)?;
        let (status, found) = quick_status(&rec, self.project_dir().as_deref());
        match found.filter(|_| status == Availability::Online) {
            Some(f) => Ok((rec, f)),
            None => Err(ProjectError::Asset(AssetError::new(
                AssetErrorCode::AssetOffline,
                format!("asset {id} is {}: no file to read", status.as_str()),
            ))),
        }
    }

    /// Falha rápido (no submit, não no job) se o asset não tem o stream que o derivado precisa.
    fn require_stream(rec: &AssetRecord, video: bool) -> Result<(), ProjectError> {
        let ok = if video {
            rec.media.video().is_some()
        } else {
            rec.media.audio().is_some()
        };
        if ok {
            Ok(())
        } else {
            Err(ProjectError::Asset(AssetError::new(
                AssetErrorCode::Media(capia_media::MediaErrorCode::MediaUnsupportedFormat),
                format!(
                    "asset {} has no {} stream",
                    rec.asset_id,
                    if video { "video" } else { "audio" }
                ),
            )))
        }
    }

    fn submit_derived<F>(
        &self,
        kind: JobKind,
        priority: Priority,
        label: String,
        dedup: String,
        params: Value,
        run: F,
    ) -> Result<Submitted, ProjectError>
    where
        F: FnOnce(&JobCtx, &MediaToolchain, &CacheDir) -> Result<Value, AssetError>
            + Send
            + 'static,
    {
        let p = self.pipeline_ref()?;
        let toolchain = p.toolchain.clone();
        let cache = self.cache_dir();
        let spec = JobSpec::new(kind, priority)
            .label(label)
            .dedup(dedup)
            .params(params);
        p.executor
            .submit(spec, move |ctx| {
                run(ctx, &toolchain, &cache).map_err(job_error)
            })
            .map_err(submit_error)
    }

    pub fn submit_frame_index(
        &self,
        id: &AssetId,
        priority: Priority,
    ) -> Result<Submitted, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        Self::require_stream(&rec, true)?;
        let dedup = derive::dedup_key_index(&rec);
        self.submit_derived(
            JobKind::FrameIndex,
            priority,
            format!("frame index {id}"),
            dedup,
            json!({ "asset_id": id.as_str() }),
            move |ctx, tc, cache| {
                let env = Env {
                    toolchain: tc,
                    cache,
                };
                let out = derive::ensure_frame_index(
                    &env,
                    &rec,
                    &file,
                    &|| ctx.is_cancelled(),
                    &mut |d, t| {
                        ctx.set_progress(d, t);
                    },
                )?;
                Ok(derived_result("frame_index", &out))
            },
        )
    }

    pub fn submit_waveform(
        &self,
        id: &AssetId,
        priority: Priority,
    ) -> Result<Submitted, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        Self::require_stream(&rec, false)?;
        let dedup = derive::dedup_key_waveform(&rec);
        self.submit_derived(
            JobKind::Waveform,
            priority,
            format!("waveform {id}"),
            dedup,
            json!({ "asset_id": id.as_str() }),
            move |ctx, tc, cache| {
                let env = Env {
                    toolchain: tc,
                    cache,
                };
                let out = derive::ensure_waveform(
                    &env,
                    &rec,
                    &file,
                    &|| ctx.is_cancelled(),
                    &mut |d, t| {
                        ctx.set_progress(d, t);
                    },
                )?;
                Ok(derived_result("waveform", &out))
            },
        )
    }

    pub fn submit_proxy(
        &self,
        id: &AssetId,
        profile: ProxyProfileV1,
        priority: Priority,
    ) -> Result<Submitted, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        Self::require_stream(&rec, true)?;
        profile
            .validate()
            .map_err(|e| ProjectError::Asset(e.into()))?;
        let dedup = derive::dedup_key_proxy(&rec, &profile);
        let params = json!({ "asset_id": id.as_str(), "profile": profile });
        self.submit_derived(
            JobKind::Proxy,
            priority,
            format!("proxy {id}"),
            dedup,
            params,
            move |ctx, tc, cache| {
                let env = Env {
                    toolchain: tc,
                    cache,
                };
                let out = derive::ensure_proxy(
                    &env,
                    &rec,
                    &file,
                    &profile,
                    None,
                    &|| ctx.is_cancelled(),
                    &mut |d, t| ctx.set_progress(d, t),
                )?;
                Ok(derived_result("proxy", &out))
            },
        )
    }

    pub fn submit_thumbnail(
        &self,
        id: &AssetId,
        at: Ticks,
        max_dim: u32,
        priority: Priority,
    ) -> Result<Submitted, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        let dedup = format!("thumb:{}:{}:{}", rec.content_hash, at.0, max_dim);
        self.submit_derived(
            JobKind::Thumbnail,
            priority,
            format!("thumbnail {id}"),
            dedup,
            json!({ "asset_id": id.as_str(), "at": at.0, "max_dim": max_dim }),
            move |ctx, tc, cache| {
                ctx.check()
                    .map_err(|_| AssetError::new(AssetErrorCode::AssetCancelled, "cancelled"))?;
                let path = capia_assets::ensure_thumbnail(tc, &rec, &file, at, max_dim, cache)?;
                Ok(json!({ "kind": "thumbnail", "path": path.display().to_string() }))
            },
        )
    }

    // ---- derivados síncronos (sem executor; o chamador escolhe a thread) ----------------------

    /// Fonte de quadros precisos (gera/lê o índice do cache e o mantém em memória).
    pub fn frame_source(
        &self,
        id: &AssetId,
        toolchain: &MediaToolchain,
        cancel: &dyn Fn() -> bool,
    ) -> Result<FrameSource, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        let cache = self.cache_dir();
        let env = Env {
            toolchain,
            cache: &cache,
        };
        Ok(derive::frame_source(&env, &rec, &file, cancel)?)
    }

    pub fn waveform(
        &self,
        id: &AssetId,
        toolchain: &MediaToolchain,
        cancel: &dyn Fn() -> bool,
    ) -> Result<capia_media::Waveform, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        let cache = self.cache_dir();
        let env = Env {
            toolchain,
            cache: &cache,
        };
        let out = derive::ensure_waveform(&env, &rec, &file, cancel, &mut |_, _| {})?;
        Ok(derive::load_waveform(&out.path)?)
    }

    pub fn proxy(
        &self,
        id: &AssetId,
        profile: &ProxyProfileV1,
        toolchain: &MediaToolchain,
        cancel: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        let cache = self.cache_dir();
        let env = Env {
            toolchain,
            cache: &cache,
        };
        Ok(derive::ensure_proxy(&env, &rec, &file, profile, None, cancel, &mut |_, _| {})?.path)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decode_audio(
        &self,
        id: &AssetId,
        start: Ticks,
        duration: Ticks,
        sample_rate: Option<u32>,
        channels: Option<u32>,
        toolchain: &MediaToolchain,
        cancel: &dyn Fn() -> bool,
    ) -> Result<capia_media::AudioPcm, ProjectError> {
        let (rec, file) = self.online_source(id)?;
        let cache = self.cache_dir();
        let env = Env {
            toolchain,
            cache: &cache,
        };
        Ok(derive::decode_pcm(
            &env,
            &rec,
            &file,
            start,
            duration,
            sample_rate,
            channels,
            cancel,
        )?)
    }

    // ---- relink em lote -------------------------------------------------------------------------

    fn relink_targets(&self, only: Option<&[AssetId]>) -> Result<Vec<RelinkTarget>, ProjectError> {
        let dir = self.project_dir();
        let mut out = Vec::new();
        for rec in self.catalog().list()? {
            let wanted = match only {
                Some(ids) => ids.contains(&rec.asset_id),
                None => quick_status(&rec, dir.as_deref()).0 != Availability::Online,
            };
            if wanted {
                out.push(RelinkTarget::from(&rec));
            }
        }
        Ok(out)
    }

    /// Relink em lote como **job**: varre `folder` (limites e política de links em `scan`), casa por
    /// tamanho → impressão rápida → SHA-256 e, em [`Project::pump`], aplica os que casaram de forma
    /// única. `only = None` ⇒ todos os assets offline/modificados.
    pub fn submit_batch_relink(
        &mut self,
        folder: &Path,
        scan: ScanOptions,
        only: Option<Vec<AssetId>>,
    ) -> Result<Submitted, ProjectError> {
        let targets = self.relink_targets(only.as_deref())?;
        let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
        let p = self.pipeline_ref()?;
        let spec = JobSpec::new(JobKind::BatchRelink, Priority::Normal)
            .label(format!("relink folder {}", folder.display()))
            .params(json!({ "folder": folder.display().to_string(), "assets": targets.len() }));
        let submitted = p
            .executor
            .submit(spec, move |ctx| {
                let cancel = || ctx.is_cancelled();
                let scanned = scan_folder(&folder, &scan, &cancel).map_err(job_error)?;
                let report = match_candidates(&targets, &scanned, &cancel, &mut |d, t| {
                    ctx.set_progress(d, t);
                })
                .map_err(job_error)?;
                serde_json::to_value(&report)
                    .map_err(|e| JobError::new("JOB_RESULT_INVALID", e.to_string()))
            })
            .map_err(submit_error)?;
        if let Some(p) = &mut self.pipeline {
            p.tracked.push(Tracked::Relink {
                handle: submitted.handle.clone(),
            });
        }
        Ok(submitted)
    }

    fn finish_relink(&mut self, snap: &JobSnapshot) -> Result<PumpEvent, ProjectError> {
        let job_id = snap.id.0.clone();
        if snap.state != JobState::Completed {
            let error = snap.error.clone().unwrap_or_else(|| {
                JobError::new(
                    "JOB_NOT_COMPLETED",
                    format!("the job ended {}", snap.state.as_str()),
                )
            });
            return Ok(PumpEvent::BatchRelinkFailed { job_id, error });
        }
        let report: BatchRelinkReport = snap
            .result
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| {
                ProjectError::invalid("JOB_RESULT_INVALID", "batch relink returned no report")
            })?;
        let (applied, apply_errors) = self.apply_batch_relink(&report)?;
        Ok(PumpEvent::BatchRelinkApplied {
            job_id,
            report: Box::new(report),
            applied,
            apply_errors,
        })
    }

    /// Aplica os casamentos **únicos** de um relatório: confere de novo (barato: tamanho +
    /// impressão) que o arquivo ainda é o verificado e atualiza a localização. Nada de ambíguo,
    /// rejeitado ou por nome é aplicado.
    pub(crate) fn apply_batch_relink(
        &mut self,
        report: &BatchRelinkReport,
    ) -> Result<(Vec<RelinkResult>, Vec<Value>), ProjectError> {
        let mut applied = Vec::new();
        let mut errors = Vec::new();
        for m in &report.matched {
            let rec = self.managed(&m.asset_id)?;
            let still = std::fs::metadata(&m.path).map(|x| x.len()).ok() == Some(rec.size_bytes)
                && rec.fingerprint.as_ref().is_none_or(|want| {
                    fingerprint_file(&m.path)
                        .map(|f| f.to_text())
                        .ok()
                        .as_deref()
                        == Some(want)
                });
            if !still {
                errors.push(json!({
                    "asset_id": m.asset_id.as_str(),
                    "path": m.path.display().to_string(),
                    "code": "ASSET_CHANGED_DURING_PROCESSING",
                    "message": "the matched file changed before the relink was applied",
                }));
                continue;
            }
            applied.push(self.apply_relink_unchecked(&m.asset_id, &m.path, "batch")?);
        }
        Ok((applied, errors))
    }

    /// Relink em lote **síncrono** (varre, casa e aplica na thread do chamador).
    pub fn batch_relink_folder(
        &mut self,
        folder: &Path,
        scan: &ScanOptions,
        only: Option<&[AssetId]>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(BatchRelinkReport, Vec<RelinkResult>, Vec<Value>), ProjectError> {
        let targets = self.relink_targets(only)?;
        let scanned = scan_folder(folder, scan, cancel)?;
        let report = match_candidates(&targets, &scanned, cancel, &mut |_, _| {})?;
        let (applied, errors) = self.apply_batch_relink(&report)?;
        Ok((report, applied, errors))
    }

    // ---- cache ----------------------------------------------------------------------------------

    pub fn cache_usage(&self) -> Result<capia_assets::CacheUsage, ProjectError> {
        Ok(self.cache_dir().usage()?)
    }

    /// `all = true` apaga todo o cache; senão remove só o que não pertence a nenhum asset do
    /// catálogo e os temporários velhos. O projeto continua válido nos dois casos.
    pub fn cache_clean(&self, all: bool) -> Result<capia_assets::GcReport, ProjectError> {
        let cache = self.cache_dir();
        if all {
            let usage = cache.usage()?;
            cache.clear()?;
            return Ok(capia_assets::GcReport {
                removed_files: usage.files + usage.temp_files,
                removed_bytes: usage.bytes + usage.temp_bytes,
            });
        }
        let live: std::collections::HashSet<String> = self
            .catalog()
            .list()?
            .iter()
            .map(|r| r.content_hash.hex()[..16].to_owned())
            .collect();
        let mut report = cache.remove_unused(&live)?;
        report.removed_files += cache.sweep_temp(Duration::from_secs(3600));
        Ok(report)
    }

    /// Invalida tudo que foi derivado de um conteúdo (força relink, arquivo modificado).
    pub fn invalidate_derived(
        &self,
        hash: &ContentHash,
    ) -> Result<capia_assets::GcReport, ProjectError> {
        Ok(self.cache_dir().invalidate_content(hash)?)
    }
}
