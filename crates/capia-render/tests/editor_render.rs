//! Render do editor (Fase 3): texto, transições (dissolve/fade/slide) e fades de clip — o mesmo
//! `render_frame` serve preview e export (ADR-063).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::Command;
use capia_model::{ClipContent, TextStyle, TrackKind, Transition, TransitionKind};
use capia_render::{
    Image, MediaSource, RenderGraph, RenderSettings, frame_digest, mix_audio_range, render_frame,
};
use capia_time::{FrameRate, Rational, Ticks, TimeRange};
use common::*;

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

fn render_at(d: &Dsl, src: &dyn MediaSource, t: i64) -> Image {
    render_sz(d, src, t, 32, 32)
}

fn render_sz(d: &Dsl, src: &dyn MediaSource, t: i64, w: u32, h: u32) -> Image {
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    let f = render_frame(&g, &"S".into(), Ticks(t), &RenderSettings::new(w, h), src).unwrap();
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    f.image
}

/// V1: vermelho [0, 30F) e azul [30F, 60F) (azul com handle de 15F antes do início).
fn two_clips() -> (Dsl, Synth) {
    let mut d = Dsl::new();
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.register("red", 10, true, false);
    d.register("blue", 10, true, false);
    d.media("V1", "a", "red", 0, 30 * F, true, false);
    d.clip(
        "V1",
        "b",
        ClipContent::Media {
            asset: "blue".into(),
            has_video: true,
            has_audio: false,
        },
        30 * F,
        30 * F,
        15 * F,
        Rational::ONE,
    );
    let src = Synth::default()
        .solid_video("red", 16, 16, RED)
        .solid_video("blue", 16, 16, BLUE);
    (d, src)
}

fn set_tr(d: &mut Dsl, kind: TransitionKind, frames: i64) {
    d.run(Command::SetTransition {
        clip: "b".into(),
        transition: Some(Transition {
            kind,
            duration: Ticks(frames * F),
        }),
    });
}

#[test]
fn dissolve_blends_both_sides_of_the_cut_and_is_linear() {
    let (mut d, src) = two_clips();
    set_tr(&mut d, TransitionKind::Dissolve, 20); // região [20F, 40F)
    // fora da região: sólido
    assert_eq!(render_at(&d, &src, 10 * F).pixel(5, 5), RED);
    assert_eq!(render_at(&d, &src, 45 * F).pixel(5, 5), BLUE);
    // início da região: ~100% vermelho; centro (corte): ~50/50; fim: ~100% azul
    let p0 = render_at(&d, &src, 20 * F).pixel(5, 5);
    let pm = render_at(&d, &src, 30 * F).pixel(5, 5);
    let p1 = render_at(&d, &src, 39 * F).pixel(5, 5);
    assert_eq!(p0, RED);
    assert!(
        (i32::from(pm[0]) - 128).abs() <= 2 && (i32::from(pm[2]) - 128).abs() <= 2,
        "{pm:?}"
    );
    assert!(p1[2] > 230 && p1[0] < 30, "{p1:?}");
    // monotonia do vermelho ao longo da região
    let reds: Vec<u8> = (20..40)
        .map(|f| render_at(&d, &src, f * F).pixel(5, 5)[0])
        .collect();
    assert!(reds.windows(2).all(|w| w[0] >= w[1]), "{reds:?}");
}

#[test]
fn fade_dips_each_side_towards_the_background() {
    let (mut d, src) = two_clips();
    set_tr(&mut d, TransitionKind::Fade, 20); // metades: [20F,30F) saindo, [30F,40F) entrando
    assert_eq!(render_at(&d, &src, 10 * F).pixel(5, 5), RED);
    let before = render_at(&d, &src, 25 * F).pixel(5, 5);
    let at_cut = render_at(&d, &src, 30 * F).pixel(5, 5);
    let after = render_at(&d, &src, 35 * F).pixel(5, 5);
    assert!(before[0] > 100 && before[0] < 200, "{before:?}");
    assert_eq!(at_cut, BLACK, "no corte o quadro mergulha no fundo");
    assert!(
        after[2] > 100 && after[2] < 200 && after[0] == 0,
        "{after:?}"
    );
    assert_eq!(render_at(&d, &src, 45 * F).pixel(5, 5), BLUE);
}

#[test]
fn slide_in_enters_from_the_right_over_the_lower_track() {
    let (mut d, src) = two_clips();
    set_tr(&mut d, TransitionKind::SlideIn, 20); // [30F, 50F)
    let start = render_at(&d, &src, 30 * F);
    assert_eq!(
        start.pixel(5, 5),
        BLACK,
        "começa fora do quadro (à direita)"
    );
    let mid = render_at(&d, &src, 40 * F);
    assert_eq!(mid.pixel(5, 5), BLACK, "ainda não cobriu o lado esquerdo");
    assert_eq!(mid.pixel(30, 5), BLUE, "ponta direita já visível");
    assert_eq!(render_at(&d, &src, 55 * F).pixel(5, 5), BLUE);
}

#[test]
fn clip_fade_in_and_out_ramp_the_opacity() {
    let mut d = Dsl::new();
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.register("red", 10, true, false);
    d.media("V1", "a", "red", 0, 60 * F, true, false);
    d.prop("a", "fade_in", 1.0);
    d.prop("a", "fade_out", 1.0);
    let src = Synth::default().solid_video("red", 16, 16, RED);
    assert_eq!(render_at(&d, &src, 0).pixel(5, 5), BLACK);
    let q = render_at(&d, &src, 15 * F).pixel(5, 5)[0]; // 0,5 s de 1 s
    assert!((i32::from(q) - 128).abs() <= 3, "{q}");
    assert_eq!(render_at(&d, &src, 30 * F).pixel(5, 5), RED);
    let tail = render_at(&d, &src, 45 * F).pixel(5, 5)[0]; // 0,5 s antes do fim
    assert!((i32::from(tail) - 128).abs() <= 3, "{tail}");
}

#[test]
fn text_clips_render_with_style_position_and_keyframed_opacity() {
    let mut d = Dsl::new();
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.track("S", "V2", TrackKind::Visual);
    d.solid("V1", "bg", "#202020", 0, 60 * F);
    d.clip(
        "V2",
        "t",
        ClipContent::Text {
            text: "CapIA".into(),
            style: TextStyle {
                size_permille: 200,
                weight: 700,
                ..TextStyle::default()
            },
        },
        0,
        60 * F,
        0,
        Rational::ONE,
    );
    let src = Synth::default();
    let a = render_at(&d, &src, 0);
    assert!(
        a.data.chunks(4).any(|p| p[0] > 200),
        "branco sobre fundo escuro"
    );
    // determinismo
    assert_eq!(frame_digest(&a), frame_digest(&render_at(&d, &src, 0)));
    // opacidade 0 some o texto
    d.prop("t", "opacity", 0.0);
    let hidden = render_at(&d, &src, 0);
    assert!(hidden.data.chunks(4).all(|p| p[..3] == [0x20, 0x20, 0x20]));
    // keyframes de opacidade: aparece ao longo do clip
    d.prop("t", "opacity", 1.0);
    d.keyframe("t", "opacity", 0, 0.0);
    d.keyframe("t", "opacity", 60 * F, 1.0);
    let early = render_at(&d, &src, 3 * F);
    let late = render_at(&d, &src, 57 * F);
    let bright = |i: &Image| i.data.chunks(4).map(|p| u32::from(p[0])).max().unwrap();
    assert!(bright(&late) > bright(&early) + 100);
}

#[test]
fn caption_style_with_background_and_stroke_is_stable() {
    let mut d = Dsl::new();
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.track("S", "V2", TrackKind::Visual);
    d.solid("V1", "bg", "#305070", 0, 30 * F);
    d.clip(
        "V2",
        "cap",
        ClipContent::Text {
            text: "legenda de teste".into(),
            style: TextStyle {
                size_permille: 150,
                background: Some("#000000B0".into()),
                stroke: Some("#FFFFFF".into()),
                stroke_permille: 3,
                color: "#FFE000".into(),
                ..TextStyle::default()
            },
        },
        0,
        30 * F,
        0,
        Rational::ONE,
    );
    d.prop("cap", "position_y", 9.0);
    let a = render_sz(&d, &Synth::default(), 0, 320, 180);
    let b = render_sz(&d, &Synth::default(), F, 320, 180);
    assert_eq!(
        frame_digest(&a),
        frame_digest(&b),
        "frame digest is time-invariant for static text"
    );
    assert!(
        a.data
            .chunks(4)
            .any(|p| p[0] > 200 && p[1] > 180 && p[2] < 80),
        "yellow text"
    );
}

#[test]
fn audio_fades_shape_the_gain() {
    let mut d = Dsl::new();
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "A1", TrackKind::Audio);
    d.register("tone", 10, false, true);
    d.media("A1", "m", "tone", 0, 90 * F, false, true); // 3 s
    d.prop("m", "fade_in", 1.0);
    d.prop("m", "fade_out", 1.0);
    let src = Synth::default().tone("tone", 440.0, 10.0);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    let rms = |from: i64, to: i64| {
        let (buf, _) = mix_audio_range(
            &g,
            &"S".into(),
            TimeRange::new(Ticks(from * F), Ticks((to - from) * F)),
            &RenderSettings::new(16, 16),
            &src,
        )
        .unwrap();
        let sum: f64 = buf
            .samples
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum();
        (sum / buf.samples.len() as f64).sqrt()
    };
    let (head, mid, tail) = (rms(0, 5), rms(40, 50), rms(85, 90));
    assert!(
        head < mid * 0.25,
        "fade-in começa quase mudo: {head} vs {mid}"
    );
    assert!(
        tail < mid * 0.25,
        "fade-out termina quase mudo: {tail} vs {mid}"
    );
}

/// Preview em resolução menor tem a mesma composição do export: o deslocamento (pixels da
/// sequence) é escalado para a saída quando `design_size` está definido.
#[test]
fn design_size_makes_low_res_preview_match_full_res_composition() {
    let (mut d, src) = two_clips();
    d.prop("a", "scale", 0.5);
    d.prop("a", "position_x", 40.0);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    let bbox_x = |w: u32, h: u32, design: Option<(u32, u32)>| -> (f64, f64) {
        let mut st = RenderSettings::new(w, h);
        st.design_size = design;
        let f = render_frame(&g, &"S".into(), Ticks(5 * F), &st, &src).unwrap();
        let xs: Vec<u32> = (0..w).filter(|&x| f.image.pixel(x, h / 2) == RED).collect();
        (
            f64::from(*xs.first().unwrap()) / f64::from(w),
            f64::from(*xs.last().unwrap() + 1) / f64::from(w),
        )
    };
    let big = bbox_x(200, 100, Some((200, 100)));
    let small = bbox_x(100, 50, Some((200, 100)));
    assert!(
        (big.0 - small.0).abs() < 0.03 && (big.1 - small.1).abs() < 0.03,
        "{big:?} {small:?}"
    );
    // sem design_size o deslocamento é literal em pixels (comportamento da Fase 2)
    let lit = bbox_x(100, 50, None);
    assert!((lit.0 - small.0).abs() > 0.1, "{lit:?} {small:?}");
}

#[test]
fn scale_keyframes_grow_the_picture_and_render_is_deterministic() {
    let (mut d, src) = two_clips();
    // 50% → 100% ao longo do clip vermelho (as keyframes do inspector viram estes comandos)
    d.keyframe("a", "scale", 0, 0.5);
    d.keyframe("a", "scale", 29 * F, 1.0);
    // início (50%): o canto é fundo, o centro é vermelho; ponto (6,6) ainda é fundo
    let t0 = render_at(&d, &src, 0);
    assert_eq!(t0.pixel(1, 1), BLACK);
    assert_eq!(t0.pixel(16, 16), RED);
    assert_eq!(t0.pixel(6, 6), BLACK);
    // meio (~75%): (6,6) já é vermelho, o canto ainda não
    let mid = render_at(&d, &src, 15 * F);
    assert_eq!(mid.pixel(6, 6), RED);
    assert_eq!(mid.pixel(1, 1), BLACK);
    // fim (100%): cobre tudo
    let end = render_at(&d, &src, 29 * F);
    assert_eq!(end.pixel(1, 1), RED);
    // o mesmo quadro duas vezes → mesmo digest (preview e export compartilham `render_frame`)
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    let s = RenderSettings::new(32, 32);
    let a = render_frame(&g, &"S".into(), Ticks(15 * F), &s, &src).unwrap();
    let b = render_frame(&g, &"S".into(), Ticks(15 * F), &s, &src).unwrap();
    assert_eq!(frame_digest(&a.image), frame_digest(&b.image));
}
