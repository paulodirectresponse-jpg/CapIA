//! Export headless: intermediário atômico, MP4 por encoder aprovado, validação por ffprobe.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_media::{ExportCodec, MediaErrorCode};
use capia_model::TrackKind;
use capia_project::{ExportOptions, Mp4Options, ProjectError};
use capia_render::{RenderSettings, frame_digest};
use capia_time::{FrameRate, Ticks, TimeRange};
use lab::{F, Lab};
use std::path::Path;

fn no_cancel() -> bool {
    false
}

fn setup(tag: &str, tc: &capia_media::MediaToolchain) -> Lab {
    let mut lab = Lab::new(tag, tc);
    let path = lab.gen_av("a.mp4", "30", 3, 48_000);
    let a = lab.import(&path);
    lab.seq("S", FrameRate::FPS_30);
    lab.track("S", "V1", TrackKind::Visual);
    lab.track("S", "A1", TrackKind::Audio);
    lab.media("V1", "c1", &a, 0, 60 * F, 0, true, false);
    lab.media("A1", "c2", &a, 0, 60 * F, 0, false, true);
    lab
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".partial-"))
        .collect()
}

#[test]
fn intermediate_export_is_atomic_complete_and_matches_the_renderer() {
    let tc = need!();
    let lab = setup("inter", &tc);
    let out = lab.dir.join("exp");
    let s = RenderSettings::new(64, 48);
    let seq = "S".into();
    let range = TimeRange::new(Ticks(0), Ticks(20 * F));
    let r = lab
        .p()
        .export_intermediate(
            &lab.services,
            &seq,
            range,
            &s,
            &out,
            &ExportOptions::default(),
            &no_cancel,
        )
        .unwrap();
    assert_eq!(r.frames, 20);
    assert_eq!(r.audio_frames, 32_000);
    assert!(leftovers(&lab.dir).is_empty());
    assert_eq!(
        std::fs::metadata(out.join("video.rgba")).unwrap().len(),
        20 * 64 * 48 * 4
    );
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["format"], "capia-intermediate-v1");
    assert_eq!(m["frames"], 20);
    assert_eq!(m["video_sha256"], r.video_sha256);
    // cada digest do manifesto = o digest de render_frame daquele instante
    for (i, d) in r.frame_digests.iter().enumerate() {
        let f = lab
            .p()
            .render_frame(&lab.services, &seq, Ticks(i as i64 * F), &s)
            .unwrap();
        assert_eq!(&frame_digest(&f.image), d, "frame {i}");
    }
    // não sobrescreve sem pedir; com overwrite, troca
    let again = lab
        .p()
        .export_intermediate(
            &lab.services,
            &seq,
            range,
            &s,
            &out,
            &ExportOptions::default(),
            &no_cancel,
        )
        .unwrap_err();
    assert_eq!(again.code(), "EXPORT_EXISTS");
    let r2 = lab
        .p()
        .export_intermediate(
            &lab.services,
            &seq,
            range,
            &s,
            &out,
            &ExportOptions { overwrite: true },
            &no_cancel,
        )
        .unwrap();
    assert_eq!(r2.video_sha256, r.video_sha256);
    assert!(leftovers(&lab.dir).is_empty());
}

#[test]
fn cancelled_export_publishes_nothing_and_cleans_its_staging_area() {
    let tc = need!();
    let lab = setup("cancel", &tc);
    let out = lab.dir.join("exp");
    let s = RenderSettings::new(64, 48);
    let n = std::cell::Cell::new(0);
    let cancel = || {
        n.set(n.get() + 1);
        n.get() > 5
    };
    let e = lab
        .p()
        .export_intermediate(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(0), Ticks(20 * F)),
            &s,
            &out,
            &ExportOptions::default(),
            &cancel,
        )
        .unwrap_err();
    assert_eq!(e.code(), "EXPORT_CANCELLED");
    assert!(!out.exists());
    assert!(leftovers(&lab.dir).is_empty());
}

#[test]
fn stale_partials_from_a_killed_export_are_cleaned_on_the_next_run() {
    let tc = need!();
    let lab = setup("stale", &tc);
    let out = lab.dir.join("exp");
    let stale = lab.dir.join("exp.partial-99999-0");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("video.rgba"), b"half").unwrap();
    let s = RenderSettings::new(64, 48);
    lab.p()
        .export_intermediate(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(0), Ticks(4 * F)),
            &s,
            &out,
            &ExportOptions::default(),
            &no_cancel,
        )
        .unwrap();
    assert!(!stale.exists());
    assert!(out.join("manifest.json").exists());
}

#[test]
fn encoder_detection_reports_every_backend_and_never_offers_gpl() {
    let tc = need!();
    let lab = setup("enc", &tc);
    let caps = capia_project::Project::detect_export_encoders(&lab.services).unwrap();
    let names: Vec<_> = caps.iter().map(|c| c.ffmpeg_name.as_str()).collect();
    for want in [
        "h264_nvenc",
        "h264_qsv",
        "h264_amf",
        "h264_mf",
        "libopenh264",
        "libx264",
        "mpeg4",
    ] {
        assert!(names.contains(&want), "{want} missing from the report");
    }
    for c in &caps {
        if c.ffmpeg_name.starts_with("libx26") {
            assert!(!c.available, "{} must never be available", c.ffmpeg_name);
            assert!(
                c.reason_unavailable
                    .as_deref()
                    .unwrap()
                    .contains("prohibited")
            );
        }
        if !c.available {
            assert!(
                c.reason_unavailable.is_some(),
                "{} needs a reason",
                c.ffmpeg_name
            );
        }
    }
    eprintln!(
        "available H.264 encoders: {:?}",
        caps.iter()
            .filter(|c| c.codec == "h264" && c.available)
            .map(|c| &c.ffmpeg_name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn h264_export_uses_an_approved_encoder_or_fails_loudly_without_publishing() {
    let tc = need!();
    let lab = setup("h264", &tc);
    let out = lab.dir.join("o.mp4");
    let s = RenderSettings::new(64, 48);
    let caps = capia_project::Project::detect_export_encoders(&lab.services).unwrap();
    let have = caps.iter().any(|c| {
        c.codec == "h264" && c.available && c.policy != capia_media::EncoderPolicy::Prohibited
    });
    let opts = Mp4Options {
        capabilities: Some(caps),
        ..Mp4Options::default()
    };
    let r = lab.p().export_mp4(
        &lab.services,
        &"S".into(),
        TimeRange::new(Ticks(0), Ticks(30 * F)),
        &s,
        &out,
        &opts,
        &no_cancel,
    );
    if have {
        let r = r.unwrap();
        assert_eq!(r.codec, "h264");
        assert!(!r.encoder.contains("x264") && !r.encoder.contains("x265"));
        assert!(out.exists());
        assert_eq!(r.frames, 30);
    } else {
        let e = r.unwrap_err();
        let ProjectError::Asset(a) = &e else {
            panic!("{e:?}")
        };
        assert_eq!(
            a.code.to_string(),
            MediaErrorCode::MediaEncoderUnavailable.as_str()
        );
        assert!(!out.exists());
        assert!(leftovers(&lab.dir).is_empty());
    }
}

#[test]
fn gpl_encoders_are_refused_by_name_even_when_ffmpeg_has_them() {
    let tc = need!();
    let lab = setup("gpl", &tc);
    let out = lab.dir.join("o.mp4");
    let s = RenderSettings::new(64, 48);
    for name in ["libx264", "libx265", "libfoo"] {
        let opts = Mp4Options {
            encoder: Some(name.into()),
            ..Mp4Options::default()
        };
        let e = lab
            .p()
            .export_mp4(
                &lab.services,
                &"S".into(),
                TimeRange::new(Ticks(0), Ticks(4 * F)),
                &s,
                &out,
                &opts,
                &no_cancel,
            )
            .unwrap_err();
        let ProjectError::Asset(a) = &e else {
            panic!("{e:?}")
        };
        assert_eq!(a.code.to_string(), "MEDIA_ENCODER_PROHIBITED", "{name}");
        assert!(!out.exists());
    }
}

#[test]
fn mpeg4_reference_mp4_is_valid_with_synced_audio_and_exact_frame_count() {
    let tc = need!();
    let lab = setup("mp4ref", &tc);
    let out = lab.dir.join("ref.mp4");
    let s = RenderSettings::new(64, 48);
    let opts = Mp4Options {
        codec: ExportCodec::Mpeg4Reference,
        ..Mp4Options::default()
    };
    let r = lab
        .p()
        .export_mp4(
            &lab.services,
            &"S".into(),
            TimeRange::new(Ticks(0), Ticks(60 * F)),
            &s,
            &out,
            &opts,
            &no_cancel,
        )
        .unwrap();
    assert_eq!(r.codec, "mpeg4");
    assert_eq!(r.frames, 60);
    assert!(out.exists());
    assert!(leftovers(&lab.dir).is_empty());
    assert!(r.av_drift_ticks.unwrap() <= F);
    // validação independente
    let info = capia_media::MediaProbe::probe(&capia_media::FfprobeBackend::new(tc.clone()), &out)
        .unwrap();
    let v = info.video().unwrap();
    assert_eq!((v.width, v.height), (64, 48));
    assert_eq!(info.audio().unwrap().sample_rate, 48_000);
}
