//! Mixer de áudio determinístico (ADR-066). f32; soma com ganho e **hard clip** em [-1, 1] no
//! fim (sem limiter, EQ, compressão ou loudness na Fase 2). NaN/inf viram silêncio.
//!
//! Semântica: amostra de timeline `n` (na taxa de saída) ↔ clip por arredondamento half-up do início
//! do clip em amostras; dentro do clip a posição de fonte é `src_in + local·speed` (ou, com
//! reverso, `src_in + (dur − local)·speed`), interpolada linearmente (`speed = 1` ⇒ cópia exata;
//! sem *pitch-correction*: varispeed). `volume_db` é avaliado por blocos de 256 amostras.

use crate::audio::{AudioBuffer, resample_linear_position};
use crate::error::{RenderError, RenderWarning};
use crate::graph::{GraphClip, GraphClipKind, GraphSequence, MAX_NEST_DEPTH, RenderGraph};
use crate::settings::RenderSettings;
use crate::source::{AudioRequest, MediaSource};
use capia_model::{Animatable, SequenceId, property_spec};
use capia_time::{TICKS_PER_SECOND, Ticks, TimeRange};

const GAIN_BLOCK: u64 = 256;

fn ticks_to_samples_round(t: Ticks, rate: u32) -> i64 {
    let num = i128::from(t.0) * i128::from(rate) * 2 + i128::from(TICKS_PER_SECOND);
    num.div_euclid(2 * i128::from(TICKS_PER_SECOND)) as i64
}

fn gain_at(clip: &GraphClip, content_t: Ticks) -> f64 {
    let Some(spec) = property_spec("volume_db") else {
        return 1.0;
    };
    let db = match clip.clip.properties.get("volume_db") {
        Some(a) => a.eval(content_t, spec),
        None => Animatable::Static(spec.default).eval(content_t, spec),
    };
    10f64.powf(db / 20.0)
}

struct Ctx<'a> {
    graph: &'a RenderGraph,
    settings: &'a RenderSettings,
    source: &'a dyn MediaSource,
    warnings: Vec<RenderWarning>,
}

fn audible_tracks(gs: &GraphSequence) -> impl Iterator<Item = &crate::graph::GraphTrack> {
    let any_solo = gs.tracks.iter().any(|t| t.solo);
    gs.tracks
        .iter()
        .filter(move |t| !t.muted && (!any_solo || t.solo))
}

/// Mix de `frames` amostras de timeline a partir de `start` (índice absoluto na taxa de saída).
fn mix_seq(
    ctx: &mut Ctx<'_>,
    seq: &SequenceId,
    start: i64,
    frames: u64,
    depth: usize,
) -> Result<Vec<f32>, RenderError> {
    if depth > MAX_NEST_DEPTH {
        return Err(RenderError::new(
            "RENDER_NEST_DEPTH",
            "nested sequences are too deep",
        ));
    }
    let rate = ctx.settings.audio_sample_rate;
    let ch = ctx.settings.audio_channels as usize;
    let gs = ctx.graph.sequence(seq)?;
    let mut out = vec![0.0f32; frames as usize * ch];
    let end = start + frames as i64;
    for track in audible_tracks(gs) {
        // tracks visuais com mídia que tem áudio também contribuem (vídeo com áudio embutido)
        for gc in &track.clips {
            if !gc.clip.enabled {
                continue;
            }
            let c_start = ticks_to_samples_round(gc.clip.start, rate);
            let c_end = ticks_to_samples_round(gc.clip.end(), rate);
            let (s0, s1) = (c_start.max(start), c_end.min(end));
            if s0 >= s1 {
                continue;
            }
            let speed_num = gc.clip.speed.num();
            let speed_den = gc.clip.speed.den();
            let seg_frames = (s1 - s0) as u64;
            let src_in = ticks_to_samples_round(gc.clip.source_in, rate);
            let dur_s = c_end - c_start;
            let local0 = s0 - c_start;
            // posição de fonte (amostras de fonte/filho) do primeiro quadro do segmento
            let (base, reversed) = if gc.clip.reversed {
                let at_end = src_in + (dur_s * speed_num) / speed_den;
                (at_end - (local0 * speed_num) / speed_den, true)
            } else {
                (src_in + (local0 * speed_num) / speed_den, false)
            };
            // faixa de fonte necessária (com margem de 2 amostras para a interpolação)
            let span = ((seg_frames as i64) * speed_num + speed_den - 1) / speed_den + 2;
            let (lo, n_src) = if reversed {
                (base - span, span + 2)
            } else {
                (base, span + 2)
            };
            let lo_clamped = lo.max(0);
            let skipped = lo_clamped - lo;
            let n_clamped = (n_src - skipped).max(0) as u64;
            let block: Vec<f32> = match &gc.kind {
                GraphClipKind::Media {
                    asset,
                    has_audio: true,
                    ..
                } => {
                    if n_clamped == 0 {
                        Vec::new()
                    } else {
                        let req = AudioRequest {
                            start_sample: lo_clamped as u64,
                            frames: n_clamped,
                            sample_rate: rate,
                            channels: ctx.settings.audio_channels,
                        };
                        match ctx.source.audio(asset, req) {
                            Ok(b) => b.samples,
                            Err(e) => {
                                if ctx.settings.strict_sources {
                                    return Err(RenderError::new(
                                        "RENDER_SOURCE_FAILED",
                                        format!("clip {} (asset {asset}): {e}", gc.clip.id),
                                    ));
                                }
                                ctx.warnings.push(RenderWarning::new(
                                    "SOURCE_UNAVAILABLE",
                                    format!("clip {} (asset {asset}): {e}", gc.clip.id),
                                ));
                                continue;
                            }
                        }
                    }
                }
                GraphClipKind::Nested { sequence } => {
                    if n_clamped == 0 {
                        Vec::new()
                    } else {
                        mix_seq(ctx, sequence, lo_clamped, n_clamped, depth + 1)?
                    }
                }
                _ => continue,
            };
            let seg = resample_linear_position(
                &block,
                ctx.settings.audio_channels,
                lo_clamped as u64,
                base,
                speed_num,
                speed_den,
                reversed,
                seg_frames,
            );
            // ganho por bloco + soma
            let off = (s0 - start) as usize;
            let mut done = 0u64;
            while done < seg_frames {
                let n = GAIN_BLOCK.min(seg_frames - done);
                let t_local = Ticks(
                    gc.clip.start.0
                        + (((local0 + done as i64) as i128 * i128::from(TICKS_PER_SECOND))
                            / i128::from(rate)) as i64,
                );
                let content_t = gc.clip.content_time(t_local).unwrap_or(gc.clip.source_in);
                let g = gain_at(gc, content_t);
                for f in 0..n as usize {
                    let fi = done as usize + f;
                    for c in 0..ch {
                        let v = f64::from(seg[fi * ch + c]) * g;
                        out[(off + fi) * ch + c] += v as f32;
                    }
                }
                done += n;
            }
        }
    }
    Ok(out)
}

/// Mix de áudio do `range` (tempo da sequence) na taxa/canais de `settings`.
pub fn mix_audio_range(
    graph: &RenderGraph,
    seq: &SequenceId,
    range: TimeRange,
    settings: &RenderSettings,
    source: &dyn MediaSource,
) -> Result<(AudioBuffer, Vec<RenderWarning>), RenderError> {
    settings.validate()?;
    let end = range
        .end()
        .ok_or_else(|| RenderError::new("RENDER_TIME_OVERFLOW", "range end overflows"))?;
    if range.start.0 < 0 || end < range.start {
        return Err(RenderError::new(
            "RENDER_RANGE_INVALID",
            "range must satisfy 0 ≤ start ≤ end",
        ));
    }
    let rate = settings.audio_sample_rate;
    let s0 = ticks_to_samples_round(range.start, rate);
    let s1 = ticks_to_samples_round(end, rate);
    let frames = (s1 - s0).max(0) as u64;
    let bytes = frames
        .checked_mul(u64::from(settings.audio_channels))
        .and_then(|n| n.checked_mul(4))
        .filter(|b| *b <= (2u64 << 30));
    if bytes.is_none() {
        return Err(RenderError::new(
            "RENDER_AUDIO_TOO_LARGE",
            "audio range is too large",
        ));
    }
    let mut ctx = Ctx {
        graph,
        settings,
        source,
        warnings: Vec::new(),
    };
    let mut samples = mix_seq(&mut ctx, seq, s0, frames, 0)?;
    for v in &mut samples {
        *v = if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
    ctx.warnings.sort();
    ctx.warnings.dedup();
    Ok((
        AudioBuffer {
            sample_rate: rate,
            channels: settings.audio_channels,
            samples,
        },
        ctx.warnings,
    ))
}
