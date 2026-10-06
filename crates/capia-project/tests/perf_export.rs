//! RC3: onde o export MP4 gasta o tempo (decode / composição / escrita no encoder / áudio / mux /
//! validação) em um trecho curto e em um maior, com mídia 1080p real. Só IMPRIME e grava JSON em
//! `target/rc3-export-bench.json`.
//! `cargo test --release -p capia-project --test perf_export -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_media::ExportCodec;
use capia_model::TrackKind;
use capia_project::Mp4Options;
use capia_render::RenderSettings;
use capia_time::{FrameRate, Ticks, TimeRange};
use lab::{F, Lab};
use serde_json::json;

fn scenario(name: &str, src_secs: u32, frames: i64, layers: u32) -> Option<serde_json::Value> {
    let tc = lab::toolchain()?;
    let mut lab = Lab::new(name, &tc);
    let path = lab.dir.join("hd.mp4");
    let secs = src_secs.to_string();
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
        &secs,
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
    for l in 0..layers {
        lab.track("S", &format!("V{l}"), TrackKind::Visual);
    }
    lab.track("S", "A1", TrackKind::Audio);
    for l in 0..layers {
        let id = format!("c{l}");
        lab.media(
            &format!("V{l}"),
            &id,
            &a,
            0,
            i64::from(src_secs) * 30 * F,
            0,
            true,
            false,
        );
        if l > 0 {
            lab.prop(&id, "opacity", 0.5);
            lab.prop(&id, "scale", 0.5);
        }
    }
    lab.media(
        "A1",
        "ca",
        &a,
        0,
        i64::from(src_secs) * 30 * F,
        0,
        false,
        true,
    );
    let seq = "S".into();
    let s = RenderSettings::new(1920, 1080);
    let range = TimeRange::new(Ticks(0), Ticks(frames * F));
    let caps = capia_project::Project::detect_export_encoders(&lab.services).unwrap();
    let codec = if caps.iter().any(|c| c.codec == "h264" && c.available) {
        ExportCodec::H264
    } else {
        ExportCodec::Mpeg4Reference
    };
    let mp4 = lab.dir.join("o.mp4");
    let r = lab
        .p()
        .export_mp4(
            &lab.services,
            &seq,
            range,
            &s,
            &mp4,
            &Mp4Options {
                codec,
                capabilities: Some(caps),
                ..Mp4Options::default()
            },
            &|| false,
        )
        .unwrap();
    let t = &r.timings;
    eprintln!(
        "PERF export {name}: {} {}x{} {} frames, {} layer(s) | total {:.0} ms = {:.2}x realtime, loop {:.1} fps | audio render {:.0} + wav {:.0} | encoder start {:.0} | render loop {:.0} = decode {:.0} + composite {:.0} + encoder-wait {:.0} (writer busy {:.0}) | drain {:.0} | validate {:.0} | sync {:.0}",
        r.encoder,
        r.width,
        r.height,
        r.frames,
        layers,
        t.total_ms,
        t.realtime_factor,
        t.render_fps,
        t.audio_render_ms,
        t.audio_write_ms,
        t.encoder_start_ms,
        t.render_ms,
        t.decode_ms,
        t.composite_ms,
        t.encoder_wait_ms,
        t.encode_write_ms,
        t.encoder_drain_ms,
        t.validate_ms,
        t.sync_publish_ms
    );
    Some(
        json!({ "scenario": name, "encoder": r.encoder, "frames": r.frames, "layers": layers,
            "width": r.width, "height": r.height, "timings": r.timings }),
    )
}

#[test]
#[ignore = "medição: rode com --release --ignored --nocapture"]
fn export_stage_timings_short_and_larger() {
    let runs: Vec<_> = [
        scenario("short-3s-1layer", 10, 90, 1),
        scenario("short-3s-2layers", 10, 90, 2),
        scenario("larger-30s-2layers", 40, 900, 2),
    ]
    .into_iter()
    .flatten()
    .collect();
    let out =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/rc3-export-bench.json");
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&json!({ "runs": runs })).unwrap(),
    )
    .unwrap();
    eprintln!("wrote {}", out.display());
}
