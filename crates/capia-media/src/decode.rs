//! Decodificação **precisa** de quadros (ADR-054) e de áudio PCM (ADR-055).
//!
//! Vídeo: o [`FrameIndex`] diz qual é o quadro lógico e qual keyframe o precede; o ffmpeg
//! busca o keyframe, decodifica para frente e o filtro `select=eq(pts,N)` entrega **exatamente** o
//! quadro com aquele PTS (nada de `tempo × fps`). Saída: RGBA8 cru com limites verificados.
//!
//! Áudio: decodifica do início do stream (exato por construção), reamostra/remixa para f32
//! intercalado, descarta até `start` e para o processo assim que tem as amostras pedidas.

use crate::error::{MediaError, MediaErrorCode};
use crate::ffprobe::{checked_input_path, file_url_arg};
use crate::index::FrameIndex;
use crate::process::{Flow, StreamLimits, run_collect, run_streaming};
use crate::toolchain::MediaToolchain;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// Teto padrão de um quadro cru (256 MiB ≈ 8K RGBA).
pub const DEFAULT_MAX_FRAME_BYTES: u64 = 256 * 1024 * 1024;
/// Teto padrão de PCM numa chamada (≈ 1 h de estéreo 48 kHz em f32 = 1,4 GB é demais: 512 MiB).
pub const DEFAULT_MAX_PCM_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    Rgba8,
}

/// Quadro decodificado: linhas de `stride` bytes (aqui `width × 4`, sem *padding*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub pixel_format: PixelFormat,
    /// PTS real do quadro (unidades do *time base* do índice).
    pub pts: i64,
    /// Índice lógico no [`FrameIndex`].
    pub index: usize,
    /// Tempo (ticks, relativo ao primeiro quadro).
    pub time: Ticks,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct DecodeLimits {
    pub max_frame_bytes: u64,
    pub timeout: Duration,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            timeout: Duration::from_secs(120),
        }
    }
}

pub(crate) fn ffmpeg_of(tc: &MediaToolchain) -> Result<&Path, MediaError> {
    tc.ffmpeg.as_deref().ok_or_else(|| {
        MediaError::new(
            MediaErrorCode::MediaBackendNotFound,
            "ffmpeg was not found (needed to decode)",
        )
    })
}

/// `Ticks` → segundos com 6 casas por aritmética inteira (arredondando para **cima** quando
/// `ceil`, senão para baixo). Negativos viram 0.
pub(crate) fn seconds_arg(t: Ticks, ceil: bool) -> String {
    let n = i128::from(t.0.max(0)) * 1_000_000;
    let d = i128::from(TICKS_PER_SECOND);
    let micros = if ceil { (n + d - 1) / d } else { n / d };
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// Tempo absoluto (em ticks, **sem** subtrair o início do índice) de um PTS cru.
pub(crate) fn absolute_ticks(index: &FrameIndex, pts: i64) -> Ticks {
    let tb = index.time_base();
    let num = i128::from(pts) * i128::from(tb.num()) * i128::from(TICKS_PER_SECOND);
    let den = i128::from(tb.den());
    let t = num.div_euclid(den);
    Ticks(i64::try_from(t).unwrap_or(i64::MAX))
}

pub(crate) fn frame_len(
    width: u32,
    height: u32,
    limits: &DecodeLimits,
) -> Result<usize, MediaError> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|p| p.checked_mul(4))
        .filter(|b| *b > 0 && *b <= limits.max_frame_bytes)
        .ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::MediaLimitExceeded,
                format!("{width}x{height} exceeds the decoded-frame limit"),
            )
        })?;
    usize::try_from(bytes).map_err(|_| {
        MediaError::new(
            MediaErrorCode::MediaLimitExceeded,
            "frame does not fit in memory",
        )
    })
}

/// Decodifica o quadro lógico `frame` do índice (`width × height` = dimensões **codificadas** do
/// stream; a rotação de exibição é metadado, não é aplicada).
#[allow(clippy::too_many_arguments)]
pub fn decode_frame_by_index(
    tc: &MediaToolchain,
    path: &Path,
    index: &FrameIndex,
    width: u32,
    height: u32,
    frame: usize,
    limits: &DecodeLimits,
    cancel: &dyn Fn() -> bool,
) -> Result<RawFrame, MediaError> {
    let ffmpeg = ffmpeg_of(tc)?;
    let entry = *index.frame_by_index(frame).ok_or_else(|| {
        MediaError::new(
            MediaErrorCode::MediaFrameNotFound,
            format!("frame {frame} is outside the index"),
        )
    })?;
    let len = frame_len(width, height, limits)?;
    let abs = checked_input_path(path)?;
    let kf = index.keyframe_before(frame).unwrap_or(0);
    // 1ª tentativa: no keyframe (arredondado p/ cima); 2ª: um keyframe antes; 3ª: do início
    let mut starts: Vec<Option<usize>> = vec![Some(kf)];
    if let Some(prev) = kf.checked_sub(1).and_then(|k| index.keyframe_before(k)) {
        starts.push(Some(prev));
    }
    starts.push(None);
    for start in starts {
        let mut args: Vec<OsString> = [
            "-v",
            "error",
            "-nostdin",
            "-noautorotate",
            "-protocol_whitelist",
            "file",
            "-seek_timestamp",
            "1",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        if let Some(k) = start {
            let t = absolute_ticks(index, index.entries()[k].pts);
            args.push("-ss".into());
            args.push(seconds_arg(t, false).into());
        }
        args.push("-copyts".into());
        args.push("-i".into());
        args.push(file_url_arg(&abs));
        let filter = format!("select=eq(pts\\,{})", entry.pts);
        for a in [
            "-map".to_owned(),
            format!("0:{}", index.stream_index()),
            "-an".into(),
            "-sn".into(),
            "-vf".into(),
            filter,
            "-fps_mode".into(),
            "passthrough".into(),
            "-frames:v".into(),
            "1".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "pipe:1".into(),
        ] {
            args.push(a.into());
        }
        let out = run_collect(
            ffmpeg,
            &args,
            &StreamLimits::new(limits.timeout),
            len,
            cancel,
        );
        let out = match out {
            Ok(o) => o,
            Err(e) if e.code == MediaErrorCode::MediaLimitExceeded => {
                return Err(MediaError::new(
                    MediaErrorCode::MediaDecodeFailed,
                    "the decoder produced more data than one frame (wrong dimensions?)",
                ));
            }
            Err(e) => return Err(e),
        };
        if out.status.success() && out.stdout.len() == len {
            return Ok(RawFrame {
                width,
                height,
                stride: width as usize * 4,
                pixel_format: PixelFormat::Rgba8,
                pts: entry.pts,
                index: frame,
                time: index.time_of(frame).unwrap_or(Ticks(0)),
                bytes: out.stdout,
            });
        }
        if !out.stdout.is_empty() && out.stdout.len() != len {
            return Err(MediaError::new(
                MediaErrorCode::MediaDecodeFailed,
                "the decoder returned a partial frame",
            ));
        }
    }
    Err(MediaError::new(
        MediaErrorCode::MediaFrameNotFound,
        format!(
            "the decoder never produced the frame with pts {}",
            entry.pts
        ),
    ))
}

/// Decodifica `count` quadros **consecutivos** (apresentação) a partir do quadro lógico `first`
/// com **um único** processo do ffmpeg (scrub/reprodução: evita o custo de um processo por quadro).
/// `on_frame` recebe cada quadro em ordem; devolver `Flow::Stop` encerra cedo (mata o ffmpeg).
/// Devolve quantos quadros entregou. O intervalo `[pts(first), pts(last)]` do índice contém
/// exatamente esses quadros (o índice é ordenado por PTS), então `select=between(pts,…)` os
/// seleciona sem contar nada. Teto: 512 quadros e `count × frame` ≤ 4 × `max_frame_bytes`.
#[allow(clippy::too_many_arguments)]
pub fn decode_frame_range(
    tc: &MediaToolchain,
    path: &Path,
    index: &FrameIndex,
    width: u32,
    height: u32,
    first: usize,
    count: usize,
    limits: &DecodeLimits,
    cancel: &dyn Fn() -> bool,
    on_frame: &mut dyn FnMut(RawFrame) -> Flow,
) -> Result<usize, MediaError> {
    let ffmpeg = ffmpeg_of(tc)?;
    let bad = |m: String| MediaError::new(MediaErrorCode::MediaFrameNotFound, m);
    if count == 0 || count > 512 {
        return Err(MediaError::new(
            MediaErrorCode::MediaLimitExceeded,
            "a frame range must have 1..=512 frames",
        ));
    }
    let last = first
        .checked_add(count - 1)
        .filter(|l| *l < index.len())
        .ok_or_else(|| bad(format!("frames {first}+{count} are outside the index")))?;
    let len = frame_len(width, height, limits)?;
    if (len as u64).saturating_mul(count as u64) > limits.max_frame_bytes.saturating_mul(4) {
        return Err(MediaError::new(
            MediaErrorCode::MediaLimitExceeded,
            "the frame range exceeds the byte limit",
        ));
    }
    let (pf, pl) = (index.entries()[first].pts, index.entries()[last].pts);
    let abs = checked_input_path(path)?;
    let kf = index.keyframe_before(first).unwrap_or(0);
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-nostdin",
        "-noautorotate",
        "-protocol_whitelist",
        "file",
        "-seek_timestamp",
        "1",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let t = absolute_ticks(index, index.entries()[kf].pts);
    args.push("-ss".into());
    args.push(seconds_arg(t, false).into());
    args.push("-copyts".into());
    args.push("-i".into());
    args.push(file_url_arg(&abs));
    for a in [
        "-map".to_owned(),
        format!("0:{}", index.stream_index()),
        "-an".into(),
        "-sn".into(),
        "-vf".into(),
        format!("select=between(pts\\,{pf}\\,{pl})"),
        "-fps_mode".into(),
        "passthrough".into(),
        "-frames:v".into(),
        count.to_string(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "pipe:1".into(),
    ] {
        args.push(a.into());
    }
    let mut carry: Vec<u8> = Vec::with_capacity(len);
    let mut delivered = 0usize;
    let out = run_streaming(
        ffmpeg,
        &args,
        &StreamLimits::new(limits.timeout),
        cancel,
        &mut |chunk| {
            carry.extend_from_slice(chunk);
            while carry.len() >= len && delivered < count {
                let bytes: Vec<u8> = carry.drain(..len).collect();
                let i = first + delivered;
                delivered += 1;
                let frame = RawFrame {
                    width,
                    height,
                    stride: width as usize * 4,
                    pixel_format: PixelFormat::Rgba8,
                    pts: index.entries()[i].pts,
                    index: i,
                    time: index.time_of(i).unwrap_or(Ticks(0)),
                    bytes,
                };
                if on_frame(frame) == Flow::Stop {
                    return Ok(Flow::Stop);
                }
            }
            Ok(if delivered >= count {
                Flow::Stop
            } else {
                Flow::Continue
            })
        },
    )?;
    if !out.stopped_early && delivered < count {
        return Err(MediaError::new(
            MediaErrorCode::MediaFrameNotFound,
            format!("the decoder produced {delivered} of {count} frames"),
        ));
    }
    Ok(delivered)
}

/// Decodifica o quadro **em ou antes de** `at` (o que o preview mostra no tempo `at`).
#[allow(clippy::too_many_arguments)]
pub fn decode_frame_at(
    tc: &MediaToolchain,
    path: &Path,
    index: &FrameIndex,
    width: u32,
    height: u32,
    at: Ticks,
    limits: &DecodeLimits,
    cancel: &dyn Fn() -> bool,
) -> Result<RawFrame, MediaError> {
    let i = index.frame_at_or_before(at).ok_or_else(|| {
        MediaError::new(
            MediaErrorCode::MediaFrameNotFound,
            "the time is before the first frame",
        )
    })?;
    decode_frame_by_index(tc, path, index, width, height, i, limits, cancel)
}

// ---------------------------------------------------------------------------------------------
// Áudio
// ---------------------------------------------------------------------------------------------

/// PCM f32 **intercalado** normalizado; posições em amostras por canal.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioPcm {
    pub sample_rate: u32,
    pub channels: u32,
    /// Primeira amostra (por canal) deste trecho, contada do início do stream.
    pub start_sample: u64,
    /// Nº de amostras por canal devolvidas (pode ser menor que o pedido no fim da mídia).
    pub frames: u64,
    pub samples: Vec<f32>,
}

impl AudioPcm {
    /// Duração do trecho em ticks (inteiro; arredonda para baixo).
    pub fn duration(&self) -> Ticks {
        samples_to_ticks(self.frames, self.sample_rate)
    }
}

/// `n` amostras a `rate` Hz → ticks (floor). 705.600.000 é múltiplo de 44.100 e 48.000: exato.
pub fn samples_to_ticks(n: u64, rate: u32) -> Ticks {
    let t = u128::from(n) * TICKS_PER_SECOND as u128 / u128::from(rate.max(1));
    Ticks(i64::try_from(t).unwrap_or(i64::MAX))
}

/// Ticks → índice de amostra (floor) a `rate` Hz.
pub fn ticks_to_samples(t: Ticks, rate: u32) -> u64 {
    let n = i128::from(t.0.max(0)) * i128::from(rate);
    (n / i128::from(TICKS_PER_SECOND)) as u64
}

#[derive(Clone, Debug)]
pub struct AudioRequest {
    /// Índice **absoluto** do stream de áudio.
    pub stream_index: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub start: Ticks,
    pub duration: Ticks,
}

fn audio_args(abs: &Path, stream_index: u32, sample_rate: u32, channels: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
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
    args.push(file_url_arg(abs));
    for a in [
        "-map".to_owned(),
        format!("0:{stream_index}"),
        "-vn".into(),
        "-sn".into(),
        "-ar".into(),
        sample_rate.to_string(),
        "-ac".into(),
        channels.to_string(),
        "-f".into(),
        "f32le".into(),
        "-acodec".into(),
        "pcm_f32le".into(),
        "pipe:1".into(),
    ] {
        args.push(a.into());
    }
    args
}

fn check_audio_shape(sample_rate: u32, channels: u32) -> Result<(), MediaError> {
    if !(8_000..=crate::limits::MAX_SAMPLE_RATE).contains(&sample_rate)
        || !(1..=crate::limits::MAX_CHANNELS).contains(&channels)
    {
        return Err(MediaError::new(
            MediaErrorCode::MediaMetadataInvalid,
            "unsupported sample rate or channel count",
        ));
    }
    Ok(())
}

/// Decodifica `[start, start+duration)` como f32 intercalado.
pub fn decode_audio(
    tc: &MediaToolchain,
    path: &Path,
    req: &AudioRequest,
    max_bytes: u64,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<AudioPcm, MediaError> {
    let ffmpeg = ffmpeg_of(tc)?;
    check_audio_shape(req.sample_rate, req.channels)?;
    if req.start.0 < 0 || req.duration.0 <= 0 {
        return Err(MediaError::new(
            MediaErrorCode::MediaMetadataInvalid,
            "invalid audio interval",
        ));
    }
    let start = ticks_to_samples(req.start, req.sample_rate);
    let end_ticks = req.start.0.checked_add(req.duration.0).ok_or_else(|| {
        MediaError::new(MediaErrorCode::MediaMetadataInvalid, "interval overflows")
    })?;
    // fim = ceil(end) para não perder a última amostra parcial; início = floor
    let end = {
        let n = i128::from(end_ticks) * i128::from(req.sample_rate);
        let d = i128::from(TICKS_PER_SECOND);
        ((n + d - 1) / d) as u64
    };
    let want = end.saturating_sub(start);
    let frame_bytes = u64::from(req.channels) * 4;
    if want.checked_mul(frame_bytes).is_none_or(|b| b > max_bytes) {
        return Err(MediaError::new(
            MediaErrorCode::MediaLimitExceeded,
            "the requested PCM exceeds the byte limit",
        ));
    }
    let abs = checked_input_path(path)?;
    let args = audio_args(&abs, req.stream_index, req.sample_rate, req.channels);
    let skip_bytes = start
        .checked_mul(frame_bytes)
        .ok_or_else(|| MediaError::new(MediaErrorCode::MediaLimitExceeded, "start overflows"))?;
    let take_bytes = want * frame_bytes;
    let mut seen: u64 = 0;
    let mut raw: Vec<u8> =
        Vec::with_capacity(usize::try_from(take_bytes.min(1 << 24)).unwrap_or(0));
    let out = run_streaming(
        ffmpeg,
        &args,
        &StreamLimits::new(timeout),
        cancel,
        &mut |chunk| {
            let chunk_start = seen;
            seen += chunk.len() as u64;
            if seen <= skip_bytes {
                return Ok(Flow::Continue);
            }
            let from = skip_bytes.saturating_sub(chunk_start) as usize;
            let room = (take_bytes - raw.len() as u64) as usize;
            let slice = &chunk[from..];
            let n = slice.len().min(room);
            raw.extend_from_slice(&slice[..n]);
            Ok(if raw.len() as u64 >= take_bytes {
                Flow::Stop
            } else {
                Flow::Continue
            })
        },
    )?;
    if let Some(status) = out.status
        && !status.success()
    {
        return Err(MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            format!("ffmpeg failed to decode the audio ({status})"),
        ));
    }
    let whole = raw.len() - raw.len() % frame_bytes as usize;
    raw.truncate(whole);
    let samples: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok(AudioPcm {
        sample_rate: req.sample_rate,
        channels: req.channels,
        start_sample: start,
        frames: (raw.len() as u64) / frame_bytes,
        samples,
    })
}

/// Decodifica o stream **inteiro** em blocos (para o waveform): `on_block(samples_intercaladas)`.
/// Devolve o total de amostras por canal. Cancelável (mata o ffmpeg).
#[allow(clippy::too_many_arguments)]
pub fn decode_audio_blocks(
    tc: &MediaToolchain,
    path: &Path,
    stream_index: u32,
    sample_rate: u32,
    channels: u32,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
    on_block: &mut dyn FnMut(&[f32]) -> Result<(), MediaError>,
) -> Result<u64, MediaError> {
    let ffmpeg = ffmpeg_of(tc)?;
    check_audio_shape(sample_rate, channels)?;
    let abs = checked_input_path(path)?;
    let args = audio_args(&abs, stream_index, sample_rate, channels);
    let frame_bytes = channels as usize * 4;
    let mut carry: Vec<u8> = Vec::new();
    let mut total: u64 = 0;
    let mut buf: Vec<f32> = Vec::new();
    let out = run_streaming(
        ffmpeg,
        &args,
        &StreamLimits::new(timeout),
        cancel,
        &mut |chunk| {
            carry.extend_from_slice(chunk);
            let usable = carry.len() - carry.len() % frame_bytes;
            if usable == 0 {
                return Ok(Flow::Continue);
            }
            buf.clear();
            buf.extend(
                carry[..usable]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
            carry.drain(..usable);
            total += (usable / frame_bytes) as u64;
            on_block(&buf)?;
            Ok(Flow::Continue)
        },
    )?;
    match out.status {
        Some(s) if s.success() => Ok(total),
        _ => Err(MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            "ffmpeg failed while decoding the audio",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_tick_conversion_is_exact_for_common_rates() {
        for rate in [44_100u32, 48_000, 96_000, 22_050] {
            for n in [0u64, 1, 441, 48_000, 123_456_789] {
                assert_eq!(ticks_to_samples(samples_to_ticks(n, rate), rate), n);
            }
        }
        assert_eq!(samples_to_ticks(48_000, 48_000), Ticks(TICKS_PER_SECOND));
    }

    #[test]
    fn seconds_round_in_the_requested_direction() {
        assert_eq!(seconds_arg(Ticks(1), false), "0.000000");
        assert_eq!(seconds_arg(Ticks(1), true), "0.000001");
        assert_eq!(seconds_arg(Ticks(-9), true), "0.000000");
    }

    #[test]
    fn frame_len_checks_overflow_and_limits() {
        let l = DecodeLimits::default();
        assert_eq!(frame_len(64, 48, &l).ok(), Some(64 * 48 * 4));
        assert!(frame_len(0, 10, &l).is_err());
        assert!(frame_len(u32::MAX, u32::MAX, &l).is_err());
        assert!(frame_len(65_536, 65_536, &l).is_err());
    }
}
