//! Render do projeto com mídia REAL: o compositor puro + decode service + índice/PCM de áudio.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_decode::Priority;
use capia_media::{DecodeLimits, FrameIndex, decode_frame_by_index};
use capia_model::TrackKind;
use capia_render::{RenderSettings, frame_digest};
use capia_time::{FrameRate, Ticks, TimeRange};
use lab::{F, Lab};

fn fps30() -> FrameRate {
    FrameRate::FPS_30
}

fn ref_frame(lab: &Lab, path: &std::path::Path, i: usize) -> Vec<u8> {
    let info =
        capia_media::MediaProbe::probe(&capia_media::FfprobeBackend::new(lab.tc.clone()), path)
            .unwrap();
    let v = info.video().unwrap().clone();
    let idx: FrameIndex = capia_media::build_frame_index(
        &lab.tc,
        path,
        v.index,
        v.time_base.unwrap(),
        0,
        &capia_media::IndexOptions::default(),
        &|| false,
        &mut |_, _| {},
    )
    .unwrap();
    decode_frame_by_index(
        &lab.tc,
        path,
        &idx,
        v.width,
        v.height,
        i,
        &DecodeLimits::default(),
        &|| false,
    )
    .unwrap()
    .bytes
}

#[test]
fn full_frame_clip_renders_exactly_the_decoded_frame_and_survives_reopen() {
    let tc = need!();
    let mut lab = Lab::new("full", &tc);
    let path = lab.gen_av("a.mp4", "30", 3, 48_000);
    let a = lab.import(&path);
    lab.seq("S", fps30());
    lab.track("S", "V1", TrackKind::Visual);
    lab.media("V1", "c1", &a, 0, 60 * F, 0, true, false);
    let s = RenderSettings::new(64, 48);
    let seq = "S".into();
    let mut digests = Vec::new();
    for n in [0i64, 7, 29, 59] {
        let f = lab
            .p()
            .render_frame(&lab.services, &seq, Ticks(n * F), &s)
            .unwrap();
        assert_eq!(
            f.image.data,
            ref_frame(&lab, &path, n as usize),
            "frame {n}"
        );
        assert!(f.warnings.is_empty());
        digests.push(frame_digest(&f.image));
    }
    // fecha/reabre: caches de memória frios, mesmos bytes
    lab.reopen();
    for (k, n) in [0i64, 7, 29, 59].iter().enumerate() {
        let f = lab
            .p()
            .render_frame(&lab.services, &seq, Ticks(n * F), &s)
            .unwrap();
        assert_eq!(frame_digest(&f.image), digests[k]);
    }
}

#[test]
fn render_range_is_sequential_and_matches_single_frame_renders() {
    let tc = need!();
    let mut lab = Lab::new("range", &tc);
    let path = lab.gen_av("a.mp4", "30", 2, 48_000);
    let a = lab.import(&path);
    lab.seq("S", fps30());
    lab.track("S", "V1", TrackKind::Visual);
    lab.media("V1", "c1", &a, 0, 40 * F, 0, true, false);
    let s = RenderSettings::new(64, 48);
    let seq = "S".into();
    let mut got = Vec::new();
    let (n, w) = lab
        .p()
        .render_range(
            &lab.services,
            &seq,
            TimeRange::new(Ticks(0), Ticks(20 * F)),
            &s,
            &mut |i, t, img| {
                got.push((i, t, frame_digest(&img)));
                true
            },
        )
        .unwrap();
    assert_eq!(n, 20);
    assert!(w.is_empty());
    for (i, t, d) in &got {
        assert_eq!(t.0, i * F);
        let f = lab.p().render_frame(&lab.services, &seq, *t, &s).unwrap();
        assert_eq!(&frame_digest(&f.image), d, "frame {i}");
    }
    // um único processo de decode atendeu a faixa sequencial
    let m = lab.services.decode().metrics();
    assert!(m.sessions_opened <= 2, "{m:?}");
    let _ = Priority::Playback;
}

#[test]
fn audio_range_matches_the_native_decode_at_the_same_rate() {
    let tc = need!();
    let mut lab = Lab::new("audio", &tc);
    let path = lab.gen_av("a.mp4", "30", 3, 48_000);
    let a = lab.import(&path);
    lab.seq("S", fps30());
    lab.track("S", "A1", TrackKind::Audio);
    lab.media("A1", "c1", &a, 0, 60 * F, 0, false, true);
    let s = RenderSettings::new(64, 48);
    let (buf, w) = lab
        .p()
        .render_audio_range(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(0), Ticks(30 * F)),
            &s,
        )
        .unwrap();
    assert!(w.is_empty());
    assert_eq!(buf.sample_rate, 48_000);
    assert_eq!(buf.samples.len(), 48_000 * 2);
    let full = lab
        .p()
        .decode_audio(
            &a,
            Ticks(0),
            Ticks(705_600_000),
            None,
            None,
            &lab.tc,
            &|| false,
        )
        .unwrap();
    // o mixer faz hard clip em ±1 (o AAC ultrapassa 1,0 em picos): compara com o decode clipado
    let worst = buf
        .samples
        .iter()
        .zip(&full.samples)
        .map(|(x, y)| (x - y.clamp(-1.0, 1.0)).abs())
        .fold(0f32, f32::max);
    assert!(worst <= 1e-6, "worst {worst}");
}

#[test]
fn audio_is_resampled_to_the_output_rate() {
    let tc = need!();
    let mut lab = Lab::new("resample", &tc);
    let path = lab.gen_av("a44.mp4", "30", 3, 44_100);
    let a = lab.import(&path);
    lab.seq("S", fps30());
    lab.track("S", "A1", TrackKind::Audio);
    lab.media("A1", "c1", &a, 0, 60 * F, 0, false, true);
    let s = RenderSettings::new(64, 48);
    let (buf, _) = lab
        .p()
        .render_audio_range(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(F * 15), Ticks(15 * F)),
            &s,
        )
        .unwrap();
    assert_eq!(buf.samples.len(), 24_000 * 2);
    let peak = buf.samples.iter().fold(0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.5 && peak <= 1.0, "peak {peak}");
    // determinismo: mesma coisa de novo (cache de PCM quente) é idêntica byte a byte
    let (again, _) = lab
        .p()
        .render_audio_range(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(F * 15), Ticks(15 * F)),
            &s,
        )
        .unwrap();
    assert_eq!(buf.samples, again.samples);
}

/// O grafo compilado é reaproveitado entre quadros, mas **nunca** serve um grafo antigo: editar,
/// desfazer e refazer mudam a revisão e o quadro acompanha exatamente (o cache é só desempenho).
#[test]
fn graph_cache_never_serves_a_stale_graph_across_edit_undo_redo() {
    let tc = need!();
    let mut lab = Lab::new("gcache", &tc);
    lab.seq("S", fps30());
    lab.track("S", "V1", TrackKind::Visual);
    lab.clip(
        "V1",
        "c1",
        capia_model::ClipContent::Solid {
            color: "#FF0000".into(),
        },
        0,
        30 * F,
        0,
        capia_time::Rational::ONE,
    );
    let s = RenderSettings::new(16, 16);
    let seq = "S".into();
    let render = |lab: &Lab| {
        frame_digest(
            &lab.p()
                .render_frame(&lab.services, &seq, Ticks(5 * F), &s)
                .unwrap()
                .image,
        )
    };
    let red = render(&lab);
    // mesmo quadro duas vezes: cache quente, mesmo resultado
    assert_eq!(render(&lab), red);
    lab.prop("c1", "opacity", 0.0);
    let transparent = render(&lab);
    assert_ne!(
        transparent, red,
        "edição precisa invalidar o grafo em cache"
    );
    lab.pm().undo(&capia_commands::Actor::user("lab")).unwrap();
    assert_eq!(render(&lab), red, "undo precisa invalidar o grafo em cache");
    lab.pm().redo(&capia_commands::Actor::user("lab")).unwrap();
    assert_eq!(
        render(&lab),
        transparent,
        "redo precisa invalidar o grafo em cache"
    );
}
