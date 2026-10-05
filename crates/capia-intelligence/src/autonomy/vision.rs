//! Visão do Critic (PHASE5_PLANNER_EDITOR_CRITIC §Critic, closeout ADR-100): amostragem
//! **determinística e limitada** de quadros da timeline (o mesmo `render.frame` do preview),
//! reduzidos e codificados em PNG, para um modelo com `VisionInput`.
//!
//! Regras: nunca vídeo inteiro; no máximo `max_frames` quadros e um teto de bytes; timestamps em
//! `Ticks` inteiros alinhados a quadro; o rótulo de cada quadro só contém ids gerados pelo sistema
//! (nunca nomes de arquivo nem texto do projeto — texto dentro do quadro é dado, não instrução);
//! o digest dos PNGs entra na chave de cache do Critic.

use crate::engine::{Engine, RawFrame};
use crate::error::{IntelError, IntelResult};
use capia_ai::CancelToken;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Ticks por segundo (`docs/TIMELINE_ENGINE.md`).
const TICKS_PER_SECOND: i64 = 705_600_000;
/// Duração de um quadro a 30 fps quando o cabeçalho não diz.
const DEFAULT_FRAME_TICKS: i64 = TICKS_PER_SECOND / 30;
/// Maior lado do quadro enviado ao modelo.
pub const MAX_DIM: u32 = 384;
/// Teto de bytes de PNG por Review (todos os quadros juntos).
pub const MAX_TOTAL_BYTES: usize = 1_500_000;
pub const DEFAULT_MAX_FRAMES: u32 = 6;
pub const HARD_MAX_FRAMES: u32 = 12;

/// Onde e por que amostrar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleSpec {
    pub t_ticks: i64,
    pub reason: &'static str,
    /// Clips visuais/texto visíveis nesse instante (ids do sistema).
    pub clips: Vec<String>,
}

/// Quadro capturado.
#[derive(Clone, Debug)]
pub struct FrameSample {
    pub index: usize,
    pub t_ticks: i64,
    pub reason: &'static str,
    pub clips: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
    pub sha: String,
}

impl FrameSample {
    /// Rótulo (só ids do sistema) que acompanha a imagem no prompt.
    pub fn label(&self) -> String {
        format!(
            "FRAME {} at {} ms (reason: {}; visible clips: [{}])",
            self.index,
            self.t_ticks * 1000 / TICKS_PER_SECOND,
            self.reason,
            self.clips.join(", ")
        )
    }
}

fn frame_ticks(seq: &Value) -> i64 {
    let fr = &seq["header"]["frame_rate"];
    let parse = |s: &str| -> Option<i64> {
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let (n, d): (i64, i64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
        (n > 0 && d > 0).then(|| TICKS_PER_SECOND * d / n)
    };
    fr.as_str()
        .and_then(parse)
        .or_else(|| {
            let n = fr["num"].as_i64()?;
            let d = fr["den"].as_i64().unwrap_or(1);
            (n > 0 && d > 0).then(|| TICKS_PER_SECOND * d / n)
        })
        .unwrap_or(DEFAULT_FRAME_TICKS)
}

fn visual_track_ids(seq: &Value) -> Vec<String> {
    seq["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["kind"] == "visual")
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect()
}

/// Escolhe até `max` instantes: primeiro quadro, ponto médio de cada clip de mídia visual
/// (mudança de plano/B-roll), ponto médio de cada texto (legibilidade/composição) e último quadro.
/// Ordem estável e **sem aleatoriedade**: mesma timeline ⇒ mesmos instantes.
pub fn plan_samples(seq: &Value, max: u32) -> Vec<SampleSpec> {
    let max = max.clamp(1, HARD_MAX_FRAMES) as usize;
    let ft = frame_ticks(seq);
    let tracks = visual_track_ids(seq);
    let mut clips: Vec<(String, i64, i64, bool)> = seq["clips"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, c)| {
            c["track"]
                .as_str()
                .is_some_and(|t| tracks.iter().any(|x| x == t))
        })
        .filter_map(|(id, c)| {
            let start = c["start"].as_i64()?;
            let dur = c["duration"].as_i64()?;
            (dur > 0).then(|| (id.clone(), start, dur, c["content"]["type"] == "media"))
        })
        .collect();
    clips.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    let Some(end) = clips.iter().map(|c| c.1 + c.2).max() else {
        return Vec::new();
    };
    let align = |t: i64| (t.max(0) / ft) * ft;
    let mut points: Vec<(i64, &'static str)> = vec![(align(0), "first frame")];
    for (_, start, dur, media) in &clips {
        let mid = align(start + dur / 2);
        points.push((
            mid,
            if *media {
                "media clip midpoint"
            } else {
                "text midpoint"
            },
        ));
    }
    points.push((align((end - ft).max(0)), "last frame"));
    // dedupe por instante (mantém o primeiro motivo)
    let mut seen = std::collections::BTreeSet::new();
    points.retain(|(t, _)| seen.insert(*t));
    points.sort_by_key(|p| p.0);
    // se passar do teto: mantém primeiro e último e espalha o resto por índice (determinístico)
    if points.len() > max {
        let n = points.len();
        let mut keep = std::collections::BTreeSet::new();
        keep.insert(0);
        keep.insert(n - 1);
        let slots = max.saturating_sub(2);
        for k in 0..slots {
            keep.insert(1 + (k * (n - 2)) / slots.max(1));
        }
        points = keep.into_iter().map(|i| points[i]).collect();
        points.truncate(max);
    }
    points
        .into_iter()
        .map(|(t, reason)| {
            let mut visible: Vec<String> = clips
                .iter()
                .filter(|(_, s, d, _)| *s <= t && t < s + d)
                .map(|c| c.0.clone())
                .collect();
            visible.sort();
            SampleSpec {
                t_ticks: t,
                reason,
                clips: visible,
            }
        })
        .collect()
}

/// Largura/altura reduzidas (maior lado = `MAX_DIM`, pares) a partir do cabeçalho da sequence.
pub fn reduced_size(seq: &Value) -> (u32, u32) {
    let w = seq["header"]["width"].as_u64().unwrap_or(1920).max(2) as f64;
    let h = seq["header"]["height"].as_u64().unwrap_or(1080).max(2) as f64;
    let k = f64::from(MAX_DIM) / w.max(h);
    let even = |v: f64| ((v * k).round() as u32).max(2) & !1;
    (even(w), even(h))
}

/// PNG (RGBA8) de um quadro.
pub fn encode_png(f: &RawFrame) -> IntelResult<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, f.width, f.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc
            .write_header()
            .map_err(|e| IntelError::new("FRAME_ENCODE", e.to_string()))?;
        w.write_image_data(&f.rgba)
            .map_err(|e| IntelError::new("FRAME_ENCODE", e.to_string()))?;
    }
    Ok(out)
}

/// Decodifica um PNG de volta para RGBA8 (usado pelos testes e pelo "modelo de visão" Replay).
pub fn decode_png(bytes: &[u8]) -> IntelResult<RawFrame> {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = dec
        .read_info()
        .map_err(|e| IntelError::new("FRAME_DECODE", e.to_string()))?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| IntelError::new("FRAME_DECODE", e.to_string()))?;
    buf.truncate(info.buffer_size());
    Ok(RawFrame {
        width: info.width,
        height: info.height,
        rgba: buf,
    })
}

/// Captura os quadros. `Ok(vazio)` quando o engine não renderiza. Cancelamento é checado entre
/// quadros; o teto de bytes descarta os últimos quadros (nunca estoura o prompt).
pub fn capture(
    engine: &dyn Engine,
    sequence: &str,
    seq_json: &Value,
    specs: &[SampleSpec],
    cancel: &CancelToken,
) -> IntelResult<Vec<FrameSample>> {
    let (w, h) = reduced_size(seq_json);
    let mut out: Vec<FrameSample> = Vec::new();
    let mut total = 0usize;
    for spec in specs {
        if cancel.is_cancelled() {
            return Err(IntelError::cancelled());
        }
        let Some(raw) = engine.render_frame(sequence, spec.t_ticks, w, h)? else {
            return Ok(Vec::new());
        };
        let png = encode_png(&raw)?;
        if total + png.len() > MAX_TOTAL_BYTES && !out.is_empty() {
            break;
        }
        total += png.len();
        let sha = format!("{:x}", Sha256::digest(&png));
        out.push(FrameSample {
            index: out.len(),
            t_ticks: spec.t_ticks,
            reason: spec.reason,
            clips: spec.clips.clone(),
            width: raw.width,
            height: raw.height,
            png,
            sha,
        });
    }
    Ok(out)
}

/// Digest do conjunto (entra na chave de cache: outro conteúdo visual ⇒ outra análise).
pub fn frames_digest(frames: &[FrameSample]) -> String {
    let mut h = Sha256::new();
    for f in frames {
        h.update(f.t_ticks.to_le_bytes());
        h.update(f.sha.as_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use serde_json::json;

    fn seq(n: usize) -> Value {
        let mut clips = serde_json::Map::new();
        for i in 0..n {
            clips.insert(
                format!("c{i:02}"),
                json!({"track": "V1", "start": i as i64 * TICKS_PER_SECOND, "duration": TICKS_PER_SECOND,
                       "content": {"type": if i % 3 == 2 { "text" } else { "media" }}}),
            );
        }
        clips.insert(
            "audio".into(),
            json!({"track": "A1", "start": 0, "duration": 99 * TICKS_PER_SECOND, "content": {"type": "media"}}),
        );
        json!({"header": {"width": 1080, "height": 1920, "frame_rate": "30"},
               "tracks": [{"id": "V1", "kind": "visual"}, {"id": "A1", "kind": "audio"}], "clips": clips})
    }

    #[test]
    fn sampling_is_deterministic_bounded_frame_aligned_and_ignores_audio_tracks() {
        let s = seq(10);
        let a = plan_samples(&s, 6);
        assert_eq!(a, plan_samples(&s, 6), "same timeline ⇒ same instants");
        assert!(a.len() <= 6 && a.len() >= 2);
        assert!(
            a.windows(2).all(|w| w[0].t_ticks < w[1].t_ticks),
            "ordered, unique"
        );
        assert!(
            a.iter().all(|x| x.t_ticks % DEFAULT_FRAME_TICKS == 0),
            "frame aligned"
        );
        assert_eq!(a[0].reason, "first frame");
        assert_eq!(a.last().unwrap().reason, "last frame");
        assert!(a.iter().all(|x| !x.clips.contains(&"audio".to_owned())));
        // nunca mais que o teto duro
        assert!(plan_samples(&s, 999).len() <= HARD_MAX_FRAMES as usize);
        assert_eq!(
            plan_samples(&json!({"clips": {}, "tracks": []}), 6),
            Vec::new()
        );
    }

    #[test]
    fn short_timelines_sample_each_shot_once() {
        let a = plan_samples(&seq(2), 6);
        let ts: Vec<i64> = a.iter().map(|x| x.t_ticks).collect();
        let mut u = ts.clone();
        u.dedup();
        assert_eq!(ts, u);
        assert!(a.iter().any(|x| x.clips == vec!["c00".to_owned()]));
        assert!(a.iter().any(|x| x.clips == vec!["c01".to_owned()]));
    }

    #[test]
    fn reduced_size_keeps_aspect_and_is_even_and_small() {
        let (w, h) = reduced_size(&seq(1));
        assert_eq!((w, h), (216, 384));
        assert!(w % 2 == 0 && h % 2 == 0);
        let (w, h) = reduced_size(&json!({"header": {"width": 1920, "height": 1080}}));
        assert_eq!(w.max(h), MAX_DIM);
    }

    #[test]
    fn png_round_trips_pixels() {
        let f = RawFrame {
            width: 4,
            height: 2,
            rgba: (0..32u8).collect(),
        };
        let back = decode_png(&encode_png(&f).unwrap()).unwrap();
        assert_eq!((back.width, back.height, back.rgba), (4, 2, f.rgba));
    }

    #[test]
    fn labels_carry_only_system_ids() {
        let f = FrameSample {
            index: 2,
            t_ticks: TICKS_PER_SECOND * 3 / 2,
            reason: "media clip midpoint",
            clips: vec!["c1".into()],
            width: 2,
            height: 2,
            png: vec![],
            sha: String::new(),
        };
        assert_eq!(
            f.label(),
            "FRAME 2 at 1500 ms (reason: media clip midpoint; visible clips: [c1])"
        );
    }
}
