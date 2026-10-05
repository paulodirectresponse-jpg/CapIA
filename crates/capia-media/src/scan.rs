//! Varreduras para a inteligência (Fase 4, ADR-083): quadros **minúsculos** a taxa fixa (detecção de
//! cenas) e áudio mono comprimido por trechos (transcrição). Tudo por ffmpeg **sem shell**, com
//! caminho validado, timeout, teto de saída e cancelamento que mata o processo.

use crate::decode::ffmpeg_of;
use crate::error::{MediaError, MediaErrorCode};
use crate::ffprobe::{checked_input_path, file_url_arg};
use crate::process::{Flow, StreamLimits, run_collect, run_streaming};
use crate::toolchain::MediaToolchain;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// Callback por quadro de [`decode_small_frames`].
pub type FrameSink<'a> = &'a mut dyn FnMut(u64, &[u8]) -> Result<Flow, MediaError>;

/// Teto de quadros por varredura (≈ 5,5 h a 30 fps).
pub const MAX_SCAN_FRAMES: u64 = 600_000;

fn bad(msg: &str) -> MediaError {
    MediaError::new(MediaErrorCode::MediaMetadataInvalid, msg.to_owned())
}

fn base_args(abs: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    a.push(file_url_arg(abs));
    a
}

/// Decodifica o stream de vídeo `stream_index` como RGB24 `width`×`height` a **taxa constante**
/// `fps_num/fps_den` (filtro `fps`), entregando cada quadro a `on_frame(índice, bytes)`. O quadro
/// `n` ocorre em `n · fps_den / fps_num` segundos. Devolve o total de quadros.
#[allow(clippy::too_many_arguments)]
pub fn decode_small_frames(
    tc: &MediaToolchain,
    path: &Path,
    stream_index: u32,
    fps_num: u32,
    fps_den: u32,
    width: u32,
    height: u32,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
    on_frame: FrameSink<'_>,
) -> Result<u64, MediaError> {
    let ffmpeg = ffmpeg_of(tc)?;
    if !(8..=512).contains(&width) || !(8..=512).contains(&height) {
        return Err(bad("scan frame size must be within 8..=512"));
    }
    if fps_num == 0 || fps_den == 0 || u64::from(fps_num) > 240 * u64::from(fps_den) {
        return Err(bad("invalid scan frame rate"));
    }
    let abs = checked_input_path(path)?;
    let mut args = base_args(&abs);
    for a in [
        "-map".to_owned(),
        format!("0:{stream_index}"),
        "-an".into(),
        "-sn".into(),
        "-vf".into(),
        format!("fps={fps_num}/{fps_den},scale={width}:{height}:flags=area,format=rgb24"),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "pipe:1".into(),
    ] {
        args.push(a.into());
    }
    let frame_len = width as usize * height as usize * 3;
    let mut carry: Vec<u8> = Vec::with_capacity(frame_len * 2);
    let mut n: u64 = 0;
    let out = run_streaming(
        ffmpeg,
        &args,
        &StreamLimits::new(timeout),
        cancel,
        &mut |chunk| {
            carry.extend_from_slice(chunk);
            let mut off = 0;
            while carry.len() - off >= frame_len {
                if n >= MAX_SCAN_FRAMES {
                    return Err(MediaError::new(
                        MediaErrorCode::MediaLimitExceeded,
                        "the scan exceeds the frame limit",
                    ));
                }
                let flow = on_frame(n, &carry[off..off + frame_len])?;
                n += 1;
                off += frame_len;
                if flow == Flow::Stop {
                    return Ok(Flow::Stop);
                }
            }
            carry.drain(..off);
            Ok(Flow::Continue)
        },
    )?;
    match out.status {
        Some(s) if !s.success() => Err(MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            "ffmpeg failed while scanning the video",
        )),
        _ => Ok(n),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttAudioFormat {
    /// FLAC mono 16 kHz: sem perda e ~50% menor que o WAV.
    Flac,
    /// WAV PCM16 mono 16 kHz (sempre disponível).
    Wav,
}

impl SttAudioFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Flac => "audio/flac",
            Self::Wav => "audio/wav",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Wav => "wav",
        }
    }
}

/// Extrai `[start, start+duration)` do áudio como **mono 16 kHz** comprimido para enviar a um STT
/// (nunca o vídeo). `max_bytes` limita a saída.
#[allow(clippy::too_many_arguments)]
pub fn extract_audio_chunk(
    tc: &MediaToolchain,
    path: &Path,
    stream_index: u32,
    start: Ticks,
    duration: Ticks,
    format: SttAudioFormat,
    max_bytes: usize,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>, MediaError> {
    let ffmpeg = ffmpeg_of(tc)?;
    if start.0 < 0 || duration.0 <= 0 {
        return Err(bad("invalid audio interval"));
    }
    let abs = checked_input_path(path)?;
    let secs = |t: Ticks| {
        let micros = i128::from(t.0) * 1_000_000 / i128::from(TICKS_PER_SECOND);
        format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
    };
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-ss",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(secs(start).into());
    args.push("-t".into());
    args.push(secs(duration).into());
    args.push("-i".into());
    args.push(file_url_arg(&abs));
    let (codec, fmt) = match format {
        SttAudioFormat::Flac => ("flac", "flac"),
        SttAudioFormat::Wav => ("pcm_s16le", "wav"),
    };
    for a in [
        "-map".to_owned(),
        format!("0:{stream_index}"),
        "-vn".into(),
        "-sn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        "16000".into(),
        "-c:a".into(),
        codec.into(),
        "-f".into(),
        fmt.into(),
        "pipe:1".into(),
    ] {
        args.push(a.into());
    }
    let out = run_collect(
        ffmpeg,
        &args,
        &StreamLimits::new(timeout),
        max_bytes,
        cancel,
    )?;
    if !out.status.success() {
        return Err(MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            "ffmpeg failed while extracting the audio",
        ));
    }
    if out.stdout.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            "the audio interval is empty",
        ));
    }
    Ok(out.stdout)
}
