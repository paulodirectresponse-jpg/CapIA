//! Texto determinístico (Fase 3): UM renderizador serve preview e export. Fonte embutida
//! (Liberation Sans Regular/Bold, SIL OFL 1.1 — `assets/fonts/`); família desconhecida cai em
//! `sans` com aviso (`FONT_FALLBACK`). O tamanho é relativo à altura do quadro (‰), então o mesmo
//! estilo rende igual em qualquer resolução. O resultado é uma imagem do **tamanho do quadro**
//! (transparente) com o bloco de texto já posicionado; `position/scale/opacity` do clip são
//! aplicados depois, pelo compositor, como em qualquer layer.

use crate::error::{RenderError, RenderWarning};
use crate::image::{Image, blend_over, parse_color};
use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont, point};
use capia_model::{TextAlign, TextStyle};

static SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Regular.ttf");
static SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Bold.ttf");

/// Famílias conhecidas (apenas `sans` na Fase 3).
pub const KNOWN_FAMILIES: &[&str] = &["sans"];

fn font_for(style: &TextStyle) -> Result<(FontRef<'static>, bool), RenderError> {
    let bold = style.weight >= 600;
    let data = if bold { SANS_BOLD } else { SANS_REGULAR };
    FontRef::try_from_slice(data)
        .map(|f| (f, !KNOWN_FAMILIES.contains(&style.font_family.as_str())))
        .map_err(|e| RenderError::new("TEXT_FONT_INVALID", format!("embedded font: {e}")))
}

struct Line {
    text: String,
    width: f32,
}

fn measure<F: Font>(font: &impl ScaleFont<F>, s: &str) -> f32 {
    let mut w = 0.0f32;
    let mut prev = None;
    for ch in s.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            w += font.kern(p, id);
        }
        w += font.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Quebra gananciosa por palavras (e por `\n`) no máximo de `max_w` px.
fn wrap<F: Font>(font: &impl ScaleFont<F>, text: &str, max_w: f32) -> Vec<Line> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let candidate = if cur.is_empty() {
                word.to_owned()
            } else {
                format!("{cur} {word}")
            };
            if !cur.is_empty() && measure(font, &candidate) > max_w {
                let width = measure(font, &cur);
                lines.push(Line {
                    text: core::mem::take(&mut cur),
                    width,
                });
                cur = word.to_owned();
            } else {
                cur = candidate;
            }
        }
        let width = measure(font, &cur);
        lines.push(Line { text: cur, width });
    }
    lines
}

fn put_cov(img: &mut Image, x: i64, y: i64, color: [u8; 4], cov: f32) {
    if x < 0 || y < 0 || x >= i64::from(img.width) || y >= i64::from(img.height) {
        return;
    }
    let a = (f32::from(color[3]) * cov.clamp(0.0, 1.0) + 0.5) as u8;
    if a == 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    let dst = img.pixel(x, y);
    let out = blend_over(dst, [color[0], color[1], color[2], a]);
    let i = (y as usize * img.width as usize + x as usize) * 4;
    img.data[i..i + 4].copy_from_slice(&out);
}

fn fill_rect(img: &mut Image, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 4]) {
    let (xa, xb) = (
        (x0.floor() as i64).max(0),
        (x1.ceil() as i64).min(i64::from(img.width)),
    );
    let (ya, yb) = (
        (y0.floor() as i64).max(0),
        (y1.ceil() as i64).min(i64::from(img.height)),
    );
    for y in ya..yb {
        for x in xa..xb {
            put_cov(img, x, y, color, 1.0);
        }
    }
}

/// Desenha `glyphs` na cor dada, deslocados por `(dx, dy)` px.
fn draw_glyphs(
    img: &mut Image,
    font: &FontRef<'_>,
    glyphs: &[Glyph],
    color: [u8; 4],
    dx: f32,
    dy: f32,
) {
    for g in glyphs {
        let mut g = g.clone();
        g.position = point(g.position.x + dx, g.position.y + dy);
        if let Some(og) = font.outline_glyph(g) {
            let b = og.px_bounds();
            og.draw(|x, y, c| {
                put_cov(
                    img,
                    b.min.x as i64 + i64::from(x),
                    b.min.y as i64 + i64::from(y),
                    color,
                    c,
                );
            });
        }
    }
}

/// Rasteriza `text` num quadro `width × height`. Devolve a imagem e os avisos (fonte alternativa).
pub fn render_text(
    width: u32,
    height: u32,
    text: &str,
    style: &TextStyle,
) -> Result<(Image, Vec<RenderWarning>), RenderError> {
    let mut warnings = Vec::new();
    let mut img = Image::transparent(width, height)?;
    let (font, fell_back) = font_for(style)?;
    if fell_back {
        warnings.push(RenderWarning::new(
            "FONT_FALLBACK",
            format!(
                "font family `{}` is not available; using `sans`",
                style.font_family
            ),
        ));
    }
    let color = parse_color(&style.color).unwrap_or([255, 255, 255, 255]);
    let size_px = (f64::from(style.size_permille) * f64::from(height) / 1000.0).max(4.0) as f32;
    let scaled = font.as_scaled(PxScale::from(size_px));
    let margin = width as f32 * 0.05;
    let lines = wrap(&scaled, text, width as f32 - 2.0 * margin);
    let line_h = scaled.height() + scaled.line_gap();
    let block_h = line_h * lines.len() as f32;
    let top = (height as f32 - block_h) / 2.0;

    // posições dos glifos linha a linha
    let mut glyphs: Vec<Glyph> = Vec::new();
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    for (i, line) in lines.iter().enumerate() {
        let x0 = match style.align {
            TextAlign::Left => margin,
            TextAlign::Center => (width as f32 - line.width) / 2.0,
            TextAlign::Right => width as f32 - margin - line.width,
        };
        if !line.text.is_empty() {
            min_x = min_x.min(x0);
            max_x = max_x.max(x0 + line.width);
        }
        let baseline = top + line_h * i as f32 + scaled.ascent();
        let mut x = x0;
        let mut prev = None;
        for ch in line.text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            glyphs.push(id.with_scale_and_position(PxScale::from(size_px), point(x, baseline)));
            x += scaled.h_advance(id);
            prev = Some(id);
        }
    }
    if let Some(bg) = style.background.as_deref().and_then(parse_color)
        && min_x <= max_x
    {
        let pad = size_px * 0.25;
        fill_rect(
            &mut img,
            min_x - pad,
            top - pad,
            max_x + pad,
            top + block_h + pad,
            bg,
        );
    }
    if let (Some(stroke), true) = (
        style.stroke.as_deref().and_then(parse_color),
        style.stroke_permille > 0,
    ) {
        let r = (f64::from(style.stroke_permille) * f64::from(height) / 1000.0).max(1.0) as f32;
        // anel de 16 pontos: determinístico e suficiente para contorno de legenda
        for k in 0..16 {
            let ang = (k as f32) * core::f32::consts::TAU / 16.0;
            draw_glyphs(
                &mut img,
                &font,
                &glyphs,
                stroke,
                r * ang.cos(),
                r * ang.sin(),
            );
        }
    }
    draw_glyphs(&mut img, &font, &glyphs, color, 0.0, 0.0);
    Ok((img, warnings))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn painted(img: &Image) -> usize {
        img.data.chunks(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn text_paints_pixels_near_the_center_and_is_deterministic() {
        let s = TextStyle::default();
        let (a, w) = render_text(640, 360, "Hello CapIA", &s).unwrap();
        let (b, _) = render_text(640, 360, "Hello CapIA", &s).unwrap();
        assert!(w.is_empty());
        assert_eq!(a, b);
        assert!(painted(&a) > 500, "text must paint");
        // nada nas bordas superiores/inferiores (bloco centralizado)
        assert!(a.data[..640 * 4 * 20].chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn size_is_relative_to_frame_height_and_bold_is_heavier() {
        let small = TextStyle::default();
        let big = TextStyle {
            size_permille: 120,
            ..TextStyle::default()
        };
        let bold = TextStyle {
            weight: 700,
            ..TextStyle::default()
        };
        let count = |s: &TextStyle| painted(&render_text(640, 360, "Wide", s).unwrap().0);
        assert!(count(&big) > count(&small) * 2);
        assert!(count(&bold) > count(&small));
        // mesmo estilo, quadro 2x maior: mesma proporção de pixels pintados (±10%)
        let ink = |img: &Image| img.data.chunks(4).map(|p| f64::from(p[3])).sum::<f64>() / 255.0;
        let a = ink(&render_text(640, 360, "Wide", &small).unwrap().0);
        let b = ink(&render_text(1280, 720, "Wide", &small).unwrap().0) / 4.0;
        assert!((a - b).abs() / a < 0.1, "{a} vs {b}");
    }

    #[test]
    fn alignment_background_stroke_and_wrapping() {
        let col_range = |img: &Image| {
            let xs: Vec<u32> = (0..img.width)
                .filter(|&x| (0..img.height).any(|y| img.pixel(x, y)[3] > 0))
                .collect();
            (xs[0], *xs.last().unwrap())
        };
        let left = render_text(
            800,
            450,
            "Hi",
            &TextStyle {
                align: TextAlign::Left,
                ..TextStyle::default()
            },
        )
        .unwrap()
        .0;
        let right = render_text(
            800,
            450,
            "Hi",
            &TextStyle {
                align: TextAlign::Right,
                ..TextStyle::default()
            },
        )
        .unwrap()
        .0;
        assert!(col_range(&left).1 < 400 && col_range(&right).0 > 400);
        let bg = TextStyle {
            background: Some("#FF0000".into()),
            ..TextStyle::default()
        };
        let withbg = render_text(800, 450, "Hi", &bg).unwrap().0;
        assert!(
            painted(&withbg) > painted(&left) * 2,
            "background box must paint"
        );
        let st = TextStyle {
            stroke: Some("#000000".into()),
            stroke_permille: 4,
            ..TextStyle::default()
        };
        let stroked = render_text(800, 450, "Hi", &st).unwrap().0;
        assert!(
            painted(&stroked)
                > painted(
                    &render_text(800, 450, "Hi", &TextStyle::default())
                        .unwrap()
                        .0
                )
        );
        // quebra automática: texto longo ocupa 2+ linhas (altura do bloco cresce)
        let long = "uma frase bastante longa que não cabe numa única linha de legenda neste quadro";
        let one = render_text(800, 450, "curta", &TextStyle::default())
            .unwrap()
            .0;
        let many = render_text(800, 450, long, &TextStyle::default())
            .unwrap()
            .0;
        let rows = |img: &Image| {
            (0..img.height)
                .filter(|&y| (0..img.width).any(|x| img.pixel(x, y)[3] > 0))
                .count()
        };
        assert!(rows(&many) > rows(&one) * 15 / 10);
    }

    #[test]
    fn unknown_family_warns_and_falls_back() {
        let s = TextStyle {
            font_family: "Papyrus".into(),
            ..TextStyle::default()
        };
        let (img, w) = render_text(320, 180, "x", &s).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].code, "FONT_FALLBACK");
        assert!(painted(&img) > 0);
    }
}
