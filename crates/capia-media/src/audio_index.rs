//! Índice de áudio por **quadro decodificado** (ADR-061): `(pts, amostra inicial, nº de amostras)`
//! de cada frame que o decoder entrega, em ordem. Com ele `decode_audio_indexed` salta para perto
//! do trecho pedido (custo ~ independente do início) e ainda assim entrega a amostra **exata**:
//! a posição de amostra vem da soma real de `nb_samples` (inclui priming/skip do decoder), não de
//! `pts × taxa`.
//!
//! Formato binário `CAIX` v1 (little-endian): cabeçalho de 64 B (magic, versão, tamanho do
//! cabeçalho, nº de frames, taxa, canais, *time base*, total de amostras, stream), `n × 24` B de
//! entradas (`pts i64, sample_start u64, nb_samples u32, pad u32`) e rodapé SHA-256.

use crate::decode::AudioPcm;
use crate::error::{MediaError, MediaErrorCode};
use crate::failpoints::fp;
use crate::ffprobe::{checked_input_path, file_url_arg};
use crate::process::{Flow, StreamLimits, run_streaming};
use crate::toolchain::MediaToolchain;
use capia_time::Rational;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

pub const AUDIO_INDEX_MAGIC: [u8; 4] = *b"CAIX";
pub const AUDIO_INDEX_VERSION: u32 = 1;
/// Versão do formato + do algoritmo (entra na `CacheKey`).
pub const AUDIO_INDEX_PRODUCER: &str = "audio-index/1";
pub const MAX_AUDIO_FRAMES: u64 = 20_000_000;
const HEADER_LEN: usize = 64;
const ENTRY_LEN: usize = 24;
const FOOTER_LEN: usize = 32;
/// Frames de pré-roll descartados ao saltar (o decoder precisa aquecer o estado da MDCT).
pub const SEEK_MARGIN_FRAMES: usize = 6;

fn invalid(msg: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorCode::MediaIndexInvalid, msg)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFrameEntry {
    pub pts: i64,
    /// Índice da 1ª amostra (por canal) deste frame na saída decodificada do stream.
    pub sample_start: u64,
    pub nb_samples: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioIndex {
    sample_rate: u32,
    channels: u32,
    time_base: Rational,
    stream_index: u32,
    total_samples: u64,
    /// O container salta com precisão de pacote (mov/mp4/m4a, wav…): só então o decode usa o salto;
    /// nos demais (mkv, ogg, ts…) o seek cai em *cues* esparsos, então decodificamos desde o início.
    fast_seek: bool,
    entries: Vec<AudioFrameEntry>,
}

/// Containers cujo seek cai exatamente no pacote pedido (tabelas de amostras ou PCM cru).
pub fn container_supports_exact_seek(formats: &[String]) -> bool {
    formats.iter().any(|f| {
        matches!(
            f.as_str(),
            "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2" | "wav" | "aiff"
        )
    })
}

/// Para onde saltar e quanto descartar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekPlan {
    pub start_frame: usize,
    /// Amostra inicial (absoluta) do frame onde a decodificação começa.
    pub start_sample: u64,
    /// `pts` do frame de partida (unidades do *time base*).
    pub seek_pts: i64,
}

impl AudioIndex {
    pub fn new(
        sample_rate: u32,
        channels: u32,
        time_base: Rational,
        stream_index: u32,
        fast_seek: bool,
        entries: Vec<AudioFrameEntry>,
    ) -> Result<Self, MediaError> {
        if sample_rate == 0
            || !time_base.is_positive()
            || u32::try_from(time_base.num()).is_err()
            || u32::try_from(time_base.den()).is_err()
        {
            return Err(invalid("invalid sample rate or time base"));
        }
        if entries.is_empty() || entries.len() as u64 > MAX_AUDIO_FRAMES {
            return Err(invalid("implausible audio frame count"));
        }
        let mut next = 0u64;
        for e in &entries {
            if e.sample_start != next || e.nb_samples == 0 {
                return Err(invalid("audio frames are not contiguous"));
            }
            next += u64::from(e.nb_samples);
        }
        if entries.windows(2).any(|w| w[0].pts >= w[1].pts) {
            return Err(invalid(
                "audio frame timestamps are not strictly increasing",
            ));
        }
        Ok(Self {
            sample_rate,
            channels,
            time_base,
            stream_index,
            total_samples: next,
            fast_seek,
            entries,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    pub fn stream_index(&self) -> u32 {
        self.stream_index
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn fast_seek(&self) -> bool {
        self.fast_seek
    }

    pub fn frames(&self) -> &[AudioFrameEntry] {
        &self.entries
    }

    /// Frame que contém a amostra `sample` (`None` se está além do fim).
    pub fn locate(&self, sample: u64) -> Option<usize> {
        if sample >= self.total_samples {
            return None;
        }
        let n = self.entries.partition_point(|e| e.sample_start <= sample);
        n.checked_sub(1)
    }

    /// Plano de salto para `sample`: começa `SEEK_MARGIN_FRAMES` antes do frame que o contém.
    /// Frame do índice cujo `pts` (convertido para 1/sample_rate) é exatamente `pts_rate`.
    fn entry_at_pts_rate(&self, pts_rate: i64) -> Option<AudioFrameEntry> {
        let (n, d) = (
            i128::from(self.time_base.num()) * i128::from(self.sample_rate),
            i128::from(self.time_base.den()),
        );
        let target = i128::from(pts_rate) * d;
        let i = self
            .entries
            .partition_point(|e| i128::from(e.pts) * n < target);
        let e = *self.entries.get(i)?;
        (i128::from(e.pts) * n == target).then_some(e)
    }

    pub fn seek_plan(&self, sample: u64) -> Option<SeekPlan> {
        let f = self.locate(sample)?;
        let start_frame = if self.fast_seek {
            f.saturating_sub(SEEK_MARGIN_FRAMES)
        } else {
            0
        };
        let e = self.entries[start_frame];
        Some(SeekPlan {
            start_frame,
            start_sample: e.sample_start,
            seek_pts: e.pts,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.entries.len() * ENTRY_LEN + FOOTER_LEN);
        out.extend_from_slice(&AUDIO_INDEX_MAGIC);
        out.extend_from_slice(&AUDIO_INDEX_VERSION.to_le_bytes());
        out.extend_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.time_base.num())
                .unwrap_or(1)
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(self.time_base.den())
                .unwrap_or(1)
                .to_le_bytes(),
        );
        out.extend_from_slice(&self.total_samples.to_le_bytes());
        out.extend_from_slice(&self.stream_index.to_le_bytes());
        out.push(u8::from(self.fast_seek));
        out.resize(HEADER_LEN, 0);
        for e in &self.entries {
            out.extend_from_slice(&e.pts.to_le_bytes());
            out.extend_from_slice(&e.sample_start.to_le_bytes());
            out.extend_from_slice(&e.nb_samples.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        let sum = Sha256::digest(&out);
        out.extend_from_slice(&sum);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MediaError> {
        if bytes.len() < HEADER_LEN + FOOTER_LEN {
            return Err(invalid("audio index is truncated"));
        }
        if bytes[..4] != AUDIO_INDEX_MAGIC {
            return Err(invalid("not an audio index (bad magic)"));
        }
        let u32_at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let u64_at = |o: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[o..o + 8]);
            u64::from_le_bytes(b)
        };
        if u32_at(4) != AUDIO_INDEX_VERSION || u32_at(8) as usize != HEADER_LEN {
            return Err(invalid("unsupported audio index version or header"));
        }
        let count = u64_at(12);
        if count == 0 || count > MAX_AUDIO_FRAMES {
            return Err(invalid("implausible audio frame count"));
        }
        let expected = (count as usize)
            .checked_mul(ENTRY_LEN)
            .and_then(|b| b.checked_add(HEADER_LEN + FOOTER_LEN))
            .ok_or_else(|| invalid("size overflow"))?;
        if bytes.len() != expected {
            return Err(invalid("audio index has the wrong size"));
        }
        let body = &bytes[..bytes.len() - FOOTER_LEN];
        if Sha256::digest(body).as_slice() != &bytes[bytes.len() - FOOTER_LEN..] {
            return Err(invalid("audio index checksum mismatch"));
        }
        let tb = Rational::new(i64::from(u32_at(28)), i64::from(u32_at(32)))
            .map_err(|_| invalid("invalid time base"))?;
        let mut entries = Vec::with_capacity(count as usize);
        for k in 0..count as usize {
            let o = HEADER_LEN + k * ENTRY_LEN;
            if u32_at(o + 20) != 0 {
                return Err(invalid("corrupt entry padding"));
            }
            entries.push(AudioFrameEntry {
                pts: i64::from_le_bytes(u64_at(o).to_le_bytes()),
                sample_start: u64_at(o + 8),
                nb_samples: u32_at(o + 16),
            });
        }
        if bytes[48] > 1 {
            return Err(invalid("corrupt flags"));
        }
        let idx = Self::new(
            u32_at(20),
            u32_at(24),
            tb,
            u32_at(44),
            bytes[48] == 1,
            entries,
        )?;
        if idx.total_samples != u64_at(36) {
            return Err(invalid("total samples do not match the entries"));
        }
        Ok(idx)
    }
}

/// Constrói o índice lendo os frames decodificados do ffprobe em streaming
/// (`-show_frames`: `pts,nb_samples`). Cancelável (mata o ffprobe).
#[allow(clippy::too_many_arguments)]
pub fn build_audio_index(
    toolchain: &MediaToolchain,
    path: &Path,
    stream_index: u32,
    sample_rate: u32,
    channels: u32,
    time_base: Rational,
    fast_seek: bool,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
) -> Result<AudioIndex, MediaError> {
    let abs = checked_input_path(path)?;
    let mut args: Vec<OsString> = ["-v", "error", "-select_streams"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push(stream_index.to_string().into());
    for a in [
        "-show_entries",
        "frame=pts,nb_samples",
        "-of",
        "csv=p=0",
        "-protocol_whitelist",
        "file",
        "-i",
    ] {
        args.push(a.into());
    }
    args.push(file_url_arg(&abs));
    let mut carry = String::new();
    let mut entries: Vec<AudioFrameEntry> = Vec::new();
    let mut next_sample = 0u64;
    let mut parse_err: Option<MediaError> = None;
    let out = run_streaming(
        &toolchain.ffprobe,
        &args,
        &StreamLimits::new(timeout),
        cancel,
        &mut |chunk| {
            if entries.is_empty() && carry.is_empty() {
                fp!("audio_index_running");
            }
            carry.push_str(&String::from_utf8_lossy(chunk));
            while let Some(nl) = carry.find('\n') {
                let line: String = carry.drain(..=nl).collect();
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut it = line.split(',');
                let (Some(p), Some(n)) = (it.next(), it.next()) else {
                    return Err(invalid("malformed audio frame line"));
                };
                let (Ok(pts), Ok(ns)) = (p.parse::<i64>(), n.parse::<u32>()) else {
                    // frame sem pts (N/A) ou lixo: o índice não é confiável
                    parse_err = Some(invalid("an audio frame has no usable pts/nb_samples"));
                    return Ok(Flow::Stop);
                };
                if entries.len() as u64 >= MAX_AUDIO_FRAMES {
                    return Err(MediaError::new(
                        MediaErrorCode::MediaLimitExceeded,
                        "too many audio frames",
                    ));
                }
                entries.push(AudioFrameEntry {
                    pts,
                    sample_start: next_sample,
                    nb_samples: ns,
                });
                next_sample += u64::from(ns);
            }
            if carry.len() > 4096 {
                return Err(invalid("audio frame line is too long"));
            }
            progress(next_sample);
            Ok(Flow::Continue)
        },
    )?;
    if let Some(e) = parse_err {
        return Err(e);
    }
    if let Some(status) = out.status
        && !status.success()
    {
        return Err(MediaError::new(
            MediaErrorCode::MediaProbeFailed,
            "ffprobe could not decode the audio frames of this stream",
        ));
    }
    AudioIndex::new(
        sample_rate,
        channels,
        time_base,
        stream_index,
        fast_seek,
        entries,
    )
}

/// Decodifica `frames` amostras a partir de `start_sample` **saltando** para perto (índice de
/// áudio). Taxa = a nativa do stream; `channels` pedido ao ffmpeg. Amostras além do fim não são
/// devolvidas (resultado mais curto). Exato: igual ao decode desde o início (testado).
#[allow(clippy::too_many_arguments)]
pub fn decode_audio_indexed(
    toolchain: &MediaToolchain,
    path: &Path,
    index: &AudioIndex,
    start_sample: u64,
    frames: u64,
    channels: u32,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<AudioPcm, MediaError> {
    let ffmpeg = toolchain.ffmpeg.as_deref().ok_or_else(|| {
        MediaError::new(MediaErrorCode::MediaBackendNotFound, "ffmpeg was not found")
    })?;
    if frames == 0 || channels == 0 || channels > crate::limits::MAX_CHANNELS {
        return Err(MediaError::new(
            MediaErrorCode::MediaMetadataInvalid,
            "invalid audio request",
        ));
    }
    let frame_bytes = u64::from(channels) * 4;
    let max_frames = crate::decode::DEFAULT_MAX_PCM_BYTES / frame_bytes;
    if frames > max_frames {
        return Err(MediaError::new(
            MediaErrorCode::MediaLimitExceeded,
            "the requested PCM exceeds the byte limit",
        ));
    }
    let Some(plan) = index.seek_plan(start_sample) else {
        return Ok(AudioPcm {
            sample_rate: index.sample_rate,
            channels,
            start_sample,
            frames: 0,
            samples: Vec::new(),
        });
    };
    let abs = checked_input_path(path)?;
    let want_end = start_sample.saturating_add(frames);
    let sr = u64::from(index.sample_rate);
    // 1ª tentativa: salto rápido (com folga p/ o pacote de partida cair antes do alvo); se o pouso não
    // for verificável ou a janela não bastar, 2ª tentativa decodificando desde o início (sempre exata)
    if plan.start_frame > 0 {
        let limit = want_end.saturating_sub(plan.start_sample) + sr * 4;
        let first = decode_pcm_from(
            ffmpeg,
            &abs,
            index,
            Some(plan.seek_pts),
            limit * frame_bytes,
            channels,
            timeout,
            cancel,
        )?;
        if let Some(landing) = first.landing.and_then(|l| index.entry_at_pts_rate(l))
            && landing.sample_start <= start_sample
        {
            let have = (first.raw.len() as u64) / frame_bytes;
            let reached_eof = have < limit;
            if landing.sample_start + have >= want_end || reached_eof {
                return Ok(slice_pcm(
                    index,
                    channels,
                    &first.raw,
                    landing.sample_start,
                    start_sample,
                    frames,
                ));
            }
        }
    }
    let all = decode_pcm_from(
        ffmpeg,
        &abs,
        index,
        None,
        want_end.saturating_mul(frame_bytes),
        channels,
        timeout,
        cancel,
    )?;
    Ok(slice_pcm(
        index,
        channels,
        &all.raw,
        0,
        start_sample,
        frames,
    ))
}

fn slice_pcm(
    index: &AudioIndex,
    channels: u32,
    raw: &[u8],
    raw_start_sample: u64,
    start_sample: u64,
    frames: u64,
) -> AudioPcm {
    let fb = u64::from(channels) * 4;
    let have = raw.len() as u64 / fb;
    let skip = start_sample.saturating_sub(raw_start_sample).min(have);
    let take = frames.min(have - skip);
    let bytes = &raw[(skip * fb) as usize..((skip + take) * fb) as usize];
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    AudioPcm {
        sample_rate: index.sample_rate,
        channels,
        start_sample,
        frames: take,
        samples,
    }
}

struct RawDecode {
    raw: Vec<u8>,
    /// `pts` (em 1/sample_rate) do primeiro frame que o ffmpeg entregou após o salto.
    landing: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
fn decode_pcm_from(
    ffmpeg: &Path,
    abs: &Path,
    index: &AudioIndex,
    seek_pts: Option<i64>,
    limit_bytes: u64,
    channels: u32,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<RawDecode, MediaError> {
    let mut args: Vec<OsString> = ["-v", "info", "-nostdin", "-protocol_whitelist", "file"]
        .iter()
        .map(OsString::from)
        .collect();
    if let Some(pts) = seek_pts {
        // µs arredondados PARA CIMA: o salto cai no pacote do frame de partida (nunca no anterior).
        // Em mp4 com vídeo o demuxer pode pousar num keyframe de vídeo mais cedo: o pouso REAL é lido
        // do `ashowinfo` (-copyts mantém o pts absoluto) e verificado contra o índice.
        let tb = index.time_base;
        let num = i128::from(pts) * i128::from(tb.num()) * 1_000_000;
        let den = i128::from(tb.den());
        let us = (num + den - 1).div_euclid(den).max(0);
        for a in ["-copyts", "-seek_timestamp", "1", "-noaccurate_seek", "-ss"] {
            args.push(a.into());
        }
        args.push(format!("{}.{:06}", us / 1_000_000, us % 1_000_000).into());
    }
    args.push("-i".into());
    args.push(file_url_arg(abs));
    for a in [
        "-map".to_owned(),
        format!("0:{}", index.stream_index),
        "-vn".into(),
        "-sn".into(),
        "-af".into(),
        "ashowinfo".into(),
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
    let mut raw: Vec<u8> =
        Vec::with_capacity(usize::try_from(limit_bytes.min(1 << 24)).unwrap_or(0));
    let out = run_streaming(
        ffmpeg,
        &args,
        &StreamLimits::new(timeout),
        cancel,
        &mut |chunk| {
            let room = (limit_bytes - raw.len() as u64) as usize;
            let n = chunk.len().min(room);
            raw.extend_from_slice(&chunk[..n]);
            Ok(if raw.len() as u64 >= limit_bytes {
                Flow::Stop
            } else {
                Flow::Continue
            })
        },
    )?;
    if let Some(status) = out.status
        && !status.success()
        && (raw.len() as u64) < limit_bytes
    {
        return Err(MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            format!("ffmpeg failed to decode the audio ({status})"),
        ));
    }
    let whole = raw.len() - raw.len() % (channels as usize * 4);
    raw.truncate(whole);
    let landing = String::from_utf8_lossy(&out.stderr)
        .lines()
        .find(|l| l.contains("Parsed_ashowinfo") && l.contains(" n:0 "))
        .and_then(|l| l.split(" pts:").nth(1))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|v| v.parse::<i64>().ok());
    Ok(RawDecode { raw, landing })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn index(n: u32) -> AudioIndex {
        let entries: Vec<_> = (0..n)
            .map(|k| AudioFrameEntry {
                pts: i64::from(k) * 1024,
                sample_start: u64::from(k) * 1024,
                nb_samples: 1024,
            })
            .collect();
        AudioIndex::new(
            48_000,
            2,
            Rational::new(1, 48_000).unwrap(),
            1,
            true,
            entries,
        )
        .unwrap()
    }

    #[test]
    fn locate_and_seek_plan_use_real_sample_positions() {
        let ix = index(100);
        assert_eq!(ix.total_samples(), 102_400);
        assert_eq!(ix.locate(0), Some(0));
        assert_eq!(ix.locate(1023), Some(0));
        assert_eq!(ix.locate(1024), Some(1));
        assert_eq!(ix.locate(102_399), Some(99));
        assert_eq!(ix.locate(102_400), None);
        let p = ix.seek_plan(50 * 1024 + 7).unwrap();
        assert_eq!(p.start_frame, 50 - SEEK_MARGIN_FRAMES);
        assert_eq!(p.start_sample, (50 - SEEK_MARGIN_FRAMES as u64) * 1024);
        assert_eq!(ix.seek_plan(10).unwrap().start_frame, 0);
        // container sem seek exato: sempre do início
        let mut slow = ix.clone();
        slow.fast_seek = false;
        assert_eq!(slow.seek_plan(50 * 1024).unwrap().start_frame, 0);
        assert!(ix.seek_plan(999_999).is_none());
    }

    #[test]
    fn binary_round_trip_rejects_every_truncation_and_flip() {
        let ix = index(20);
        let bytes = ix.encode();
        assert_eq!(AudioIndex::decode(&bytes).unwrap(), ix);
        for cut in 0..bytes.len() {
            assert!(AudioIndex::decode(&bytes[..cut]).is_err(), "cut {cut}");
        }
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0x10;
            assert!(AudioIndex::decode(&b).is_err(), "flip {i}");
        }
    }

    #[test]
    fn non_contiguous_or_non_monotonic_frames_are_rejected() {
        let tb = Rational::new(1, 48_000).unwrap();
        let e = |pts, start, n| AudioFrameEntry {
            pts,
            sample_start: start,
            nb_samples: n,
        };
        assert!(
            AudioIndex::new(48_000, 2, tb, 1, true, vec![e(0, 0, 10), e(10, 11, 10)]).is_err(),
            "gap"
        );
        assert!(
            AudioIndex::new(48_000, 2, tb, 1, true, vec![e(10, 0, 10), e(10, 10, 10)]).is_err(),
            "pts"
        );
        assert!(
            AudioIndex::new(48_000, 2, tb, 1, true, vec![e(0, 0, 0)]).is_err(),
            "empty frame"
        );
        assert!(AudioIndex::new(48_000, 2, tb, 1, true, vec![]).is_err());
        assert!(AudioIndex::new(0, 2, tb, 1, true, vec![e(0, 0, 1)]).is_err());
    }
}
