//! Medidas do render/export do projeto com mídia real 1080p (só IMPRIME).
//! `cargo test --release -p capia-project --test perf_render -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_media::ExportCodec;
use capia_model::TrackKind;
use capia_project::{ExportOptions, Mp4Options};
use capia_render::RenderSettings;
use capia_time::{FrameRate, Ticks, TimeRange};
use lab::{F, Lab};
use std::time::Instant;

#[test]
#[ignore = "medição: rode com --release --ignored --nocapture"]
fn project_render_and_export_timings_1080p() {
    let tc = need!();
    let mut lab = Lab::new("perf", &tc);
    let path = lab.dir.join("hd.mp4");
    lab.ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1920x1080:rate=30",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=sin(2*PI*t*(300+40*t))|sin(2*PI*t*(500+25*t)):s=48000:c=stereo",
        "-t",
        "10",
        "-c:v",
        "mpeg4",
        "-g",
        "30",
        "-qscale:v",
        "4",
        "-c:a",
        "aac",
        path.to_str().unwrap(),
    ]);
    let a = lab.import(&path);
    lab.seq("S", FrameRate::FPS_30);
    lab.track("S", "V1", TrackKind::Visual);
    lab.track("S", "V2", TrackKind::Visual);
    lab.track("S", "A1", TrackKind::Audio);
    lab.media("V1", "c1", &a, 0, 270 * F, 0, true, false);
    lab.media("V2", "c2", &a, 0, 270 * F, 0, true, false);
    lab.prop("c2", "opacity", 0.5);
    lab.prop("c2", "scale", 0.5);
    lab.media("A1", "ca", &a, 0, 270 * F, 0, false, true);
    let seq = "S".into();
    let s = RenderSettings::new(1920, 1080);
    let frames = 90i64;
    let range = TimeRange::new(Ticks(0), Ticks(frames * F));

    // render de range: 2 layers reais (decode + composição) – cold e warm
    for pass in ["cold (decode + caches vazios)", "warm (frame cache cheio)"] {
        let t0 = Instant::now();
        let (n, _) = lab
            .p()
            .render_range(&lab.services, &seq, range, &s, &mut |_, _, img| {
                std::hint::black_box(&img.data[0]);
                true
            })
            .unwrap();
        let el = t0.elapsed();
        let m = lab.services.decode().metrics();
        eprintln!(
            "PERF project render_range 1080p 2 layers, {pass}: {n} frames in {:.0} ms = {:.1} ms/frame, {:.1} fps; frame cache {:.1} MiB ({} entries), hits {}, sessions {}",
            el.as_secs_f64() * 1000.0,
            el.as_secs_f64() * 1000.0 / n as f64,
            n as f64 / el.as_secs_f64(),
            m.cache.bytes as f64 / 1_048_576.0,
            m.cache.entries,
            m.cache.hits,
            m.sessions_opened
        );
    }
    let t0 = Instant::now();
    let (buf, _) = lab
        .p()
        .render_audio_range(&lab.services, &seq, range, &s)
        .unwrap();
    eprintln!(
        "PERF project render_audio_range 3 s stereo 48 kHz: {:.0} ms ({} samples)",
        t0.elapsed().as_secs_f64() * 1000.0,
        buf.samples.len()
    );
    let out = lab.dir.join("exp");
    let t0 = Instant::now();
    lab.p()
        .export_intermediate(
            &lab.services,
            &seq,
            range,
            &s,
            &out,
            &ExportOptions::default(),
            &|| false,
        )
        .unwrap();
    let el = t0.elapsed();
    eprintln!(
        "PERF export_intermediate 1080p {frames} frames: {:.0} ms = {:.1} fps ({:.0} MiB raw)",
        el.as_secs_f64() * 1000.0,
        frames as f64 / el.as_secs_f64(),
        (frames * 1920 * 1080 * 4) as f64 / 1_048_576.0
    );
    let mp4 = lab.dir.join("o.mp4");
    let t0 = Instant::now();
    let r = lab
        .p()
        .export_mp4(
            &lab.services,
            &seq,
            range,
            &s,
            &mp4,
            &Mp4Options {
                codec: ExportCodec::Mpeg4Reference,
                ..Mp4Options::default()
            },
            &|| false,
        )
        .unwrap();
    let el = t0.elapsed();
    eprintln!(
        "PERF export_mp4 ({}) 1080p {} frames: {:.0} ms = {:.1} fps, file {:.1} MiB",
        r.encoder,
        r.frames,
        el.as_secs_f64() * 1000.0,
        r.frames as f64 / el.as_secs_f64(),
        std::fs::metadata(&mp4).unwrap().len() as f64 / 1_048_576.0
    );
}
