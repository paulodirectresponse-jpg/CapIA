//! Export headless (ADR-067/068): render de um range → (a) **intermediário** (RGBA cru + WAV +
//! manifesto com digests) ou (b) **MP4** por um `EncoderCapability` aprovado. Em ambos: escreve em
//! área de staging ao lado do destino, valida e só então publica com `rename` atômico. Matar o
//! processo em qualquer ponto nunca deixa um destino parcial apresentado como final.

use crate::error::ProjectError;
use crate::failpoints::fp;
use crate::project::Project;
use crate::render::RenderServices;
use capia_assets::hash_reader;
use capia_media::{
    EncodeSession, EncoderCapability, ExportCodec, ExportExpect, MediaError, Mp4Spec,
    detect_encoders, select_export_encoder, validate_export,
};
use capia_model::SequenceId;
use capia_render::{AudioBuffer, RenderSettings, RenderWarning, frame_digest};
use capia_time::{Rational, TICKS_PER_SECOND, Ticks, TimeRange};
use serde::Serialize;
use serde_json::json;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const INTERMEDIATE_FORMAT: &str = "capia-intermediate-v1";
const PARTIAL_TAG: &str = ".partial-";

fn inv(code: &'static str, msg: impl Into<String>) -> ProjectError {
    ProjectError::invalid(code, msg)
}

fn media(e: MediaError) -> ProjectError {
    ProjectError::Asset(e.into())
}

fn io(what: &str, e: &std::io::Error) -> ProjectError {
    inv("EXPORT_IO", format!("{what}: {e}"))
}

#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    /// Substitui um destino já existente (por padrão é erro).
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct Mp4Options {
    pub overwrite: bool,
    pub codec: ExportCodec,
    /// Nome do encoder do FFmpeg; `None` ⇒ o primeiro aprovado e disponível.
    pub encoder: Option<String>,
    /// Capacidades já detectadas (evita refazer os testes de encoder a cada export).
    pub capabilities: Option<Vec<EncoderCapability>>,
    pub audio_bitrate_kbps: u32,
    pub video_bitrate_kbps: u32,
    pub gop: u32,
}

impl Default for Mp4Options {
    fn default() -> Self {
        Self {
            overwrite: false,
            codec: ExportCodec::H264,
            encoder: None,
            capabilities: None,
            audio_bitrate_kbps: 192,
            video_bitrate_kbps: 8000,
            gop: 30,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct IntermediateReport {
    pub path: PathBuf,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub audio_frames: u64,
    pub video_sha256: String,
    pub audio_sha256: String,
    pub frame_digests: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Mp4Report {
    pub path: PathBuf,
    pub encoder: String,
    pub codec: String,
    pub hardware: bool,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub audio_frames: u64,
    pub av_drift_ticks: Option<i64>,
    pub video_duration_ticks: i64,
    pub warnings: Vec<String>,
}

fn warn_codes(w: &[RenderWarning]) -> Vec<String> {
    w.iter().map(|x| x.code.to_owned()).collect()
}

static STAGE: AtomicU64 = AtomicU64::new(0);

fn stage_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    let k = STAGE.fetch_add(1, Ordering::SeqCst);
    target.with_file_name(format!("{name}{PARTIAL_TAG}{}-{k}", std::process::id()))
}

/// Remove áreas de staging de exports anteriores interrompidos do mesmo destino.
pub fn clean_stale_partials(target: &Path) -> usize {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return 0;
    };
    let prefix = format!("{}{PARTIAL_TAG}", name.to_string_lossy());
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let p = e.path();
                let ok = if p.is_dir() {
                    remove_dir_retry(&p)
                } else {
                    remove_file_retry(&p)
                };
                if ok {
                    n += 1;
                }
            }
        }
    }
    n
}

fn remove_file_retry(p: &Path) -> bool {
    for _ in 0..40 {
        match std::fs::remove_file(p) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    false
}

fn remove_dir_retry(p: &Path) -> bool {
    for _ in 0..40 {
        match std::fs::remove_dir_all(p) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    false
}

fn sync_file(path: &Path) -> Result<(), ProjectError> {
    // fsync exige acesso de escrita no Windows
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| io("cannot open for sync", &e))?;
    f.sync_all().map_err(|e| io("cannot sync", &e))
}

/// Publica `staged` em `target` (rename atômico no mesmo volume).
fn publish(staged: &Path, target: &Path, overwrite: bool) -> Result<(), ProjectError> {
    if target.exists() {
        if !overwrite {
            return Err(inv(
                "EXPORT_EXISTS",
                format!("`{}` already exists (use overwrite)", target.display()),
            ));
        }
        let old = stage_path(target);
        std::fs::rename(target, &old).map_err(|e| io("cannot move the old export aside", &e))?;
        if let Err(e) = std::fs::rename(staged, target) {
            let _ = std::fs::rename(&old, target);
            return Err(io("cannot publish the export", &e));
        }
        if old.is_dir() {
            remove_dir_retry(&old);
        } else {
            remove_file_retry(&old);
        }
        return Ok(());
    }
    std::fs::rename(staged, target).map_err(|e| io("cannot publish the export", &e))
}

/// WAV IEEE float 32 bits, intercalado.
pub fn write_wav_f32(path: &Path, buf: &AudioBuffer) -> Result<(), ProjectError> {
    let data_len = u32::try_from(buf.samples.len() * 4)
        .map_err(|_| inv("EXPORT_TOO_LARGE", "the audio exceeds the WAV size limit"))?;
    let ch = u16::try_from(buf.channels).map_err(|_| inv("EXPORT_AUDIO", "too many channels"))?;
    let byte_rate = buf
        .sample_rate
        .checked_mul(u32::from(ch) * 4)
        .ok_or_else(|| inv("EXPORT_AUDIO", "byte rate overflows"))?;
    let mut w = BufWriter::new(File::create(path).map_err(|e| io("cannot create the WAV", &e))?);
    let mut put = |b: &[u8]| w.write_all(b).map_err(|e| io("cannot write the WAV", &e));
    put(b"RIFF")?;
    put(&(36 + data_len).to_le_bytes())?;
    put(b"WAVEfmt ")?;
    put(&16u32.to_le_bytes())?;
    put(&3u16.to_le_bytes())?; // IEEE float
    put(&ch.to_le_bytes())?;
    put(&buf.sample_rate.to_le_bytes())?;
    put(&byte_rate.to_le_bytes())?;
    put(&(ch * 4).to_le_bytes())?;
    put(&32u16.to_le_bytes())?;
    put(b"data")?;
    put(&data_len.to_le_bytes())?;
    for s in &buf.samples {
        put(&s.to_le_bytes())?;
    }
    w.flush().map_err(|e| io("cannot flush the WAV", &e))
}

/// Quadros `[first, last_excl)` da cadência `fd` (ticks) que caem em `range`.
fn frame_span(range: TimeRange, fd: i64) -> Result<(i64, i64), ProjectError> {
    let end = range
        .end()
        .ok_or_else(|| inv("RENDER_TIME_OVERFLOW", "range end overflows"))?;
    if fd <= 0 || range.start.0 < 0 || end.0 < range.start.0 {
        return Err(inv(
            "RENDER_RANGE_INVALID",
            "range must satisfy 0 ≤ start ≤ end",
        ));
    }
    let first = (range.start.0 + fd - 1) / fd;
    let last_excl = (end.0 + fd - 1) / fd;
    Ok((first, last_excl.max(first)))
}

struct Plan {
    frame_rate: capia_time::FrameRate,
    first: i64,
    count: u64,
    audio_range: TimeRange,
}

fn plan(
    project: &Project,
    seq: &SequenceId,
    range: TimeRange,
    settings: &RenderSettings,
) -> Result<Plan, ProjectError> {
    let graph = project.render_graph(seq)?;
    let gs = graph.sequence(seq).map_err(|e| inv(e.code, e.message))?;
    let rate = settings.frame_rate.unwrap_or(gs.frame_rate);
    let fd = rate.frame_duration().0;
    let (first, last) = frame_span(range, fd)?;
    let count = u64::try_from(last - first).map_err(|_| inv("RENDER_RANGE_INVALID", "bad span"))?;
    let start = first
        .checked_mul(fd)
        .ok_or_else(|| inv("RENDER_TIME_OVERFLOW", "start overflows"))?;
    let dur = i64::try_from(count)
        .ok()
        .and_then(|c| c.checked_mul(fd))
        .ok_or_else(|| inv("RENDER_TIME_OVERFLOW", "duration overflows"))?;
    Ok(Plan {
        frame_rate: rate,
        first,
        count,
        audio_range: TimeRange::new(Ticks(start), Ticks(dur)),
    })
}

impl Project {
    /// Export intermediário atômico: `<out>/{video.rgba, audio.wav, manifest.json}`.
    pub fn export_intermediate(
        &self,
        services: &Arc<RenderServices>,
        seq: &SequenceId,
        range: TimeRange,
        settings: &RenderSettings,
        out: &Path,
        opts: &ExportOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<IntermediateReport, ProjectError> {
        settings.validate().map_err(|e| inv(e.code, e.message))?;
        let p = plan(self, seq, range, settings)?;
        if out.exists() && !opts.overwrite {
            return Err(inv(
                "EXPORT_EXISTS",
                format!("`{}` already exists (use overwrite)", out.display()),
            ));
        }
        clean_stale_partials(out);
        let stage = stage_path(out);
        std::fs::create_dir_all(&stage).map_err(|e| io("cannot create the staging area", &e))?;
        let result = (|| -> Result<IntermediateReport, ProjectError> {
            let video_path = stage.join("video.rgba");
            let mut vw = BufWriter::new(
                File::create(&video_path).map_err(|e| io("cannot create video.rgba", &e))?,
            );
            let mut digests = Vec::new();
            let mut write_err: Option<ProjectError> = None;
            let mut cancelled = false;
            let mut warnings: Vec<RenderWarning> = Vec::new();
            let mut s = *settings;
            s.frame_rate = Some(p.frame_rate);
            let vrange = TimeRange::new(p.audio_range.start, p.audio_range.duration);
            let (n, w) = self.render_range(services, seq, vrange, &s, &mut |_, _, img| {
                if cancel() {
                    cancelled = true;
                    return false;
                }
                if digests.is_empty() {
                    fp!("export_frames_running");
                }
                digests.push(frame_digest(&img));
                if let Err(e) = vw.write_all(&img.data) {
                    write_err = Some(io("cannot write video.rgba", &e));
                    return false;
                }
                true
            })?;
            warnings.extend(w);
            if let Some(e) = write_err {
                return Err(e);
            }
            if cancelled {
                return Err(inv("EXPORT_CANCELLED", "the export was cancelled"));
            }
            vw.flush().map_err(|e| io("cannot flush video.rgba", &e))?;
            drop(vw);
            if n != p.count {
                return Err(inv(
                    "EXPORT_FRAME_COUNT",
                    format!("rendered {n} of {} frames", p.count),
                ));
            }
            let (audio, aw) = self.render_audio_range(services, seq, p.audio_range, &s)?;
            warnings.extend(aw);
            warnings.sort();
            warnings.dedup();
            let wav_path = stage.join("audio.wav");
            write_wav_f32(&wav_path, &audio)?;
            let audio_frames = (audio.samples.len() / audio.channels.max(1) as usize) as u64;
            sync_file(&video_path)?;
            sync_file(&wav_path)?;
            let vd = hash_reader(File::open(&video_path).map_err(|e| io("cannot reopen", &e))?)
                .map_err(|e| io("cannot hash video.rgba", &e))?;
            let ad = hash_reader(File::open(&wav_path).map_err(|e| io("cannot reopen", &e))?)
                .map_err(|e| io("cannot hash audio.wav", &e))?;
            let frame_len = u64::from(settings.width) * u64::from(settings.height) * 4;
            // validação do que vai ser publicado
            if vd.size != frame_len * p.count {
                return Err(inv("EXPORT_VALIDATION", "video.rgba has the wrong size"));
            }
            if ad.size != 44 + audio.samples.len() as u64 * 4 {
                return Err(inv("EXPORT_VALIDATION", "audio.wav has the wrong size"));
            }
            let manifest = json!({
                "format": INTERMEDIATE_FORMAT,
                "sequence": seq.as_str(),
                "range_start_ticks": p.audio_range.start.0,
                "range_duration_ticks": p.audio_range.duration.0,
                "first_frame": p.first,
                "frame_rate": { "num": p.frame_rate.rate().num(), "den": p.frame_rate.rate().den() },
                "frames": p.count,
                "width": settings.width,
                "height": settings.height,
                "pixel_format": "rgba8",
                "video_sha256": vd.hash.as_str(),
                "audio": {
                    "sample_rate": audio.sample_rate,
                    "channels": audio.channels,
                    "frames": audio_frames,
                    "format": "f32le-wav",
                    "sha256": ad.hash.as_str(),
                },
                "frame_digests": digests,
                "warnings": warn_codes(&warnings),
            });
            let mpath = stage.join("manifest.json");
            std::fs::write(
                &mpath,
                serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
            )
            .map_err(|e| io("cannot write the manifest", &e))?;
            sync_file(&mpath)?;
            fp!("export_before_publish");
            Ok(IntermediateReport {
                path: out.to_path_buf(),
                frames: p.count,
                width: settings.width,
                height: settings.height,
                audio_frames,
                video_sha256: vd.hash.as_str().to_owned(),
                audio_sha256: ad.hash.as_str().to_owned(),
                frame_digests: digests,
                warnings: warn_codes(&warnings),
            })
        })();
        match result {
            Ok(report) => {
                publish(&stage, out, opts.overwrite).inspect_err(|_| {
                    remove_dir_retry(&stage);
                })?;
                Ok(report)
            }
            Err(e) => {
                remove_dir_retry(&stage);
                Err(e)
            }
        }
    }

    /// Detecta os encoders (testes reais de codificação). Chame uma vez e reutilize.
    pub fn detect_export_encoders(
        services: &RenderServices,
    ) -> Result<Vec<EncoderCapability>, ProjectError> {
        detect_encoders(services.toolchain()).map_err(media)
    }

    /// Export MP4: render → encoder aprovado → validação por ffprobe → publicação atômica.
    #[allow(clippy::too_many_arguments)]
    pub fn export_mp4(
        &self,
        services: &Arc<RenderServices>,
        seq: &SequenceId,
        range: TimeRange,
        settings: &RenderSettings,
        out: &Path,
        opts: &Mp4Options,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Mp4Report, ProjectError> {
        settings.validate().map_err(|e| inv(e.code, e.message))?;
        if settings.width % 2 != 0 || settings.height % 2 != 0 {
            return Err(inv("EXPORT_SIZE", "MP4 export needs even width and height"));
        }
        let p = plan(self, seq, range, settings)?;
        if p.count == 0 {
            return Err(inv("EXPORT_EMPTY", "the range has no frames"));
        }
        if out.exists() && !opts.overwrite {
            return Err(inv(
                "EXPORT_EXISTS",
                format!("`{}` already exists (use overwrite)", out.display()),
            ));
        }
        let caps = match &opts.capabilities {
            Some(c) => c.clone(),
            None => detect_encoders(services.toolchain()).map_err(media)?,
        };
        let enc = select_export_encoder(&caps, opts.codec, opts.encoder.as_deref())
            .map_err(media)?
            .clone();
        clean_stale_partials(out);
        let stage = stage_path(out);
        std::fs::create_dir_all(&stage).map_err(|e| io("cannot create the staging area", &e))?;
        let partial = stage.join("out.mp4");
        let result = (|| -> Result<Mp4Report, ProjectError> {
            let mut s = *settings;
            s.frame_rate = Some(p.frame_rate);
            let (audio, aw) = self.render_audio_range(services, seq, p.audio_range, &s)?;
            let wav = stage.join("audio.wav");
            write_wav_f32(&wav, &audio)?;
            let audio_frames = (audio.samples.len() / audio.channels.max(1) as usize) as u64;
            let fr = p.frame_rate.rate();
            let spec = Mp4Spec {
                width: settings.width,
                height: settings.height,
                frame_rate: Rational::new(fr.num(), fr.den())
                    .map_err(|e| inv("EXPORT_TIME", e.to_string()))?,
                encoder: enc.clone(),
                codec: opts.codec,
                audio_wav: Some(wav),
                audio_bitrate_kbps: opts.audio_bitrate_kbps,
                video_bitrate_kbps: opts.video_bitrate_kbps,
                gop: opts.gop,
            };
            let mut session =
                EncodeSession::start(services.toolchain(), &spec, &partial).map_err(media)?;
            let mut err: Option<ProjectError> = None;
            let mut cancelled = false;
            let mut first = true;
            let render = self.render_range(services, seq, p.audio_range, &s, &mut |_, _, img| {
                if cancel() {
                    cancelled = true;
                    return false;
                }
                if first {
                    first = false;
                    fp!("export_frames_running");
                }
                if let Err(e) = session.write_frame(&img.data) {
                    err = Some(media(e));
                    return false;
                }
                true
            });
            let (n, vw) = match render {
                Ok(v) => v,
                Err(e) => {
                    session.abort();
                    return Err(e);
                }
            };
            if let Some(e) = err {
                session.abort();
                return Err(e);
            }
            if cancelled {
                session.abort();
                return Err(inv("EXPORT_CANCELLED", "the export was cancelled"));
            }
            if n != p.count {
                session.abort();
                return Err(inv(
                    "EXPORT_FRAME_COUNT",
                    format!("rendered {n} of {} frames", p.count),
                ));
            }
            session
                .finish(Duration::from_secs(3600), cancel)
                .map_err(media)?;
            let tol = p.frame_rate.frame_duration();
            let expect = ExportExpect {
                codec: opts.codec.codec_name().to_owned(),
                width: settings.width,
                height: settings.height,
                frame_rate: Rational::new(fr.num(), fr.den())
                    .map_err(|e| inv("EXPORT_TIME", e.to_string()))?,
                frames: p.count,
                audio: Some((settings.audio_sample_rate, settings.audio_channels)),
                tolerance: tol,
            };
            let v = validate_export(services.toolchain(), &partial, &expect).map_err(media)?;
            if !v.is_valid() {
                return Err(ProjectError::Invalid {
                    code: "EXPORT_VALIDATION_FAILED",
                    message: v.problems.join("; "),
                    details: None,
                });
            }
            sync_file(&partial)?;
            fp!("export_before_publish");
            let mut warnings = aw;
            warnings.extend(vw);
            warnings.sort();
            warnings.dedup();
            Ok(Mp4Report {
                path: out.to_path_buf(),
                encoder: enc.ffmpeg_name.clone(),
                codec: enc.codec.clone(),
                hardware: enc.hardware,
                frames: p.count,
                width: settings.width,
                height: settings.height,
                audio_frames,
                av_drift_ticks: v.av_drift.map(|t| t.0),
                video_duration_ticks: v.video_duration.0,
                warnings: warn_codes(&warnings),
            })
        })();
        match result {
            Ok(report) => {
                let done = publish(&partial, out, opts.overwrite);
                remove_dir_retry(&stage);
                done?;
                Ok(report)
            }
            Err(e) => {
                remove_dir_retry(&stage);
                Err(e)
            }
        }
    }
}

/// Segundos de uma duração em ticks (só para mensagens/relatórios; nunca entra no modelo).
pub fn ticks_to_seconds_text(t: Ticks) -> String {
    let us = i128::from(t.0) * 1_000_000 / i128::from(TICKS_PER_SECOND);
    format!("{}.{:06}", us / 1_000_000, (us % 1_000_000).abs())
}
