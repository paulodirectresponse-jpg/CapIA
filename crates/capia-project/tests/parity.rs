//! Paridade e determinismo: preview × export com hashes exatos, reopen, cache frio/quente,
//! proxy presente/ausente — o proxy nunca define a verdade do export.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_media::ProxyProfileV1;
use capia_model::TrackKind;
use capia_preview::{HeadlessSink, ManualClock, PreviewScheduler};
use capia_project::{ExportOptions, SourceOptions};
use capia_render::{MediaSource, RenderSettings, frame_digest};
use capia_time::{FrameRate, Rational, Ticks, TimeRange};
use lab::{F, Lab};
use std::sync::Arc;
use std::time::Duration;

fn no_cancel() -> bool {
    false
}

/// V1: vídeo cheio; V2: nested (outro trecho do mesmo vídeo) a 50%, menor e deslocado; áudio.
fn build(tag: &str, tc: &capia_media::MediaToolchain) -> Lab {
    let mut lab = Lab::new(tag, tc);
    let path = lab.gen_av("a.mp4", "30", 3, 48_000);
    let a = lab.import(&path);
    lab.seq("CHILD", FrameRate::FPS_30);
    lab.track("CHILD", "CV", TrackKind::Visual);
    lab.media("CV", "cc", &a, 0, 20 * F, 30 * F, true, false);
    lab.seq("S", FrameRate::FPS_30);
    lab.track("S", "V1", TrackKind::Visual);
    lab.track("S", "V2", TrackKind::Visual);
    lab.track("S", "A1", TrackKind::Audio);
    lab.media("V1", "c1", &a, 0, 30 * F, 0, true, false);
    lab.nested("V2", "n1", "CHILD", 5 * F, 20 * F, 0);
    lab.prop("n1", "opacity", 0.5);
    lab.prop("n1", "scale", 0.5);
    lab.prop("n1", "position_x", 12.0);
    lab.media("A1", "ca", &a, 0, 30 * F, 0, false, true);
    lab
}

fn settings() -> RenderSettings {
    RenderSettings::new(80, 60)
}

fn export_digests(lab: &Lab) -> Vec<String> {
    let out = lab
        .dir
        .join(format!("exp-{}", lab.dir.read_dir().unwrap().count()));
    lab.p()
        .export_intermediate(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(0), Ticks(30 * F)),
            &settings(),
            &out,
            &ExportOptions::default(),
            &no_cancel,
        )
        .unwrap()
        .frame_digests
}

#[test]
fn preview_playback_matches_export_frame_for_frame() {
    let tc = need!();
    let lab = build("parity", &tc);
    let exported = export_digests(&lab);
    assert_eq!(exported.len(), 30);
    // preview: a MESMA fonte e o mesmo grafo, tocando pelo scheduler com relógio manual
    let seq = "S".into();
    let graph = Arc::new(lab.p().render_graph(&seq).unwrap());
    let source: Arc<dyn MediaSource> = Arc::new(
        lab.p()
            .render_source(&lab.services, &graph, SourceOptions::default())
            .unwrap(),
    );
    let sink = HeadlessSink::new();
    let clock = Arc::new(ManualClock::new());
    let s = PreviewScheduler::new(
        Arc::clone(&graph),
        seq,
        source,
        settings(),
        Box::new(sink.clone()),
        clock.clone(),
    )
    .unwrap();
    s.play(Ticks(0), Rational::ONE);
    assert!(sink.wait_presented(1, Duration::from_secs(60)));
    for k in 1..30 {
        clock.advance(Ticks(F));
        s.poke();
        assert!(
            sink.wait_presented(k + 1, Duration::from_secs(60)),
            "frame {k}"
        );
    }
    let p = sink.presented();
    assert_eq!(p.len(), 30);
    assert!(sink.dropped().is_empty());
    for (k, f) in p.iter().enumerate() {
        assert_eq!(
            f.digest, exported[k],
            "preview frame {k} differs from the export"
        );
    }
    // scrub aleatório também bate com o export
    for n in [27usize, 3, 15, 0, 29] {
        s.request(Ticks(n as i64 * F));
        let want = sink.presented().len() + 1;
        assert!(sink.wait_presented(want, Duration::from_secs(60)));
        assert_eq!(
            sink.presented().last().unwrap().digest,
            exported[n],
            "seek {n}"
        );
    }
}

#[test]
fn export_is_identical_after_reopen_cold_cache_and_with_a_proxy_present() {
    let tc = need!();
    let mut lab = build("determinism", &tc);
    let first = export_digests(&lab);
    // mesma sessão, caches quentes
    assert_eq!(export_digests(&lab), first, "warm cache");
    // reabre o projeto: serviços e caches de memória frios
    lab.reopen();
    assert_eq!(export_digests(&lab), first, "after reopen");
    // com um proxy gerado (resolução menor, MJPEG): o export NÃO pode mudar
    let asset = lab
        .p()
        .document()
        .assets()
        .next()
        .map(|a| a.id.clone())
        .unwrap();
    let proxy = lab
        .p()
        .proxy(&asset, &ProxyProfileV1::default(), &lab.tc, &|| false)
        .unwrap();
    assert!(proxy.exists());
    lab.reopen();
    assert_eq!(export_digests(&lab), first, "with proxy present");
    // frame cache pequeno (evicções constantes) não muda nada
    let mut cfg = capia_decode::DecodeConfig::new(lab.tc.clone());
    cfg.frame_cache_bytes = 3 * 64 * 48 * 4;
    cfg.max_sessions = 1;
    lab.services = Arc::new(capia_project::RenderServices::with_config(cfg, 1 << 20));
    assert_eq!(export_digests(&lab), first, "tiny caches");
}

#[test]
fn single_frame_renders_are_independent_of_the_order_they_are_asked_in() {
    let tc = need!();
    let lab = build("order", &tc);
    let seq = "S".into();
    let s = settings();
    let mut forward = Vec::new();
    for n in 0..30i64 {
        let f = lab
            .p()
            .render_frame(&lab.services, &seq, Ticks(n * F), &s)
            .unwrap();
        forward.push(frame_digest(&f.image));
    }
    let mut order: Vec<i64> = (0..30).collect();
    // permutação determinística (passo coprimo com 30)
    order = order.iter().map(|i| (i * 7 + 3) % 30).collect();
    for n in order {
        let f = lab
            .p()
            .render_frame(&lab.services, &seq, Ticks(n * F), &s)
            .unwrap();
        assert_eq!(frame_digest(&f.image), forward[n as usize], "frame {n}");
    }
}
