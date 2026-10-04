//! Codificação de export (ADR-067): quadros RGBA8 crus entram por um pipe no ffmpeg (sem arquivo
//! gigante intermediário), o áudio vem de um WAV, o resultado é MP4. A cadência é a da sequence,
//! expressa como racional exato; o encoder é SEMPRE um `EncoderCapability` aprovado.
//! A validação final é feita com o ffprobe: container, codec, dimensões, fps, nº de quadros,
//! áudio e sincronismo A/V.

use crate::encoder::{EncoderCapability, EncoderPolicy, ExportCodec};
use crate::error::{MediaError, MediaErrorCode};
use crate::ffprobe::{FfprobeBackend, MediaProbe, checked_input_path, file_url_arg};
use crate::process::{StreamLimits, run_collect};
use crate::toolchain::MediaToolchain;
use capia_time::{Rational, TICKS_PER_SECOND, Ticks};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Mp4Spec {
    pub width: u32,
    pub height: u32,
    /// Quadros por segundo (racional exato, ex.: 30000/1001).
    pub frame_rate: Rational,
    pub encoder: EncoderCapability,
    pub codec: ExportCodec,
    /// WAV (PCM float 32) do áudio já mixado; `None` ⇒ MP4 sem áudio.
    pub audio_wav: Option<PathBuf>,
    pub audio_bitrate_kbps: u32,
    pub video_bitrate_kbps: u32,
    pub gop: u32,
}

fn media_err(code: MediaErrorCode, msg: impl Into<String>) -> MediaError {
    MediaError::new(code, msg)
}

pub struct EncodeSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Arc<Mutex<Vec<u8>>>,
    frame_len: usize,
    written: u64,
    done: bool,
}

impl std::fmt::Debug for EncodeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodeSession")
            .field("written", &self.written)
            .finish_non_exhaustive()
    }
}

impl EncodeSession {
    pub fn start(tc: &MediaToolchain, spec: &Mp4Spec, out: &Path) -> Result<Self, MediaError> {
        let ffmpeg = tc.ffmpeg.as_deref().ok_or_else(|| {
            media_err(MediaErrorCode::MediaBackendNotFound, "ffmpeg was not found")
        })?;
        // defesa em profundidade: o encoder tem de ser aprovado, disponível e do codec pedido
        if spec.encoder.policy == EncoderPolicy::Prohibited {
            return Err(media_err(
                MediaErrorCode::MediaEncoderProhibited,
                format!("encoder `{}` is prohibited", spec.encoder.ffmpeg_name),
            ));
        }
        if !spec.encoder.available || spec.encoder.codec != spec.codec.codec_name() {
            return Err(media_err(
                MediaErrorCode::MediaEncoderUnavailable,
                format!(
                    "encoder `{}` is not an available {} encoder",
                    spec.encoder.ffmpeg_name,
                    spec.codec.codec_name()
                ),
            ));
        }
        if spec.width == 0 || spec.height == 0 || !spec.frame_rate.is_positive() {
            return Err(media_err(
                MediaErrorCode::MediaMetadataInvalid,
                "invalid export dimensions or frame rate",
            ));
        }
        let frame_len = (spec.width as usize)
            .checked_mul(spec.height as usize)
            .and_then(|p| p.checked_mul(4))
            .ok_or_else(|| media_err(MediaErrorCode::MediaLimitExceeded, "frame too large"))?;
        let pix = if spec.encoder.pixel_formats.is_empty()
            || spec.encoder.pixel_formats.iter().any(|p| p == "yuv420p")
        {
            "yuv420p".to_owned()
        } else {
            spec.encoder.pixel_formats[0].clone()
        };
        let fr = format!("{}/{}", spec.frame_rate.num(), spec.frame_rate.den());
        let mut args: Vec<OsString> = ["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"]
            .iter()
            .map(OsString::from)
            .collect();
        args.push("-s".into());
        args.push(format!("{}x{}", spec.width, spec.height).into());
        args.push("-framerate".into());
        args.push(fr.clone().into());
        args.push("-i".into());
        args.push("pipe:0".into());
        if let Some(wav) = &spec.audio_wav {
            let abs = checked_input_path(wav)?;
            args.push("-protocol_whitelist".into());
            args.push("file".into());
            args.push("-i".into());
            args.push(file_url_arg(&abs));
        }
        let mut rest: Vec<String> = vec!["-map".into(), "0:v:0".into()];
        if spec.audio_wav.is_some() {
            rest.extend(["-map".into(), "1:a:0".into()]);
        }
        rest.extend([
            "-vf".into(),
            format!("scale=out_color_matrix=bt709:out_range=tv,format={pix}"),
            "-c:v".into(),
            spec.encoder.ffmpeg_name.clone(),
            "-colorspace".into(),
            "bt709".into(),
            "-color_primaries".into(),
            "bt709".into(),
            "-color_trc".into(),
            "bt709".into(),
            "-color_range".into(),
            "tv".into(),
            "-g".into(),
            spec.gop.max(1).to_string(),
            "-fps_mode".into(),
            "cfr".into(),
            "-r".into(),
            fr,
        ]);
        if spec.codec == ExportCodec::Mpeg4Reference {
            rest.extend(["-q:v".into(), "3".into()]);
        } else if spec.video_bitrate_kbps > 0 {
            rest.extend(["-b:v".into(), format!("{}k", spec.video_bitrate_kbps)]);
        }
        if spec.audio_wav.is_some() {
            rest.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{}k", spec.audio_bitrate_kbps.max(32)),
            ]);
        }
        rest.extend([
            "-movflags".into(),
            "+faststart".into(),
            "-f".into(),
            "mp4".into(),
        ]);
        for a in rest {
            args.push(a.into());
        }
        args.push(out.as_os_str().to_owned());
        let mut cmd = Command::new(ffmpeg);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut tries = 0;
        let mut child = loop {
            match cmd.spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(media_err(
                        MediaErrorCode::MediaBackendFailed,
                        format!("cannot start ffmpeg: {e}"),
                    ));
                }
                Ok(c) => break c,
            }
        };
        let stdin = child.stdin.take();
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut err) = child.stderr.take() {
            let sink = Arc::clone(&stderr);
            std::thread::spawn(move || {
                use std::io::Read;
                let mut buf = [0u8; 4096];
                while let Ok(n) = err.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut g) = sink.lock()
                        && g.len() < 64 * 1024
                    {
                        g.extend_from_slice(&buf[..n]);
                    }
                }
            });
        }
        Ok(Self {
            child,
            stdin,
            stderr,
            frame_len,
            written: 0,
            done: false,
        })
    }

    fn stderr_text(&self) -> String {
        let g = self.stderr.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8_lossy(&g)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .take(240)
            .collect()
    }

    pub fn frames_written(&self) -> u64 {
        self.written
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        if rgba.len() != self.frame_len {
            return Err(media_err(
                MediaErrorCode::MediaMetadataInvalid,
                "frame size does not match the export size",
            ));
        }
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(media_err(
                MediaErrorCode::MediaEncodeFailed,
                "encoder closed",
            ));
        };
        if let Err(e) = stdin.write_all(rgba) {
            let why = self.stderr_text();
            self.abort();
            return Err(media_err(
                MediaErrorCode::MediaEncodeFailed,
                format!("the encoder stopped accepting frames ({e}): {why}"),
            ));
        }
        self.written += 1;
        Ok(())
    }

    /// Fecha a entrada e espera o ffmpeg terminar (cancelável; timeout mata).
    pub fn finish(
        mut self,
        timeout: Duration,
        cancel: &dyn Fn() -> bool,
    ) -> Result<u64, MediaError> {
        drop(self.stdin.take());
        let started = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(st)) => {
                    self.done = true;
                    return if st.success() {
                        Ok(self.written)
                    } else {
                        Err(media_err(
                            MediaErrorCode::MediaEncodeFailed,
                            format!("the encoder failed ({st}): {}", self.stderr_text()),
                        ))
                    };
                }
                Ok(None) => {}
                Err(e) => {
                    self.abort();
                    return Err(media_err(
                        MediaErrorCode::MediaBackendFailed,
                        format!("wait failed: {e}"),
                    ));
                }
            }
            if cancel() {
                self.abort();
                return Err(media_err(
                    MediaErrorCode::MediaCancelled,
                    "export cancelled",
                ));
            }
            if started.elapsed() >= timeout {
                self.abort();
                return Err(media_err(
                    MediaErrorCode::MediaProbeTimeout,
                    "the encoder did not finish in time and was killed",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Mata o ffmpeg. O chamador apaga o arquivo parcial.
    pub fn abort(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.done = true;
    }
}

impl Drop for EncodeSession {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

// ---- validação -------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ExportExpect {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    pub frames: u64,
    /// `(sample_rate, channels)`; `None` ⇒ sem áudio.
    pub audio: Option<(u32, u32)>,
    /// Tolerância de duração/sincronismo (ticks): ≤ 1 quadro.
    pub tolerance: Ticks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportValidation {
    pub container: Vec<String>,
    pub video_codec: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Option<Rational>,
    pub video_packets: u64,
    pub video_duration: Ticks,
    pub audio_duration: Option<Ticks>,
    /// `|duração de vídeo − duração de áudio|`.
    pub av_drift: Option<Ticks>,
    pub problems: Vec<String>,
}

impl ExportValidation {
    pub fn is_valid(&self) -> bool {
        self.problems.is_empty()
    }
}

fn count_packets(tc: &MediaToolchain, path: &Path, selector: &str) -> Result<u64, MediaError> {
    let abs = checked_input_path(path)?;
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-count_packets",
        "-select_streams",
        selector,
        "-show_entries",
        "stream=nb_read_packets",
        "-of",
        "csv=p=0",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(file_url_arg(&abs));
    let out = run_collect(
        &tc.ffprobe,
        &args,
        &StreamLimits::new(Duration::from_secs(120)),
        1 << 16,
        &|| false,
    )?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .map_err(|_| media_err(MediaErrorCode::MediaProbeFailed, "cannot count the packets"))
}

/// Valida o arquivo exportado com o ffprobe. Não executa nenhum decode do vídeo.
pub fn validate_export(
    tc: &MediaToolchain,
    path: &Path,
    expect: &ExportExpect,
) -> Result<ExportValidation, MediaError> {
    let info = FfprobeBackend::new(tc.clone()).probe(path)?;
    let mut problems = Vec::new();
    if !info.container.formats.iter().any(|f| f == "mp4") {
        problems.push(format!(
            "container is {:?}, not mp4",
            info.container.formats
        ));
    }
    let v = info.video();
    let (video_codec, width, height, frame_rate, video_duration) = match v {
        Some(v) => (
            v.codec.clone(),
            v.width,
            v.height,
            v.frame_rate,
            v.duration.or(info.duration).unwrap_or(Ticks(0)),
        ),
        None => {
            problems.push("no video stream".into());
            (String::new(), 0, 0, None, Ticks(0))
        }
    };
    if v.is_some() {
        if video_codec != expect.codec {
            problems.push(format!(
                "video codec is {video_codec}, expected {}",
                expect.codec
            ));
        }
        if (width, height) != (expect.width, expect.height) {
            problems.push(format!(
                "size is {width}x{height}, expected {}x{}",
                expect.width, expect.height
            ));
        }
        match frame_rate {
            Some(fr) if fr == expect.frame_rate => {}
            other => problems.push(format!(
                "frame rate is {other:?}, expected {:?}",
                expect.frame_rate
            )),
        }
    }
    let video_packets = if v.is_some() {
        count_packets(tc, path, "v:0")?
    } else {
        0
    };
    if v.is_some() && video_packets != expect.frames {
        problems.push(format!(
            "video has {video_packets} frames, expected {}",
            expect.frames
        ));
    }
    let expected_video = ticks_for_frames(expect.frames, expect.frame_rate).unwrap_or(Ticks(0));
    if v.is_some() && (video_duration.0 - expected_video.0).abs() > expect.tolerance.0 {
        problems.push(format!(
            "video duration is {} ticks, expected {} (±{})",
            video_duration.0, expected_video.0, expect.tolerance.0
        ));
    }
    let a = info.audio();
    let (audio_duration, av_drift) = match (a, expect.audio) {
        (Some(a), Some((rate, ch))) => {
            if (a.sample_rate, a.channels) != (rate, ch) {
                problems.push(format!(
                    "audio is {} Hz × {} ch, expected {rate} × {ch}",
                    a.sample_rate, a.channels
                ));
            }
            let d = a.duration.unwrap_or(Ticks(0));
            let drift = Ticks((video_duration.0 - d.0).abs());
            if drift.0 > expect.tolerance.0 {
                problems.push(format!(
                    "A/V drift is {} ticks (limit {})",
                    drift.0, expect.tolerance.0
                ));
            }
            (Some(d), Some(drift))
        }
        (None, Some(_)) => {
            problems.push("no audio stream".into());
            (None, None)
        }
        (Some(_), None) => {
            problems.push("unexpected audio stream".into());
            (None, None)
        }
        (None, None) => (None, None),
    };
    Ok(ExportValidation {
        container: info.container.formats.clone(),
        video_codec,
        width,
        height,
        frame_rate,
        video_packets,
        video_duration,
        audio_duration,
        av_drift,
        problems,
    })
}

fn ticks_for_frames(frames: u64, rate: Rational) -> Option<Ticks> {
    let n = i128::from(frames) * i128::from(rate.den()) * i128::from(TICKS_PER_SECOND);
    let t = n / i128::from(rate.num());
    i64::try_from(t).ok().map(Ticks)
}
