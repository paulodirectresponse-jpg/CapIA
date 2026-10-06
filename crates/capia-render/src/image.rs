//! Imagem RGBA8 (alpha reto) e o compositor CPU de referência (ADR-064/065).
//!
//! **Convenções (normativas):**
//! * Coordenadas de saída: origem no canto superior esquerdo, x → direita, y → baixo; o pixel
//!   `(x, y)` cobre `[x, x+1) × [y, y+1)` e seu centro é `(x+0.5, y+0.5)`.
//! * Posicionamento de um layer: a fonte é **ajustada** (*fit/contain*) à saída preservando a
//!   proporção e centralizada; `scale` multiplica esse ajuste; `position_x/y` deslocam o centro em
//!   **pixels de saída** (x → direita, y → baixo); `rotation` é em graus no sentido horário, em torno
//!   do centro — só múltiplos exatos de 90° são renderizados (os demais geram aviso).
//! * Amostragem: bilinear em ponto fixo (pesos de 8 bits), com interpolação em alpha
//!   **pré-multiplicado** (sem franjas escuras). O pixel de saída é coberto se o **seu centro** cai
//!   dentro do retângulo da fonte (borda dura, sem halo); taps além da borda repetem a borda. Com
//!   escala 1 e deslocamento inteiro vira cópia exata.
//! * `opacity ∈ [0, 1]` é quantizada para `round(opacity·255)` e multiplica o alpha do layer.
//! * Mistura `source-over` com alpha reto, em inteiros:
//!   `a_o = a_s + a_d·(255−a_s)/255`; `c_o = (c_s·a_s·255 + c_d·a_d·(255−a_s)) / (a_o·255)`,
//!   cada divisão arredondada half-up.
//! * Cor/faixa: a imagem já chega em RGBA8 full-range (a conversão YUV→RGB é da camada de decode).

use crate::error::RenderError;

pub const MAX_DIMENSION: u32 = 16_384;
/// Abaixo disto (pixels do retângulo do layer) o laço fica numa thread só (custo de spawn).
const PAR_MIN_PIXELS: usize = 250_000;
const MAX_RENDER_THREADS: usize = 8;
const MAX_BYTES: u64 = 1 << 30;

/// RGBA8, alpha reto, linhas contíguas (`stride = width × 4`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

fn checked_len(width: u32, height: u32) -> Result<usize, RenderError> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|p| p.checked_mul(4))
        .filter(|b| *b > 0 && *b <= MAX_BYTES && width <= MAX_DIMENSION && height <= MAX_DIMENSION)
        .ok_or_else(|| {
            RenderError::new(
                "RENDER_IMAGE_TOO_LARGE",
                format!("{width}x{height} exceeds the image limits"),
            )
        })?;
    usize::try_from(bytes)
        .map_err(|_| RenderError::new("RENDER_IMAGE_TOO_LARGE", "image too large"))
}

impl Image {
    /// Preenche com `rgba`.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self, RenderError> {
        let len = checked_len(width, height)?;
        let mut data = Vec::with_capacity(len);
        for _ in 0..(len / 4) {
            data.extend_from_slice(&rgba);
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn transparent(width: u32, height: u32) -> Result<Self, RenderError> {
        let len = checked_len(width, height)?;
        Ok(Self {
            width,
            height,
            data: vec![0; len],
        })
    }

    /// Valida o tamanho do buffer.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self, RenderError> {
        if checked_len(width, height)? != data.len() {
            return Err(RenderError::new(
                "RENDER_IMAGE_INVALID",
                "buffer length does not match width × height × 4",
            ));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    #[cfg(test)]
    fn put(&mut self, x: u32, y: u32, p: [u8; 4]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i..i + 4].copy_from_slice(&p);
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }
}

/// `round(a / b)` para inteiros não negativos.
#[inline]
fn div_round(a: u64, b: u64) -> u64 {
    (2 * a + b) / (2 * b)
}

/// `source-over` com alpha reto (ver a convenção no topo do módulo).
#[inline]
pub fn blend_over(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let (a_s, a_d) = (u64::from(src[3]), u64::from(dst[3]));
    if a_s == 0 {
        return dst;
    }
    if a_s == 255 {
        return src;
    }
    let a_o = a_s + div_round(a_d * (255 - a_s), 255);
    if a_o == 0 {
        return [0, 0, 0, 0];
    }
    let den = a_o * 255;
    let mut out = [0u8; 4];
    for c in 0..3 {
        let num = u64::from(src[c]) * a_s * 255 + u64::from(dst[c]) * a_d * (255 - a_s);
        out[c] = div_round(num, den).min(255) as u8;
    }
    out[3] = a_o.min(255) as u8;
    out
}

/// Cor `#RGB`, `#RGBA`, `#RRGGBB` ou `#RRGGBBAA` (hex); outra coisa ⇒ `None`.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nib = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = h.as_bytes();
    match b.len() {
        3 | 4 => {
            let mut o = [0, 0, 0, 255];
            for (i, c) in b.iter().enumerate() {
                let d = nib(*c)?;
                o[i] = d * 17;
            }
            Some(o)
        }
        6 | 8 => {
            let mut o = [0, 0, 0, 255];
            for i in 0..b.len() / 2 {
                o[i] = nib(b[2 * i])? * 16 + nib(b[2 * i + 1])?;
            }
            Some(o)
        }
        _ => None,
    }
}

/// Posiciona `src` sobre `dst` (modelo no topo do módulo). `rotation_deg` precisa ser múltiplo de
/// 90 (o chamador avisa e ignora os demais). Devolve `Ok(false)` se nada foi desenhado.
pub fn blit_layer(
    dst: &mut Image,
    src: &Image,
    opacity: f64,
    scale: f64,
    pos_x: f64,
    pos_y: f64,
    rotation_deg: f64,
) -> Result<bool, RenderError> {
    blit_layer_threads(dst, src, opacity, scale, pos_x, pos_y, rotation_deg, None)
}

/// `blit_layer` com número de threads explícito (`None` = automático). O resultado não depende dele.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blit_layer_threads(
    dst: &mut Image,
    src: &Image,
    opacity: f64,
    scale: f64,
    pos_x: f64,
    pos_y: f64,
    rotation_deg: f64,
    threads: Option<usize>,
) -> Result<bool, RenderError> {
    if !(opacity.is_finite() && scale.is_finite() && pos_x.is_finite() && pos_y.is_finite())
        || !rotation_deg.is_finite()
    {
        return Err(RenderError::new(
            "RENDER_TRANSFORM_INVALID",
            "non-finite transform",
        ));
    }
    let op = (opacity.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u64;
    if op == 0 || scale <= 0.0 {
        return Ok(false);
    }
    let quarter = ((rotation_deg / 90.0).round() as i64).rem_euclid(4);
    let (w, h) = (f64::from(dst.width), f64::from(dst.height));
    let (sw, sh) = (f64::from(src.width), f64::from(src.height));
    // dimensões do layer depois de girar (90/270 trocam largura e altura)
    let (lw, lh) = if quarter % 2 == 0 { (sw, sh) } else { (sh, sw) };
    let fit = (w / lw).min(h / lh);
    let s = fit * scale;
    // retângulo de destino (aproximado por excesso de 1 px) para limitar o laço
    let half_w = lw * s / 2.0;
    let half_h = lh * s / 2.0;
    let cx = w / 2.0 + pos_x;
    let cy = h / 2.0 + pos_y;
    let x0 = ((cx - half_w).floor() as i64 - 1).clamp(0, i64::from(dst.width));
    let x1 = ((cx + half_w).ceil() as i64 + 1).clamp(0, i64::from(dst.width));
    let y0 = ((cy - half_h).floor() as i64 - 1).clamp(0, i64::from(dst.height));
    let y1 = ((cy + half_h).ceil() as i64 + 1).clamp(0, i64::from(dst.height));
    if x0 >= x1 || y0 >= y1 {
        return Ok(false);
    }
    // caminho rápido: escala 1, sem rotação e deslocamento inteiro ⇒ a amostragem bilinear é uma
    // cópia exata (pesos 0/256), então pulamos a matemática de ponto flutuante por pixel
    let ox = cx - sw / 2.0;
    let oy = cy - sh / 2.0;
    let (y0u, y1u) = (y0 as usize, y1 as usize);
    let (x0u, x1u) = (x0 as usize, x1 as usize);
    if quarter == 0 && s == 1.0 && ox.fract() == 0.0 && oy.fract() == 0.0 {
        let (ox, oy) = (ox as i64, oy as i64);
        return Ok(par_rows(dst, y0u, y1u, threads, |y, row| {
            let sy = y as i64 - oy;
            if sy < 0 || sy >= i64::from(src.height) {
                return false;
            }
            let src_row =
                &src.data[sy as usize * src.width as usize * 4..][..src.width as usize * 4];
            let mut drew = false;
            for x in x0u..x1u {
                let sx = x as i64 - ox;
                if sx < 0 || sx >= i64::from(src.width) {
                    continue;
                }
                let sp = &src_row[sx as usize * 4..sx as usize * 4 + 4];
                if sp[3] == 0 {
                    continue;
                }
                let a = ((u64::from(sp[3]) * op + 127) / 255) as u8;
                if a == 0 {
                    continue;
                }
                let d = &mut row[x * 4..x * 4 + 4];
                let out = blend_over([d[0], d[1], d[2], d[3]], [sp[0], sp[1], sp[2], a]);
                d.copy_from_slice(&out);
                drew = true;
            }
            drew
        }));
    }
    let inv = 1.0 / s;
    Ok(par_rows(dst, y0u, y1u, threads, |y, row| {
        let mut drew = false;
        for x in x0u..x1u {
            // centro do pixel de saída, relativo ao centro do layer, no espaço do layer girado
            let px = (x as f64 + 0.5 - cx) * inv;
            let py = (y as f64 + 0.5 - cy) * inv;
            // inverso da rotação horária: (u, v) → coordenadas relativas à fonte
            let (u, v) = match quarter {
                0 => (px, py),
                1 => (py, -px),
                2 => (-px, -py),
                _ => (-py, px),
            };
            // cobertura: o pixel de saída entra se o SEU CENTRO cai dentro do retângulo da fonte
            // (borda dura, sem halo); a cor vem da bilinear com os taps limitados à borda
            let (cu, cv) = (u + sw / 2.0, v + sh / 2.0);
            if !(cu >= 0.0 && cu < sw && cv >= 0.0 && cv < sh) {
                continue;
            }
            let p = sample_bilinear(src, cu - 0.5, cv - 0.5);
            if p[3] == 0 {
                continue;
            }
            let a = ((u64::from(p[3]) * op + 127) / 255) as u8;
            if a == 0 {
                continue;
            }
            let d = &mut row[x * 4..x * 4 + 4];
            let out = blend_over([d[0], d[1], d[2], d[3]], [p[0], p[1], p[2], a]);
            d.copy_from_slice(&out);
            drew = true;
        }
        drew
    }))
}

/// Executa `f(y, linha)` para cada linha `y ∈ [y0, y1)` de `dst`, em faixas contíguas por várias
/// threads quando a área é grande. Cada pixel de saída depende só da fonte e do próprio destino,
/// então o resultado é **idêntico** ao serial (bit a bit) qualquer que seja o número de threads.
/// Devolve se alguma linha desenhou algo.
fn par_rows<F>(dst: &mut Image, y0: usize, y1: usize, threads: Option<usize>, f: F) -> bool
where
    F: Fn(usize, &mut [u8]) -> bool + Sync,
{
    let stride = dst.width as usize * 4;
    let rows = y1.saturating_sub(y0);
    if rows == 0 || stride == 0 {
        return false;
    }
    let area = rows * dst.width as usize;
    let threads = threads
        .unwrap_or_else(|| {
            if area >= PAR_MIN_PIXELS {
                // `CAPIA_RENDER_THREADS=1` força o caminho serial (diagnóstico/benchmark)
                std::env::var("CAPIA_RENDER_THREADS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or_else(|| {
                        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
                    })
                    .clamp(1, MAX_RENDER_THREADS)
            } else {
                1
            }
        })
        .clamp(1, rows);
    let region = &mut dst.data[y0 * stride..y1 * stride];
    if threads <= 1 {
        let mut drew = false;
        for (k, row) in region.chunks_mut(stride).enumerate() {
            drew |= f(y0 + k, row);
        }
        return drew;
    }
    let per = rows.div_ceil(threads);
    let drew = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|sc| {
        for (b, band) in region.chunks_mut(per * stride).enumerate() {
            let (f, drew) = (&f, &drew);
            sc.spawn(move || {
                let mut any = false;
                for (k, row) in band.chunks_mut(stride).enumerate() {
                    any |= f(y0 + b * per + k, row);
                }
                if any {
                    drew.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
    });
    drew.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bilinear em ponto fixo com alpha pré-multiplicado; fora da fonte = transparente.
fn sample_bilinear(src: &Image, sx: f64, sy: f64) -> [u8; 4] {
    let fx = (sx * 256.0).floor() as i64;
    let fy = (sy * 256.0).floor() as i64;
    let (ix, iy) = (fx >> 8, fy >> 8);
    let (wx, wy) = ((fx & 255) as u64, (fy & 255) as u64);
    let weights = [
        (256 - wx) * (256 - wy),
        wx * (256 - wy),
        (256 - wx) * wy,
        wx * wy,
    ];
    let taps = [(ix, iy), (ix + 1, iy), (ix, iy + 1), (ix + 1, iy + 1)];
    let (mut a_sum, mut c_sum) = (0u64, [0u64; 3]);
    let mut any = false;
    for (k, (tx, ty)) in taps.iter().enumerate() {
        if weights[k] == 0 {
            continue;
        }
        // taps fora da imagem repetem a borda (a cobertura já foi decidida pelo centro do pixel)
        let cx = (*tx).clamp(0, i64::from(src.width) - 1) as u32;
        let cy = (*ty).clamp(0, i64::from(src.height) - 1) as u32;
        let p = src.pixel(cx, cy);
        let wa = weights[k] * u64::from(p[3]);
        a_sum += wa;
        for c in 0..3 {
            c_sum[c] += wa * u64::from(p[c]);
        }
        any = true;
    }
    if !any || a_sum == 0 {
        return [0, 0, 0, 0];
    }
    let a = div_round(a_sum, 65_536).min(255);
    let mut out = [0u8; 4];
    for c in 0..3 {
        out[c] = div_round(c_sum[c], a_sum).min(255) as u8;
    }
    out[3] = a as u8;
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn blend_identities_and_known_values() {
        let d = [10, 20, 30, 255];
        assert_eq!(blend_over(d, [1, 2, 3, 0]), d);
        assert_eq!(blend_over(d, [200, 100, 50, 255]), [200, 100, 50, 255]);
        // 50 % de branco sobre preto opaco ⇒ cinza 128 (half-up de 127,5)
        assert_eq!(
            blend_over([0, 0, 0, 255], [255, 255, 255, 128]),
            [128, 128, 128, 255]
        );
        // sobre transparente: mantém a cor do layer e o alpha dele
        assert_eq!(blend_over([0, 0, 0, 0], [9, 8, 7, 100]), [9, 8, 7, 100]);
        // não é comutativa: a ordem (quem está em cima) importa
        let a = blend_over([255, 0, 0, 255], [0, 0, 255, 128]);
        let b = blend_over([0, 0, 255, 255], [255, 0, 0, 128]);
        assert_ne!(a, b);
    }

    #[test]
    fn colors_parse_in_all_forms() {
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#f008"), Some([255, 0, 0, 136]));
        assert_eq!(parse_color("#102030"), Some([16, 32, 48, 255]));
        assert_eq!(parse_color("#10203040"), Some([16, 32, 48, 64]));
        for bad in ["", "red", "#12", "#gggggg", "#1234567"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn threaded_blit_is_bit_identical_to_serial() {
        // 700×500 com transparência variada: escala ≠ 1, rotação de 90° e o caminho de cópia
        let mut data = Vec::new();
        for y in 0..500u32 {
            for x in 0..700u32 {
                data.extend_from_slice(&[
                    (x % 251) as u8,
                    (y % 241) as u8,
                    ((x + y) % 239) as u8,
                    if (x / 7 + y / 5) % 9 == 0 {
                        0
                    } else {
                        40 + ((x * y) % 215) as u8
                    },
                ]);
            }
        }
        let src = Image::from_rgba(700, 500, data).unwrap();
        for (op, scale, px, py, rot) in [
            (1.0, 1.0, 0.0, 0.0, 0.0),
            (0.6, 0.73, 31.5, -12.25, 0.0),
            (0.9, 1.4, -20.0, 8.0, 90.0),
            (1.0, 0.5, 5.0, 5.0, 270.0),
        ] {
            let mut base = Image::filled(900, 700, [10, 20, 30, 255]).unwrap();
            let mut a = base.clone();
            let mut b = base.clone();
            let da = blit_layer_threads(&mut a, &src, op, scale, px, py, rot, Some(1)).unwrap();
            let db = blit_layer_threads(&mut b, &src, op, scale, px, py, rot, Some(5)).unwrap();
            assert_eq!(da, db);
            assert_eq!(a, b, "op={op} scale={scale} rot={rot}");
            let _ = blit_layer(&mut base, &src, op, scale, px, py, rot).unwrap();
            assert_eq!(a, base);
        }
    }

    #[test]
    fn identity_blit_is_an_exact_copy() {
        let src = Image::from_rgba(2, 2, (0..16).map(|i| (i * 13) as u8 | 1).collect()).unwrap();
        let mut dst = Image::transparent(2, 2).unwrap();
        // alpha opaco para a cópia ser exata
        let mut s = src.clone();
        for p in s.data.chunks_exact_mut(4) {
            p[3] = 255;
        }
        assert!(blit_layer(&mut dst, &s, 1.0, 1.0, 0.0, 0.0, 0.0).unwrap());
        assert_eq!(dst, s);
    }

    #[test]
    fn fit_centers_and_letterboxes() {
        // fonte 2×1 em saída 4×4: fit = 2 ⇒ 4×2 no centro (linhas 1 e 2)
        let src = Image::filled(2, 1, [255, 0, 0, 255]).unwrap();
        let mut dst = Image::filled(4, 4, [0, 0, 0, 255]).unwrap();
        blit_layer(&mut dst, &src, 1.0, 1.0, 0.0, 0.0, 0.0).unwrap();
        for y in 0..4u32 {
            for x in 0..4u32 {
                let want = if (1..3).contains(&y) {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 0, 255]
                };
                assert_eq!(dst.pixel(x, y), want, "({x},{y})");
            }
        }
    }

    #[test]
    fn position_moves_in_output_pixels_and_scale_shrinks_around_the_center() {
        let src = Image::filled(2, 2, [0, 255, 0, 255]).unwrap();
        let mut dst = Image::filled(4, 4, [0, 0, 0, 255]).unwrap();
        // scale 0,5 sobre fit 2 ⇒ 2×2 no centro, deslocado +1 em x
        blit_layer(&mut dst, &src, 1.0, 0.5, 1.0, 0.0, 0.0).unwrap();
        let green = [0, 255, 0, 255];
        for (x, y, g) in [
            (2, 1, true),
            (3, 2, true),
            (1, 1, false),
            (0, 0, false),
            (3, 3, false),
        ] {
            assert_eq!(dst.pixel(x, y) == green, g, "({x},{y})");
        }
    }

    #[test]
    fn opacity_half_blends_and_zero_draws_nothing() {
        let src = Image::filled(1, 1, [255, 255, 255, 255]).unwrap();
        let mut dst = Image::filled(1, 1, [0, 0, 0, 255]).unwrap();
        assert!(!blit_layer(&mut dst, &src, 0.0, 1.0, 0.0, 0.0, 0.0).unwrap());
        assert_eq!(dst.pixel(0, 0), [0, 0, 0, 255]);
        blit_layer(&mut dst, &src, 0.5, 1.0, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(dst.pixel(0, 0), [128, 128, 128, 255]);
    }

    #[test]
    fn rotation_by_multiples_of_90_is_exact() {
        // fonte 2×1: [R, G]; girar 90° horário em saída 2×2 (fit de 1×2): R em cima, G embaixo
        let mut src = Image::transparent(2, 1).unwrap();
        src.put(0, 0, [255, 0, 0, 255]);
        src.put(1, 0, [0, 255, 0, 255]);
        let mut dst = Image::transparent(4, 4).unwrap();
        blit_layer(&mut dst, &src, 1.0, 1.0, 0.0, 0.0, 90.0).unwrap();
        // layer girado 1×2, fit = min(4/1, 4/2) = 2 ⇒ 2 colunas (x ∈ [1,3)) × 4 linhas, bordas em
        // fronteiras de pixel; o vermelho (x = 0 da fonte) vai para o topo, o verde para baixo
        assert_eq!(dst.pixel(1, 0), [255, 0, 0, 255]);
        assert_eq!(dst.pixel(2, 0), [255, 0, 0, 255]);
        assert_eq!(dst.pixel(1, 3), [0, 255, 0, 255]);
        assert_eq!(dst.pixel(2, 3), [0, 255, 0, 255]);
        assert_eq!(
            dst.pixel(0, 0),
            [0, 0, 0, 0],
            "outside the rotated rectangle"
        );
        // 180°: 2×1 em saída 2×1 inverte as colunas exatamente
        let mut d2 = Image::transparent(2, 1).unwrap();
        blit_layer(&mut d2, &src, 1.0, 1.0, 0.0, 0.0, 180.0).unwrap();
        assert_eq!(d2.pixel(0, 0), [0, 255, 0, 255]);
        assert_eq!(d2.pixel(1, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn limits_and_non_finite_input_are_rejected() {
        assert!(Image::filled(0, 4, [0; 4]).is_err());
        assert!(Image::filled(MAX_DIMENSION + 1, 1, [0; 4]).is_err());
        assert!(Image::from_rgba(2, 2, vec![0; 15]).is_err());
        let src = Image::filled(1, 1, [1, 2, 3, 255]).unwrap();
        let mut dst = Image::filled(2, 2, [0, 0, 0, 255]).unwrap();
        assert!(blit_layer(&mut dst, &src, f64::NAN, 1.0, 0.0, 0.0, 0.0).is_err());
        assert!(blit_layer(&mut dst, &src, 1.0, 1.0, f64::INFINITY, 0.0, 0.0).is_err());
    }
}
