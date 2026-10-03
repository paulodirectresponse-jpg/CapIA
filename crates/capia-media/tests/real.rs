//! Probe com o **ffprobe real** sobre as fixtures versionadas (tests/fixtures/media). No CI
//! (`CAPIA_REQUIRE_FFMPEG=1`) a ausência do binário FALHA; localmente o teste é ignorado com aviso.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_media::*;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) => Some(t),
        Err(e) => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "CAPIA_REQUIRE_FFMPEG is set but ffprobe is unavailable: {e}"
            );
            eprintln!("SKIP (no ffprobe): {e}");
            None
        }
    }
}

macro_rules! need {
    () => {
        match toolchain() {
            Some(t) => t,
            None => return,
        }
    };
}

fn probe(t: &MediaToolchain, name: &str) -> Result<MediaInfo, MediaError> {
    FfprobeBackend::new(t.clone()).probe(&fixture(name))
}

/// Remove campos que variam entre versões/encoders do FFmpeg (taxa de bits) antes de comparar.
fn stable(info: &MediaInfo) -> serde_json::Value {
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                m.remove("bit_rate");
                m.values_mut().for_each(strip);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(info).unwrap();
    strip(&mut v);
    v
}

fn golden(name: &str, info: &MediaInfo) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.json"));
    let got = serde_json::to_string_pretty(&stable(info)).unwrap() + "\n";
    if std::env::var_os("CAPIA_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got, want,
        "golden mismatch for {name} (CAPIA_BLESS=1 to bless)"
    );
}

#[test]
fn video_with_audio() {
    let t = need!();
    let m = probe(&t, "video_audio.mp4").unwrap();
    assert_eq!(m.kind, MediaKind::Video);
    let v = m.video().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (64, 48, "h264"));
    assert_eq!(v.frame_rate, capia_time::Rational::new(25, 1).ok());
    assert_eq!(v.rotation, 0);
    let a = m.audio().unwrap();
    assert_eq!(
        (a.channels, a.sample_rate, a.codec.as_str()),
        (2, 48000, "aac")
    );
    assert_eq!(m.duration, Some(Ticks(TICKS_PER_SECOND)));
    golden("video_audio", &m);
}

#[test]
fn video_without_audio() {
    let t = need!();
    let m = probe(&t, "video_only.mp4").unwrap();
    assert_eq!(m.kind, MediaKind::Video);
    assert!(m.has_video() && !m.has_audio());
    golden("video_only", &m);
}

#[test]
fn audio_only() {
    let t = need!();
    let m = probe(&t, "audio.wav").unwrap();
    assert_eq!(m.kind, MediaKind::Audio);
    assert!(!m.has_video() && m.has_audio());
    assert_eq!(m.duration, Some(Ticks(TICKS_PER_SECOND)));
    golden("audio_wav", &m);
}

#[test]
fn images() {
    let t = need!();
    let png = probe(&t, "image_alpha.png").unwrap();
    assert_eq!(png.kind, MediaKind::Image);
    assert_eq!(png.duration, None);
    let v = png.video().unwrap();
    assert_eq!((v.width, v.height, v.has_alpha), (32, 24, Some(true)));
    golden("image_alpha_png", &png);
    let jpg = probe(&t, "image.jpg").unwrap();
    assert_eq!(jpg.kind, MediaKind::Image);
    assert_eq!(jpg.video().unwrap().has_alpha, Some(false));
    golden("image_jpg", &jpg);
}

#[test]
fn invalid_file_is_a_structured_error() {
    let t = need!();
    let e = probe(&t, "invalid.mp4").unwrap_err();
    assert_eq!(e.code, MediaErrorCode::MediaProbeFailed);
}

#[test]
fn missing_and_special_paths_are_structured_errors() {
    let t = need!();
    let b = FfprobeBackend::new(t);
    for p in [
        Path::new("/definitely/not/here.mp4"),
        Path::new(""),
        Path::new("."),
    ] {
        let e = b.probe(p).unwrap_err();
        assert!(
            matches!(
                e.code,
                MediaErrorCode::MediaIo | MediaErrorCode::MediaInvalidPath
            ),
            "{p:?}: {e}"
        );
    }
    let long = "x".repeat(MAX_PATH_BYTES + 1);
    assert_eq!(
        b.probe(Path::new(&long)).unwrap_err().code,
        MediaErrorCode::MediaInvalidPath
    );
}

#[test]
fn probing_is_deterministic_across_runs() {
    let t = need!();
    let a = stable(&probe(&t, "video_audio.mp4").unwrap());
    for _ in 0..5 {
        assert_eq!(stable(&probe(&t, "video_audio.mp4").unwrap()), a);
    }
}

// nomes com `:` e `|` não existem no Windows
#[cfg(unix)]
#[test]
fn protocol_looking_names_are_treated_as_files_not_urls() {
    // um arquivo cujo NOME parece um protocolo não pode ser interpretado como tal
    let t = need!();
    let dir = std::env::temp_dir().join(format!("capia-media-proto-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let evil = dir.join("concat:video_audio.mp4|video_audio.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &evil).unwrap();
    let m = FfprobeBackend::new(t).probe(&evil).unwrap();
    // duração de UMA cópia, não da concatenação
    assert_eq!(m.duration, Some(Ticks(TICKS_PER_SECOND)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thumbnail_is_a_png_and_respects_the_size_limit() {
    let t = need!();
    if t.ffmpeg.is_none() {
        assert!(
            std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
            "ffmpeg required"
        );
        return;
    }
    let png = extract_frame_png(
        &t,
        &fixture("video_audio.mp4"),
        ThumbnailRequest {
            at: Ticks(TICKS_PER_SECOND / 2),
            max_dim: 32,
        },
    )
    .unwrap();
    assert_eq!(&png[1..4], b"PNG");
    // 64x48 reduzido para caber em 32 ⇒ 32x24 (largura/altura no IHDR)
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!((w, h), (32, 24));
    let e = extract_frame_png(
        &t,
        &fixture("video_audio.mp4"),
        ThumbnailRequest {
            at: Ticks(-1),
            max_dim: 32,
        },
    )
    .unwrap_err();
    assert_eq!(e.code, MediaErrorCode::MediaMetadataInvalid);
}

#[cfg(unix)]
mod fake_backend {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    fn script(dir: &Path, body: &str) -> MediaToolchain {
        let p = dir.join("ffprobe");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        MediaToolchain {
            ffprobe: p,
            ffprobe_source: ToolSource::Configured,
            ffmpeg: None,
            version: "fake".into(),
            probe_timeout: Duration::from_millis(400),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("capia-fake-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_probe_that_hangs_is_killed_with_a_timeout_error() {
        let d = tmp("hang");
        let b = FfprobeBackend::new(script(&d, "exec sleep 30"));
        let started = std::time::Instant::now();
        let e = b.probe(&fixture("video_audio.mp4")).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeTimeout);
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_probe_that_floods_stdout_is_cut_off() {
        let d = tmp("flood");
        let mut t = script(&d, "exec yes '{\"streams\":[]}'");
        t.probe_timeout = Duration::from_secs(60);
        let e = FfprobeBackend::new(t)
            .probe(&fixture("video_audio.mp4"))
            .unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeOutputTooLarge);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn garbage_json_from_the_backend_is_a_structured_error() {
        let d = tmp("garbage");
        for body in [
            "echo '{not json'",
            "printf ''",
            "echo '{\"format\":{},\"streams\":[]}'",
            "exit 7",
        ] {
            let e = FfprobeBackend::new(script(&d, body))
                .probe(&fixture("video_audio.mp4"))
                .unwrap_err();
            assert!(
                matches!(
                    e.code,
                    MediaErrorCode::MediaMetadataInvalid
                        | MediaErrorCode::MediaUnsupportedFormat
                        | MediaErrorCode::MediaProbeFailed
                ),
                "{body}: {e}"
            );
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_backend_is_media_backend_not_found_not_a_panic() {
        let t = MediaToolchain {
            ffprobe: PathBuf::from("/no/such/ffprobe"),
            ffprobe_source: ToolSource::Configured,
            ffmpeg: None,
            version: String::new(),
            probe_timeout: Duration::from_secs(1),
        };
        let e = FfprobeBackend::new(t)
            .probe(&fixture("video_audio.mp4"))
            .unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaBackendNotFound);
    }
}
