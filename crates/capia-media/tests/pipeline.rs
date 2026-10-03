//! Índice + decode de quadros + áudio com o FFmpeg real sobre fixtures com quadros
//! **identificáveis** (luma = função do número do quadro): prova que o quadro certo foi entregue.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_media::*;
use capia_time::{Rational, TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "CAPIA_REQUIRE_FFMPEG is set but ffmpeg/ffprobe are unavailable: {other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
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

fn never() -> bool {
    false
}

struct Video {
    index: FrameIndex,
    w: u32,
    h: u32,
}

fn load(tc: &MediaToolchain, name: &str) -> Video {
    let path = fixture(name);
    let info = FfprobeBackend::new(tc.clone()).probe(&path).unwrap();
    let v = info.video().unwrap().clone();
    let index = build_frame_index(
        tc,
        &path,
        v.index,
        v.time_base.unwrap(),
        0,
        &IndexOptions::default(),
        &never,
        &mut |_, _| {},
    )
    .unwrap();
    Video {
        index,
        w: v.width,
        h: v.height,
    }
}

/// Luma média do quadro RGBA (cinza ⇒ R≈G≈B).
fn luma(f: &RawFrame) -> i32 {
    let n = (f.width * f.height) as usize;
    let sum: u64 = f
        .bytes
        .chunks_exact(4)
        .map(|p| (u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2])) / 3)
        .sum();
    (sum / n as u64) as i32
}

/// Luma esperada do quadro N (limited→full range do yuv420p→rgba): 16..235 → 0..255.
fn expected_gray(code: i32) -> i32 {
    ((code - 16) as f32 * 255.0 / 219.0).round() as i32
}

fn decode(tc: &MediaToolchain, name: &str, v: &Video, i: usize) -> RawFrame {
    decode_frame_by_index(
        tc,
        &fixture(name),
        &v.index,
        v.w,
        v.h,
        i,
        &DecodeLimits::default(),
        &never,
    )
    .unwrap()
}

#[test]
fn cfr_long_gop_with_b_frames_decodes_every_logical_frame_exactly() {
    let tc = need!();
    let v = load(&tc, "cfr_gop.mp4");
    assert_eq!(v.index.len(), 50);
    assert_eq!(v.index.keyframe_count(), 1, "long GOP: one keyframe");
    for i in 0..50usize {
        let f = decode(&tc, "cfr_gop.mp4", &v, i);
        assert_eq!((f.width, f.height, f.stride), (64, 48, 256));
        assert_eq!(f.bytes.len(), 64 * 48 * 4);
        assert_eq!(f.index, i);
        let want = expected_gray(20 + 4 * i as i32);
        assert!(
            (luma(&f) - want).abs() <= 3,
            "frame {i}: luma {} vs expected {want}",
            luma(&f)
        );
    }
}

#[test]
fn vfr_lookup_and_decode_follow_real_timestamps() {
    let tc = need!();
    let v = load(&tc, "vfr.mp4");
    assert_eq!(v.index.len(), 25);
    // o quadro N (apresentação) tem luma 20+8N; os buracos fazem tempo ≠ N/25
    let tb = v.index.time_base();
    assert_eq!(tb, Rational::new(1, 12_800).unwrap());
    let t3 = v.index.time_of(3).unwrap();
    let t4 = v.index.time_of(4).unwrap();
    assert_eq!(
        t4.0 - t3.0,
        3 * TICKS_PER_SECOND / 25,
        "gap of 3 frame periods"
    );
    // um instante no meio do buraco mostra o quadro anterior (3), não o 4
    let mid = Ticks((t3.0 + t4.0) / 2);
    assert_eq!(v.index.frame_at_or_before(mid), Some(3));
    let f = decode_frame_at(
        &tc,
        &fixture("vfr.mp4"),
        &v.index,
        v.w,
        v.h,
        mid,
        &DecodeLimits::default(),
        &never,
    )
    .unwrap();
    assert_eq!(f.index, 3);
    assert!((luma(&f) - expected_gray(20 + 8 * 3)).abs() <= 3);
    // `N/25` erraria: o quadro apresentado em N/25 s para N=5 é outro
    let naive = Ticks(5 * TICKS_PER_SECOND / 25);
    assert_ne!(v.index.frame_at_or_before(naive), Some(5));
    for i in [0usize, 5, 12, 23] {
        let f = decode(&tc, "vfr.mp4", &v, i);
        assert!(
            (luma(&f) - expected_gray(20 + 8 * i as i32)).abs() <= 3,
            "vfr frame {i}"
        );
    }
}

#[test]
fn decoding_is_deterministic_and_cancellable() {
    let tc = need!();
    let v = load(&tc, "cfr_gop.mp4");
    let a = decode(&tc, "cfr_gop.mp4", &v, 30);
    let b = decode(&tc, "cfr_gop.mp4", &v, 30);
    assert_eq!(a.bytes, b.bytes);
    let err = decode_frame_by_index(
        &tc,
        &fixture("cfr_gop.mp4"),
        &v.index,
        v.w,
        v.h,
        49,
        &DecodeLimits::default(),
        &|| true,
    )
    .unwrap_err();
    assert_eq!(err.code, MediaErrorCode::MediaCancelled);
    let err = decode_frame_by_index(
        &tc,
        &fixture("cfr_gop.mp4"),
        &v.index,
        v.w,
        v.h,
        500,
        &DecodeLimits::default(),
        &never,
    )
    .unwrap_err();
    assert_eq!(err.code, MediaErrorCode::MediaFrameNotFound);
    // dimensões absurdas: limite checado ANTES de qualquer processo
    let err = decode_frame_by_index(
        &tc,
        &fixture("cfr_gop.mp4"),
        &v.index,
        u32::MAX,
        u32::MAX,
        0,
        &DecodeLimits::default(),
        &never,
    )
    .unwrap_err();
    assert_eq!(err.code, MediaErrorCode::MediaLimitExceeded);
}

fn audio_req(rate: u32, ch: u32, start: Ticks, dur: Ticks) -> AudioRequest {
    AudioRequest {
        stream_index: 0,
        sample_rate: rate,
        channels: ch,
        start,
        duration: dur,
    }
}

fn pcm(tc: &MediaToolchain, name: &str, r: &AudioRequest) -> AudioPcm {
    decode_audio(
        tc,
        &fixture(name),
        r,
        DEFAULT_MAX_PCM_BYTES,
        std::time::Duration::from_secs(60),
        &never,
    )
    .unwrap()
}

#[test]
fn audio_intervals_are_sample_exact_at_44100_and_48000() {
    let tc = need!();
    for (name, rate) in [("tone_44k.wav", 44_100u32), ("audio.wav", 48_000)] {
        let ch = if name == "audio.wav" { 1 } else { 2 };
        let all = pcm(
            &tc,
            name,
            &audio_req(rate, ch, Ticks(0), Ticks(TICKS_PER_SECOND)),
        );
        assert_eq!(all.frames, u64::from(rate), "{name}: 1 s exactly");
        assert_eq!(all.samples.len(), (rate * ch) as usize);
        assert!(all.samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(all.samples.iter().any(|s| s.abs() > 0.05), "not silence");
        // um recorte [start, start+dur) é idêntico ao mesmo trecho do decode inteiro
        let start_s = 12_345u64;
        let dur_s = 777u64;
        let start = samples_to_ticks(start_s, rate);
        let dur = samples_to_ticks(dur_s, rate);
        let part = pcm(&tc, name, &audio_req(rate, ch, start, dur));
        assert_eq!(part.start_sample, start_s);
        assert_eq!(part.frames, dur_s);
        let from = (start_s * u64::from(ch)) as usize;
        let to = from + (dur_s * u64::from(ch)) as usize;
        assert_eq!(part.samples, all.samples[from..to], "{name}");
    }
}

#[test]
fn short_clips_and_ends_of_media_are_truncated_not_padded() {
    let tc = need!();
    let one = pcm(
        &tc,
        "tone_44k.wav",
        &audio_req(44_100, 2, Ticks(0), samples_to_ticks(1, 44_100)),
    );
    assert_eq!(one.frames, 1);
    // pede além do fim: devolve só o que existe
    let tail = pcm(
        &tc,
        "tone_44k.wav",
        &audio_req(
            44_100,
            2,
            samples_to_ticks(44_000, 44_100),
            Ticks(TICKS_PER_SECOND),
        ),
    );
    assert_eq!(tail.frames, 100);
    // início depois do fim: vazio
    let none = pcm(
        &tc,
        "tone_44k.wav",
        &audio_req(
            44_100,
            2,
            Ticks(5 * TICKS_PER_SECOND),
            Ticks(TICKS_PER_SECOND),
        ),
    );
    assert_eq!(none.frames, 0);
    assert!(none.samples.is_empty());
}

#[test]
fn audio_resamples_and_remixes_to_the_requested_shape() {
    let tc = need!();
    let a = pcm(
        &tc,
        "tone_44k.wav",
        &audio_req(48_000, 1, Ticks(0), Ticks(TICKS_PER_SECOND)),
    );
    assert_eq!(a.channels, 1);
    assert!((a.frames as i64 - 48_000).abs() <= 2, "frames {}", a.frames);
    let bad = decode_audio(
        &tc,
        &fixture("tone_44k.wav"),
        &audio_req(0, 2, Ticks(0), Ticks(1)),
        1 << 20,
        std::time::Duration::from_secs(5),
        &never,
    )
    .unwrap_err();
    assert_eq!(bad.code, MediaErrorCode::MediaMetadataInvalid);
    let big = decode_audio(
        &tc,
        &fixture("tone_44k.wav"),
        &audio_req(48_000, 2, Ticks(0), Ticks(TICKS_PER_SECOND)),
        1000,
        std::time::Duration::from_secs(5),
        &never,
    )
    .unwrap_err();
    assert_eq!(big.code, MediaErrorCode::MediaLimitExceeded);
}

#[test]
fn block_decode_streams_the_whole_stream_and_can_be_cancelled() {
    let tc = need!();
    let mut seen = 0usize;
    let total = decode_audio_blocks(
        &tc,
        &fixture("tone_44k.wav"),
        0,
        44_100,
        2,
        std::time::Duration::from_secs(30),
        &never,
        &mut |b| {
            assert_eq!(b.len() % 2, 0);
            seen += b.len();
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(total, 44_100);
    assert_eq!(seen, 88_200);
    let err = decode_audio_blocks(
        &tc,
        &fixture("tone_44k.wav"),
        0,
        44_100,
        2,
        std::time::Duration::from_secs(30),
        &|| true,
        &mut |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(err.code, MediaErrorCode::MediaCancelled);
}
