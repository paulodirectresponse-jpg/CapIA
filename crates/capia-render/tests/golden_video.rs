//! Goldens de vídeo do compositor CPU (10 casos): asserções de **pixels** (semântica) + digest
//! SHA-256 travado em `tests/golden/video.json` (`CAPIA_BLESS=1` regrava).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_model::{ClipContent, TrackKind};
use capia_render::{Image, RenderGraph, RenderSettings, frame_digest, render_frame};
use capia_time::{FrameRate, Rational, Ticks};
use common::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

fn settings() -> RenderSettings {
    RenderSettings::new(16, 16)
}

fn render(d: &Dsl, src: &Synth, seq: &str, t: i64) -> Image {
    let g = RenderGraph::compile(d.doc(), &seq.into()).unwrap();
    let f = render_frame(&g, &seq.into(), Ticks(t), &settings(), src).unwrap();
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    f.image
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/video.json")
}

static BLESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn golden(name: &str, img: &Image) {
    let _guard = BLESS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = golden_path();
    let mut map: BTreeMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json_lite::parse(&s))
        .unwrap_or_default();
    let got = frame_digest(img);
    if std::env::var_os("CAPIA_BLESS").is_some() {
        map.insert(name.to_owned(), got);
        std::fs::write(&path, serde_json_lite::write(&map)).unwrap();
        return;
    }
    assert_eq!(
        map.get(name),
        Some(&got),
        "golden `{name}` changed (CAPIA_BLESS=1 to re-bless after review)"
    );
}

/// Mini JSON objeto string→string (evita dependência só para os goldens).
mod serde_json_lite {
    use std::collections::BTreeMap;
    pub(crate) fn write(m: &BTreeMap<String, String>) -> String {
        let body: Vec<String> = m
            .iter()
            .map(|(k, v)| format!("  \"{k}\": \"{v}\""))
            .collect();
        format!("{{\n{}\n}}\n", body.join(",\n"))
    }
    pub(crate) fn parse(s: &str) -> Option<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for line in s.lines() {
            let l = line.trim().trim_end_matches(',');
            let mut it = l.split("\": \"");
            let (Some(k), Some(v)) = (it.next(), it.next()) else {
                continue;
            };
            out.insert(
                k.trim_start_matches('"').to_owned(),
                v.trim_end_matches('"').to_owned(),
            );
        }
        Some(out)
    }
}

fn full_frame_setup(d: &mut Dsl, color_asset: &str) {
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.register(color_asset, 10, true, false);
}

#[test]
fn g01_one_full_frame_clip() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "red");
    d.media("V1", "c1", "red", 0, 30 * F, true, false);
    let src = Synth::default().solid_video("red", 16, 16, RED);
    let img = render(&d, &src, "S", 5 * F);
    assert!((0..16).all(|y| (0..16).all(|x| px(&img, x, y) == RED)));
    golden("g01_full_frame", &img);
    // fora do clip: preto opaco
    let out = render(&d, &src, "S", 31 * F);
    assert_eq!(px(&out, 3, 3), BLACK);
    golden("g01_outside_black", &out);
}

#[test]
fn g02_two_overlapping_clips_on_two_tracks() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "red");
    d.track("S", "V2", TrackKind::Visual);
    d.register("blue", 10, true, false);
    d.media("V1", "a", "red", 0, 20 * F, true, false);
    d.media("V2", "b", "blue", 10 * F, 20 * F, true, false);
    let src = Synth::default()
        .solid_video("red", 16, 16, RED)
        .solid_video("blue", 16, 16, BLUE);
    // antes da sobreposição: só vermelho; durante: azul por cima; depois: só azul
    assert_eq!(px(&render(&d, &src, "S", 5 * F), 8, 8), RED);
    let both = render(&d, &src, "S", 15 * F);
    assert_eq!(px(&both, 8, 8), BLUE);
    golden("g02_overlap", &both);
    assert_eq!(px(&render(&d, &src, "S", 25 * F), 8, 8), BLUE);
}

#[test]
fn g03_opacity_half() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "red");
    d.media("V1", "c", "red", 0, 30 * F, true, false);
    d.prop("c", "opacity", 0.5);
    let src = Synth::default().solid_video("red", 16, 16, RED);
    let img = render(&d, &src, "S", F);
    assert_eq!(
        px(&img, 4, 4),
        [128, 0, 0, 255],
        "50 % de vermelho sobre preto (half-up)"
    );
    golden("g03_opacity_half", &img);
}

#[test]
fn g04_scale_and_position() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "green");
    d.media("V1", "c", "green", 0, 30 * F, true, false);
    d.prop("c", "scale", 0.5);
    d.prop("c", "position_x", 4.0);
    let src = Synth::default().solid_video("green", 16, 16, GREEN);
    let img = render(&d, &src, "S", F);
    // quadrado 8×8 centrado em (8+4, 8) ⇒ x ∈ [8,16), y ∈ [4,12)
    for y in 0..16u32 {
        for x in 0..16u32 {
            let inside = (8..16).contains(&x) && (4..12).contains(&y);
            assert_eq!(
                px(&img, x, y),
                if inside { GREEN } else { BLACK },
                "({x},{y})"
            );
        }
    }
    golden("g04_scale_position", &img);
}

#[test]
fn g05_transparent_image_over_video() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "blue");
    d.track("S", "V2", TrackKind::Visual);
    d.register("logo", 1, true, false);
    d.media("V1", "v", "blue", 0, 30 * F, true, false);
    d.clip(
        "V2",
        "img",
        ClipContent::Image {
            asset: "logo".into(),
        },
        0,
        30 * F,
        0,
        Rational::ONE,
    );
    // PNG sintético 16×16: metade esquerda transparente, direita vermelha opaca
    let mut logo = Image::transparent(16, 16).unwrap();
    for y in 0..16 {
        for x in 8..16 {
            logo.data[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&RED);
        }
    }
    let src = Synth::default()
        .solid_video("blue", 16, 16, BLUE)
        .still("logo", logo);
    let img = render(&d, &src, "S", F);
    assert_eq!(px(&img, 2, 8), BLUE, "transparent half shows the video");
    assert_eq!(px(&img, 12, 8), RED, "opaque half covers it");
    golden("g05_transparent_image", &img);
}

#[test]
fn g06_nested_sequence_with_transform() {
    let mut d = Dsl::new();
    d.seq("CHILD", FrameRate::FPS_30);
    d.track("CHILD", "CV", TrackKind::Visual);
    d.register("red", 10, true, false);
    d.media("CV", "cc", "red", 0, 20 * F, true, false);
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.nested("V1", "n", "CHILD", 0, 20 * F, 0);
    d.prop("n", "scale", 0.5);
    let src = Synth::default().solid_video("red", 16, 16, RED);
    let img = render(&d, &src, "S", 3 * F);
    for (x, y, red) in [
        (8, 8, true),
        (4, 4, true),
        (11, 11, true),
        (2, 2, false),
        (13, 13, false),
    ] {
        assert_eq!(px(&img, x, y) == RED, red, "({x},{y})");
    }
    golden("g06_nested_scale", &img);
}

#[test]
fn g07_multi_level_nested_compounds_opacity_and_time() {
    // C (vermelho a partir de 1 s do filho) ← B ← A ← S, cada nível a 50 %
    let mut d = Dsl::new();
    d.register("red", 10, true, false);
    d.seq("C", FrameRate::FPS_30);
    d.track("C", "CV", TrackKind::Visual);
    d.media("CV", "cc", "red", 0, 60 * F, true, false);
    d.seq("B", FrameRate::FPS_30);
    d.track("B", "BV", TrackKind::Visual);
    d.nested("BV", "nb", "C", 0, 40 * F, 10 * F); // B(t) = C(t + 10 frames)
    d.prop("nb", "opacity", 0.5);
    d.seq("A", FrameRate::FPS_30);
    d.track("A", "AV", TrackKind::Visual);
    d.nested("AV", "na", "B", 5 * F, 30 * F, 0); // A(t) = B(t − 5 frames)
    d.prop("na", "opacity", 0.5);
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "V1", TrackKind::Visual);
    d.nested("V1", "ns", "A", 0, 40 * F, 0);
    d.prop("ns", "opacity", 0.5);
    let src = Synth::default().solid_video("red", 16, 16, RED);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    assert_eq!(g.sequences.len(), 4);
    // antes de A começar (t < 5 frames) não há B: preto
    assert_eq!(px(&render(&d, &src, "S", 2 * F), 8, 8), BLACK);
    // 12,5 % de vermelho sobre preto: três níveis de 50 % ⇒ 255·0,125 ≈ 32
    let img = render(&d, &src, "S", 10 * F);
    let r = px(&img, 8, 8)[0];
    assert!((31..=33).contains(&r), "got {r}");
    golden("g07_nested_x3", &img);
    // o mapeamento de tempo atravessa os três níveis: o plano de t = 10 frames pede C em 15 frames
    let plan = g.plan_video(&"S".into(), Ticks(10 * F)).unwrap();
    let capia_render::LayerKind::Nested { layers, .. } = &plan[0].kind else {
        panic!()
    };
    let capia_render::LayerKind::Nested { layers: l2, .. } = &layers[0].kind else {
        panic!()
    };
    let capia_render::LayerKind::Nested { layers: l3, .. } = &l2[0].kind else {
        panic!()
    };
    let capia_render::LayerKind::Media { source_t, .. } = &l3[0].kind else {
        panic!()
    };
    assert_eq!(
        *source_t,
        Ticks(15 * F),
        "10 − 5 (A→B) + 10 (B→C) = 15 frames"
    );
}

#[test]
fn g08_vfr_source_holds_the_previous_frame() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "vfr");
    d.media("V1", "c", "vfr", 0, 30 * F, true, false);
    // quadros em 0, 3 e 10 frames (buraco entre eles)
    let src = Synth::default().frames_video(
        "vfr",
        16,
        16,
        vec![
            (Ticks(0), RED),
            (Ticks(3 * F), GREEN),
            (Ticks(10 * F), BLUE),
        ],
    );
    for (t, want) in [
        (0, RED),
        (2 * F, RED),
        (3 * F, GREEN),
        (9 * F, GREEN),
        (10 * F, BLUE),
        (20 * F, BLUE),
    ] {
        assert_eq!(px(&render(&d, &src, "S", t), 5, 5), want, "t = {t}");
    }
    // N/fps cego erraria: em t = 5 frames o quadro apresentado ainda é o verde (índice 1), não o azul
    let img = render(&d, &src, "S", 5 * F);
    assert_eq!(px(&img, 5, 5), GREEN);
    golden("g08_vfr_hold", &img);
}

#[test]
fn g09_retimed_clip_maps_time_by_speed() {
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "strip");
    // 2×: 30 frames de timeline consomem 60 frames de fonte
    d.clip(
        "V1",
        "fast",
        ClipContent::Media {
            asset: "strip".into(),
            has_video: true,
            has_audio: false,
        },
        0,
        30 * F,
        0,
        Rational::from_int(2),
    );
    // fonte: um quadro novo a cada 10 frames, cores distintas
    let colors = [
        RED,
        GREEN,
        BLUE,
        [255, 255, 0, 255],
        [0, 255, 255, 255],
        [255, 0, 255, 255],
    ];
    let frames: Vec<(Ticks, [u8; 4])> = colors
        .iter()
        .enumerate()
        .map(|(i, c)| (Ticks(i as i64 * 10 * F), *c))
        .collect();
    let src = Synth::default().frames_video("strip", 16, 16, frames);
    // t = 5 frames ⇒ fonte 10 frames ⇒ verde; t = 12 ⇒ fonte 24 ⇒ azul; t = 29 ⇒ fonte 58 ⇒ último
    for (t, want) in [(5, GREEN), (12, BLUE), (29, [255, 0, 255, 255])] {
        assert_eq!(
            px(&render(&d, &src, "S", t * F), 1, 1),
            want,
            "t = {t} frames"
        );
    }
    golden("g09_retime_2x", &render(&d, &src, "S", 12 * F));
    // 0,5×: o mesmo clip em câmera lenta
    let mut d2 = Dsl::new();
    full_frame_setup(&mut d2, "strip");
    d2.clip(
        "V1",
        "slow",
        ClipContent::Media {
            asset: "strip".into(),
            has_video: true,
            has_audio: false,
        },
        0,
        30 * F,
        0,
        Rational::new(1, 2).unwrap(),
    );
    assert_eq!(
        px(&render(&d2, &src, "S", 25 * F), 1, 1),
        GREEN,
        "25 frames × 0,5 = 12,5 frames de fonte ⇒ quadro de 10 (verde)"
    );
}

#[test]
fn g10_track_order_defines_z_order() {
    let build = |bottom: &str, top: &str| {
        let mut d = Dsl::new();
        d.seq("S", FrameRate::FPS_30);
        d.track("S", "V1", TrackKind::Visual);
        d.track("S", "V2", TrackKind::Visual);
        d.register("red", 10, true, false);
        d.register("blue", 10, true, false);
        d.media("V1", "lo", bottom, 0, 30 * F, true, false);
        d.media("V2", "hi", top, 0, 30 * F, true, false);
        d
    };
    let src = Synth::default()
        .solid_video("red", 16, 16, RED)
        .solid_video("blue", 16, 16, BLUE);
    let a = render(&build("red", "blue"), &src, "S", F);
    let b = render(&build("blue", "red"), &src, "S", F);
    assert_eq!(px(&a, 0, 0), BLUE);
    assert_eq!(px(&b, 0, 0), RED);
    golden("g10_z_blue_on_red", &a);
    golden("g10_z_red_on_blue", &b);
}

#[test]
fn hidden_tracks_disabled_clips_and_unsupported_content_behave() {
    use capia_render::RenderWarning;
    let mut d = Dsl::new();
    full_frame_setup(&mut d, "red");
    d.media("V1", "c", "red", 0, 30 * F, true, false);
    d.clip(
        "V1",
        "late",
        ClipContent::Text { text: "hi".into() , style: Default::default() },
        31 * F,
        5 * F,
        0,
        Rational::ONE,
    );
    let src = Synth::default().solid_video("red", 16, 16, RED);
    let g = RenderGraph::compile(d.doc(), &"S".into()).unwrap();
    assert_eq!(
        g.warnings(),
        vec![RenderWarning::new(
            "TEXT_NOT_RENDERED",
            "clip late is text; text rendering is not part of Phase 2"
        )]
    );
    // texto em t = 32 frames: aviso, nada desenhado
    let f = render_frame(&g, &"S".into(), Ticks(32 * F), &settings(), &src).unwrap();
    assert_eq!(f.warnings.len(), 1);
    assert_eq!(px(&f.image, 3, 3), BLACK);
    // fonte ausente: erro no modo estrito, aviso + layer pulado no modo preview
    let empty = Synth::default();
    assert_eq!(
        render_frame(&g, &"S".into(), Ticks(F), &settings(), &empty)
            .unwrap_err()
            .code,
        "RENDER_SOURCE_FAILED"
    );
    let mut lenient = settings();
    lenient.strict_sources = false;
    let f = render_frame(&g, &"S".into(), Ticks(F), &lenient, &empty).unwrap();
    assert_eq!(f.warnings[0].code, "SOURCE_UNAVAILABLE");
    assert_eq!(px(&f.image, 3, 3), BLACK);
}
