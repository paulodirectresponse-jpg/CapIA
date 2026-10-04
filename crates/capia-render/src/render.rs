//! Render de quadros: plano de layers → composição CPU (ADR-064/065). O **mesmo** caminho serve
//! preview e export; só a `MediaSource` e a resolução mudam, nunca o tempo ou a ordem.

use crate::error::{RenderError, RenderWarning};
use crate::graph::{LayerKind, LayerPlan, MAX_NEST_DEPTH, RenderGraph};
use crate::image::{Image, blit_layer, parse_color};
use crate::settings::RenderSettings;
use crate::source::MediaSource;
use capia_model::SequenceId;
use capia_time::{FrameRate, Ticks, TimeRange};

#[derive(Debug)]
pub struct RenderedFrame {
    pub image: Image,
    /// Ordenados e únicos.
    pub warnings: Vec<RenderWarning>,
}

/// Instante do quadro `n` na cadência `rate` (inteiro exato: `n × duração do quadro`).
pub fn frame_time(rate: FrameRate, n: i64) -> Result<Ticks, RenderError> {
    rate.frames_to_ticks(n)
        .map_err(|e| RenderError::new("RENDER_TIME_OVERFLOW", e.to_string()))
}

fn source_err(
    settings: &RenderSettings,
    warnings: &mut Vec<RenderWarning>,
    what: &str,
    e: impl core::fmt::Display,
) -> Result<(), RenderError> {
    if settings.strict_sources {
        Err(RenderError::new(
            "RENDER_SOURCE_FAILED",
            format!("{what}: {e}"),
        ))
    } else {
        warnings.push(RenderWarning::new(
            "SOURCE_UNAVAILABLE",
            format!("{what}: {e}"),
        ));
        Ok(())
    }
}

fn draw_layers(
    canvas: &mut Image,
    layers: &[LayerPlan],
    settings: &RenderSettings,
    source: &dyn MediaSource,
    warnings: &mut Vec<RenderWarning>,
    depth: usize,
) -> Result<(), RenderError> {
    if depth > MAX_NEST_DEPTH {
        return Err(RenderError::new(
            "RENDER_NEST_DEPTH",
            "nested sequences are too deep",
        ));
    }
    for layer in layers {
        let tr = layer.transform;
        let mut rotation = tr.rotation;
        if rotation % 90.0 != 0.0 {
            warnings.push(RenderWarning::new(
                "ROTATION_NOT_MULTIPLE_OF_90",
                format!(
                    "clip {} rotation {} is not rendered (Phase 2 renders multiples of 90°)",
                    layer.clip, tr.rotation
                ),
            ));
            rotation = 0.0;
        }
        let draw = |canvas: &mut Image, img: &Image| -> Result<(), RenderError> {
            blit_layer(
                canvas, img, tr.opacity, tr.scale, tr.pos_x, tr.pos_y, rotation,
            )
            .map(|_| ())
        };
        match &layer.kind {
            LayerKind::Media { asset, source_t } => match source.video_frame(asset, *source_t) {
                Ok(Some(img)) => draw(canvas, &img)?,
                Ok(None) => {}
                Err(e) => source_err(
                    settings,
                    warnings,
                    &format!("clip {} (asset {asset})", layer.clip),
                    e,
                )?,
            },
            LayerKind::Image { asset } => match source.still_image(asset) {
                Ok(img) => draw(canvas, &img)?,
                Err(e) => source_err(
                    settings,
                    warnings,
                    &format!("clip {} (asset {asset})", layer.clip),
                    e,
                )?,
            },
            LayerKind::Solid { color } => {
                let rgba = match parse_color(color) {
                    Some(c) => c,
                    None => {
                        warnings.push(RenderWarning::new(
                            "SOLID_COLOR_INVALID",
                            format!("clip {} has an unparseable color `{color}`", layer.clip),
                        ));
                        [0, 0, 0, 0]
                    }
                };
                let img = Image::filled(canvas.width, canvas.height, rgba)?;
                draw(canvas, &img)?;
            }
            LayerKind::Nested { layers: inner, .. } => {
                let mut child = Image::transparent(canvas.width, canvas.height)?;
                draw_layers(&mut child, inner, settings, source, warnings, depth + 1)?;
                draw(canvas, &child)?;
            }
            LayerKind::Unsupported { what } => {
                warnings.push(RenderWarning::new(
                    "TEXT_NOT_RENDERED",
                    format!("clip {} ({what}) is not rendered in Phase 2", layer.clip),
                ));
            }
        }
    }
    Ok(())
}

/// Renderiza o quadro de `seq` no instante `t` (tempo da sequence) sobre preto opaco.
pub fn render_frame(
    graph: &RenderGraph,
    seq: &SequenceId,
    t: Ticks,
    settings: &RenderSettings,
    source: &dyn MediaSource,
) -> Result<RenderedFrame, RenderError> {
    settings.validate()?;
    let plan = graph.plan_video(seq, t)?;
    let mut canvas = Image::filled(settings.width, settings.height, [0, 0, 0, 255])?;
    let mut warnings = Vec::new();
    draw_layers(&mut canvas, &plan, settings, source, &mut warnings, 0)?;
    warnings.sort();
    warnings.dedup();
    Ok(RenderedFrame {
        image: canvas,
        warnings,
    })
}

/// Renderiza os quadros `n·duração ∈ [range.start, range.end)` na cadência de `settings` (ou da
/// sequence). `on_frame(índice_global, t, imagem)` devolve `false` para parar. Devolve quantos
/// quadros entregou e os avisos acumulados.
pub fn render_video_range(
    graph: &RenderGraph,
    seq: &SequenceId,
    range: TimeRange,
    settings: &RenderSettings,
    source: &dyn MediaSource,
    on_frame: &mut dyn FnMut(i64, Ticks, Image) -> bool,
) -> Result<(u64, Vec<RenderWarning>), RenderError> {
    settings.validate()?;
    let gs = graph.sequence(seq)?;
    let rate = settings.frame_rate.unwrap_or(gs.frame_rate);
    let end = range
        .end()
        .ok_or_else(|| RenderError::new("RENDER_TIME_OVERFLOW", "range end overflows"))?;
    let fd = rate.frame_duration();
    if fd.0 <= 0 || range.start.0 < 0 {
        return Err(RenderError::new(
            "RENDER_RANGE_INVALID",
            "range must start at t ≥ 0",
        ));
    }
    // primeiro quadro com t ≥ início (teto)
    let first = (range.start.0 + fd.0 - 1) / fd.0;
    let mut warnings = Vec::new();
    let mut count = 0u64;
    let mut n = first;
    loop {
        let t = frame_time(rate, n)?;
        if t >= end {
            break;
        }
        let f = render_frame(graph, seq, t, settings, source)?;
        warnings.extend(f.warnings);
        count += 1;
        if !on_frame(n, t, f.image) {
            break;
        }
        n += 1;
    }
    warnings.sort();
    warnings.dedup();
    Ok((count, warnings))
}

/// SHA-256 (hex) das dimensões e dos bytes do quadro — a "digital" usada pelos goldens e pela
/// paridade preview × export.
pub fn frame_digest(img: &Image) -> String {
    let mut h = Sha256::new();
    h.update(&img.width.to_le_bytes());
    h.update(&img.height.to_le_bytes());
    h.update(&img.data);
    h.finish_hex()
}

// SHA-256 próprio (FIPS 180-4), sem dependências — mesmo critério do `capia-commands`.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

struct Sha256 {
    state: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }

    fn block(state: &mut [u32; 8], b: &[u8]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = *state;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            state[i] = state[i].wrapping_add(v[i]);
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if !self.buf.is_empty() {
            let need = 64 - self.buf.len();
            let take = need.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 {
                let b = core::mem::take(&mut self.buf);
                Self::block(&mut self.state, &b);
            }
        }
        while data.len() >= 64 {
            Self::block(&mut self.state, &data[..64]);
            data = &data[64..];
        }
        self.buf.extend_from_slice(data);
    }

    fn finish_hex(mut self) -> String {
        let bits = self.len * 8;
        let mut pad = vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bits.to_be_bytes());
        let len_before = self.len;
        self.update(&pad);
        self.len = len_before;
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        let hex = |data: &[u8]| {
            let mut h = Sha256::new();
            h.update(data);
            h.finish_hex()
        };
        assert_eq!(
            hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 1.000 bytes em pedaços irregulares == de uma vez
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut h = Sha256::new();
        for c in data.chunks(37) {
            h.update(c);
        }
        assert_eq!(h.finish_hex(), hex(&data));
    }

    #[test]
    fn digests_depend_on_size_and_content() {
        let a = Image::filled(2, 2, [1, 2, 3, 255]).unwrap();
        let b = Image::filled(4, 1, [1, 2, 3, 255]).unwrap();
        assert_ne!(
            frame_digest(&a),
            frame_digest(&b),
            "same bytes, different shape"
        );
        assert_eq!(frame_digest(&a), frame_digest(&a.clone()));
    }

    #[test]
    fn frame_times_are_exact_integers() {
        let r = FrameRate::from_fraction(30_000, 1001).unwrap();
        assert_eq!(frame_time(r, 0).unwrap(), Ticks(0));
        assert_eq!(
            frame_time(r, 30_000).unwrap().0,
            30_000 * r.frame_duration().0
        );
    }
}
