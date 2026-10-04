//! Medidas de desempenho do compositor (não são metas inventadas: o teste só IMPRIME).
//! `cargo test --release -p capia-render --test perf -- --ignored --nocapture`

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_model::TrackKind;
use capia_render::{RenderGraph, RenderSettings, render_frame};
use capia_time::{FrameRate, Rational, Ticks};
use common::{Dsl, F, Synth};
use std::time::Instant;

fn measure(name: &str, g: &RenderGraph, seq: &str, s: &RenderSettings, src: &Synth, frames: i64) {
    let seq = seq.into();
    // aquecimento
    let _ = render_frame(g, &seq, Ticks(0), s, src).unwrap();
    let t0 = Instant::now();
    for n in 0..frames {
        let f = render_frame(g, &seq, Ticks(n * F), s, src).unwrap();
        std::hint::black_box(&f.image.data[0]);
    }
    let el = t0.elapsed();
    let ms = el.as_secs_f64() * 1000.0 / frames as f64;
    eprintln!(
        "PERF render {name:<34} {}x{}: {ms:>7.2} ms/frame  {:>7.1} fps  (frame = {:.1} MiB)",
        s.width,
        s.height,
        1000.0 / ms,
        (s.width * s.height * 4) as f64 / 1_048_576.0
    );
}

#[test]
#[ignore = "medição: rode com --release --ignored --nocapture"]
fn compositor_timings_1080p() {
    let (w, h) = (1920u32, 1080u32);
    let src = Synth::default()
        .solid_video("v1", w, h, [200, 30, 30, 255])
        .solid_video("v2", w, h, [30, 200, 30, 255]);
    let s = RenderSettings::new(w, h);

    // 1 clip, tela cheia (caminho rápido)
    let mut d = Dsl::new();
    d.register("v1", 20, true, false);
    d.register("v2", 20, true, false);
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.media("V1", "a", "v1", 0, 300 * F, true, false);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    measure("1 clip full-frame", &g, "S", &s, &src, 60);

    // 2 layers (cheio + cheio com 50% de opacidade)
    d.track("S", "V2", TrackKind::Visual);
    d.media("V2", "b", "v2", 0, 300 * F, true, false);
    d.prop("b", "opacity", 0.5);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    measure("2 layers (50% opacity)", &g, "S", &s, &src, 60);

    // transform + opacidade (bilinear, escala 0,6, deslocamento)
    d.prop("b", "scale", 0.6);
    d.prop("b", "position_x", 120.0);
    d.prop("b", "position_y", -80.0);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    measure("2 layers + scale/position/opacity", &g, "S", &s, &src, 60);

    // rotação de 90° + escala
    d.prop("b", "rotation", 90.0);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    measure("2 layers + rotation 90°", &g, "S", &s, &src, 60);

    // nested (a sequence filha tem o mesmo clip + overlay) dentro da raiz
    let mut d = Dsl::new();
    d.register("v1", 20, true, false);
    d.register("v2", 20, true, false);
    d.seq("CHILD", FrameRate::FPS_30);
    d.track("CHILD", "CV", TrackKind::Visual);
    d.media("CV", "c1", "v1", 0, 300 * F, true, false);
    d.track("CHILD", "CV2", TrackKind::Visual);
    d.media("CV2", "c2", "v2", 0, 300 * F, true, false);
    d.prop("c2", "opacity", 0.5);
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.nested("V1", "n", "CHILD", 0, 300 * F, 0);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    measure("nested (2 layers in child)", &g, "S", &s, &src, 60);

    // timeline de 10 s (300 quadros), 2 layers com keyframes de opacidade
    let mut d = Dsl::new();
    d.register("v1", 20, true, false);
    d.register("v2", 20, true, false);
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.track("S", "V2", TrackKind::Visual);
    d.media("V1", "a", "v1", 0, 300 * F, true, false);
    d.media("V2", "b", "v2", 0, 300 * F, true, false);
    d.keyframe("b", "opacity", 0, 0.0);
    d.keyframe("b", "opacity", 300 * F, 1.0);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    measure("10 s timeline, 2 layers, keyframes", &g, "S", &s, &src, 300);
    let _ = Rational::ONE;
}
