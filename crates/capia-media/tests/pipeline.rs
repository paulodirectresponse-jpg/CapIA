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

// ---- waveform ------------------------------------------------------------------------------

#[test]
fn waveform_from_a_real_file_matches_the_decoded_pcm() {
    let tc = need!();
    let w = generate_waveform(
        &tc,
        &fixture("tone_44k.wav"),
        0,
        44_100,
        std::time::Duration::from_secs(30),
        &never,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(w.total_samples(), 44_100);
    assert_eq!(w.duration(), Ticks(TICKS_PER_SECOND));
    // pico global = pico do PCM decodificado
    let all = pcm(
        &tc,
        "tone_44k.wav",
        &audio_req(44_100, 1, Ticks(0), Ticks(TICKS_PER_SECOND)),
    );
    let max = all.samples.iter().cloned().fold(f32::MIN, f32::max);
    let top = w.level(w.level_count() - 1).unwrap();
    assert_eq!(top.len(), 1);
    assert!((top[0].max - max).abs() < 1e-6);
    let bytes = w.encode();
    assert_eq!(Waveform::decode(&bytes).unwrap(), w);
    let err = generate_waveform(
        &tc,
        &fixture("tone_44k.wav"),
        0,
        44_100,
        std::time::Duration::from_secs(30),
        &|| true,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err.code, MediaErrorCode::MediaCancelled);
    // stream inexistente ⇒ erro estruturado, nunca waveform parcial
    assert!(
        generate_waveform(
            &tc,
            &fixture("tone_44k.wav"),
            9,
            44_100,
            std::time::Duration::from_secs(30),
            &never,
            &mut |_| {}
        )
        .is_err()
    );
}

// ---- proxy ---------------------------------------------------------------------------------

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("capia-media-proxy-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(windows)]
    {
        let o = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).contains(&pid.to_string())
    }
}

#[test]
fn proxy_keeps_every_frame_and_the_vfr_timestamps() {
    let tc = need!();
    let out = tmp("vfr_proxy.mov");
    let mut last = 0u64;
    let rep = generate_proxy(
        &tc,
        &fixture("vfr.mp4"),
        &out,
        &ProxyProfileV1 {
            max_width: 32,
            max_height: 32,
            audio: ProxyAudio::None,
            ..Default::default()
        },
        false,
        1_000_000,
        std::time::Duration::from_secs(60),
        None,
        &never,
        &mut |d, _| last = d,
    )
    .unwrap();
    assert_eq!(rep.encoder.name, "mjpeg");
    let v = rep.info.video().unwrap();
    assert!(
        v.width <= 32 && v.height <= 32 && v.width.is_multiple_of(2) && v.height.is_multiple_of(2)
    );
    // mesma contagem de quadros e mesmos *instantes* (em ticks) que o original
    let orig = load(&tc, "vfr.mp4");
    let pinfo = FfprobeBackend::new(tc.clone()).probe(&out).unwrap();
    let pv = pinfo.video().unwrap();
    let pidx = build_frame_index(
        &tc,
        &out,
        pv.index,
        pv.time_base.unwrap(),
        0,
        &IndexOptions::default(),
        &never,
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(pidx.len(), orig.index.len());
    for i in 0..orig.index.len() {
        let (a, b) = (orig.index.time_of(i).unwrap().0, pidx.time_of(i).unwrap().0);
        assert!(
            (a - b).abs() <= TICKS_PER_SECOND / 10_000,
            "frame {i}: {a} vs {b}"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn proxy_with_hardware_h264_reports_unavailable_when_missing() {
    let tc = need!();
    let enc = list_encoders(&tc).unwrap();
    let hw_present = HARDWARE_H264_ENCODERS
        .iter()
        .any(|h| enc.iter().any(|e| e == h));
    let r = select_encoder(ProxyCodec::H264Hardware, &enc);
    assert_eq!(r.is_ok(), hw_present);
    if let Err(e) = r {
        assert_eq!(e.code, MediaErrorCode::MediaEncoderUnavailable);
    }
}

#[test]
fn cancelling_a_proxy_kills_ffmpeg_and_leaves_no_file() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    let tc = need!();
    // entrada longa o bastante para o encode não terminar antes do cancelamento
    let long = tmp("long_src.mkv");
    let ff = tc.ffmpeg.clone().unwrap();
    let st = std::process::Command::new(&ff)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=240",
        ])
        .args(["-c:v", "mpeg4", "-q:v", "10"])
        .arg(&long)
        .status()
        .unwrap();
    assert!(st.success());
    let out = tmp("cancel_proxy.mov");
    let pid = Arc::new(AtomicU32::new(0));
    let cancelled = AtomicBool::new(false);
    let err = generate_proxy(
        &tc,
        &long,
        &out,
        &ProxyProfileV1 {
            max_width: 640,
            max_height: 360,
            audio: ProxyAudio::None,
            jpeg_quality: 2,
            ..Default::default()
        },
        false,
        240_000_000,
        std::time::Duration::from_secs(120),
        Some(pid.clone()),
        &|| cancelled.load(Ordering::SeqCst),
        &mut |_, _| cancelled.store(true, Ordering::SeqCst),
    )
    .unwrap_err();
    assert_eq!(err.code, MediaErrorCode::MediaCancelled);
    let p = pid.load(Ordering::SeqCst);
    assert_ne!(p, 0, "the pid was recorded");
    assert!(
        !pid_alive(p),
        "ffmpeg process {p} must be gone after cancel"
    );
    assert!(!out.exists(), "the partial proxy must be removed");
    let _ = std::fs::remove_file(&long);
}

// ---- runner de streaming -------------------------------------------------------------------

fn endless_source_args() -> Vec<std::ffi::OsString> {
    [
        "-v",
        "error",
        "-re",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x64:rate=30",
        "-f",
        "rawvideo",
        "pipe:1",
    ]
    .iter()
    .map(std::ffi::OsString::from)
    .collect()
}

#[test]
fn streaming_runner_stops_early_and_kills_the_child() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    let tc = need!();
    let pid = Arc::new(AtomicU32::new(0));
    let mut lim = StreamLimits::new(std::time::Duration::from_secs(30));
    lim.pid_sink = Some(pid.clone());
    let mut got = 0usize;
    let out = run_streaming(
        tc.ffmpeg.as_ref().unwrap(),
        &endless_source_args(),
        &lim,
        &never,
        &mut |c| {
            got += c.len();
            Ok(if got > 50_000 {
                Flow::Stop
            } else {
                Flow::Continue
            })
        },
    )
    .unwrap();
    assert!(out.stopped_early);
    assert!(!pid_alive(pid.load(Ordering::SeqCst)));
}

#[test]
fn streaming_runner_kills_on_cancel_and_on_timeout() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    let tc = need!();
    for (timeout_ms, cancel_now, code) in [
        (30_000u64, true, MediaErrorCode::MediaCancelled),
        (300, false, MediaErrorCode::MediaProbeTimeout),
    ] {
        let pid = Arc::new(AtomicU32::new(0));
        let mut lim = StreamLimits::new(std::time::Duration::from_millis(timeout_ms));
        lim.pid_sink = Some(pid.clone());
        let started = std::time::Instant::now();
        let err = run_streaming(
            tc.ffmpeg.as_ref().unwrap(),
            &endless_source_args(),
            &lim,
            &|| cancel_now && started.elapsed() > std::time::Duration::from_millis(200),
            &mut |_| Ok(Flow::Continue),
        )
        .unwrap_err();
        assert_eq!(err.code, code);
        assert!(!pid_alive(pid.load(Ordering::SeqCst)), "{code}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}

// ---- timestamps com deslocamento, outro container e stream de áudio não-padrão ---------------------------

/// mkv sem perdas (ffv1: o luma sai EXATO) com os timestamps deslocados 3 s: o índice usa os PTS
/// reais do container e o decode ainda acha o quadro certo.
#[test]
fn a_container_with_a_nonzero_start_offset_still_decodes_the_exact_frame() {
    let tc = need!();
    let dir = std::env::temp_dir().join(format!("capia-offset-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("offset.mkv");
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("nullsrc=size=64x48:rate=25:duration=1,format=yuv420p,geq=lum='20+7*N':cb=128:cr=128")
        .args([
            "-c:v",
            "ffv1",
            "-g",
            "5",
            "-fps_mode",
            "passthrough",
            "-output_ts_offset",
            "3",
        ])
        .arg(&file)
        .status()
        .unwrap();
    assert!(st.success());
    let info = FfprobeBackend::new(tc.clone()).probe(&file).unwrap();
    let v = info.video().unwrap().clone();
    let index = build_frame_index(
        &tc,
        &file,
        v.index,
        v.time_base.unwrap(),
        0,
        &IndexOptions::default(),
        &never,
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(index.len(), 25);
    assert!(
        index.start_pts() != 0,
        "the stream really starts late: {}",
        index.start_pts()
    );
    assert_eq!(
        index.time_of(0),
        Some(Ticks(0)),
        "times are relative to the first frame"
    );
    for i in [0usize, 7, 13, 24] {
        let f = decode_frame_by_index(
            &tc,
            &file,
            &index,
            v.width,
            v.height,
            i,
            &DecodeLimits::default(),
            &never,
        )
        .unwrap();
        let want = expected_gray(20 + 7 * i as i32);
        assert!(
            (luma(&f) - want).abs() <= 1,
            "frame {i}: {} vs {want}",
            luma(&f)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_non_default_audio_stream_is_decoded_by_its_absolute_index() {
    let tc = need!();
    let path = fixture("multi_stream.mp4");
    let info = FfprobeBackend::new(tc.clone()).probe(&path).unwrap();
    let audios: Vec<_> = info
        .streams
        .iter()
        .filter_map(|s| match s {
            StreamInfo::Audio(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(audios.len(), 2);
    for a in &audios {
        let pcm = decode_audio(
            &tc,
            &path,
            &AudioRequest {
                stream_index: a.index,
                sample_rate: a.sample_rate,
                channels: 1,
                start: Ticks(0),
                duration: Ticks(TICKS_PER_SECOND / 4),
            },
            DEFAULT_MAX_PCM_BYTES,
            std::time::Duration::from_secs(30),
            &never,
        )
        .unwrap();
        assert_eq!(
            pcm.frames,
            u64::from(a.sample_rate / 4),
            "stream #{}",
            a.index
        );
        assert!(pcm.samples.iter().any(|s| s.abs() > 0.02));
    }
}

// ---- decode de intervalo (um processo para N quadros) --------------------------------------------------

#[test]
fn a_frame_range_equals_the_individual_frames_in_a_single_process() {
    let tc = need!();
    let v = load(&tc, "cfr_gop.mp4");
    let path = fixture("cfr_gop.mp4");
    for (first, count) in [(10usize, 12usize), (0, 5), (45, 5), (20, 1)] {
        let mut got = Vec::new();
        let n = decode_frame_range(
            &tc,
            &path,
            &v.index,
            v.w,
            v.h,
            first,
            count,
            &DecodeLimits::default(),
            &never,
            &mut |f| {
                got.push(f);
                Flow::Continue
            },
        )
        .unwrap();
        assert_eq!(n, count);
        for (k, f) in got.iter().enumerate() {
            assert_eq!(f.index, first + k);
            assert_eq!(
                f.bytes,
                decode(&tc, "cfr_gop.mp4", &v, first + k).bytes,
                "frame {}",
                first + k
            );
        }
    }
    // VFR: o intervalo segue os PTS reais (e não N/25)
    let vv = load(&tc, "vfr.mp4");
    let mut luma_seq = Vec::new();
    decode_frame_range(
        &tc,
        &fixture("vfr.mp4"),
        &vv.index,
        vv.w,
        vv.h,
        2,
        8,
        &DecodeLimits::default(),
        &never,
        &mut |f| {
            luma_seq.push(luma(&f));
            Flow::Continue
        },
    )
    .unwrap();
    for (k, l) in luma_seq.iter().enumerate() {
        assert!(
            (l - expected_gray(20 + 8 * (2 + k as i32))).abs() <= 3,
            "vfr range frame {k}"
        );
    }
    // Stop encerra cedo; limites e cancelamento
    let mut seen = 0;
    let n = decode_frame_range(
        &tc,
        &path,
        &v.index,
        v.w,
        v.h,
        0,
        30,
        &DecodeLimits::default(),
        &never,
        &mut |_| {
            seen += 1;
            if seen == 3 {
                Flow::Stop
            } else {
                Flow::Continue
            }
        },
    )
    .unwrap();
    assert_eq!((n, seen), (3, 3));
    for (first, count) in [(0usize, 0usize), (0, 513), (45, 10)] {
        let e = decode_frame_range(
            &tc,
            &path,
            &v.index,
            v.w,
            v.h,
            first,
            count,
            &DecodeLimits::default(),
            &never,
            &mut |_| Flow::Continue,
        )
        .unwrap_err();
        assert!(
            matches!(
                e.code,
                MediaErrorCode::MediaLimitExceeded | MediaErrorCode::MediaFrameNotFound
            ),
            "{first}+{count}"
        );
    }
    let e = decode_frame_range(
        &tc,
        &path,
        &v.index,
        v.w,
        v.h,
        0,
        10,
        &DecodeLimits::default(),
        &|| true,
        &mut |_| Flow::Continue,
    )
    .unwrap_err();
    assert_eq!(e.code, MediaErrorCode::MediaCancelled);
}

// ---- um decode que falha no meio nunca vira resultado -------------------------------------------------

/// ffmpeg falso: escreve PCM válido e **sai com erro** (disco/arquivo corrompido no meio).
#[cfg(unix)]
fn failing_ffmpeg(real: &MediaToolchain) -> (MediaToolchain, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("capia-fake-ffmpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("ffmpeg");
    std::fs::write(&script, "#!/bin/sh\nhead -c 400000 /dev/zero\nexit 3\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut tc = real.clone();
    tc.ffmpeg = Some(script);
    (tc, dir)
}

#[cfg(unix)]
#[test]
fn a_decode_that_dies_midway_is_an_error_never_a_partial_waveform_or_pcm() {
    let real = need!();
    let (tc, dir) = failing_ffmpeg(&real);
    let e = generate_waveform(
        &tc,
        &fixture("tone_44k.wav"),
        0,
        44_100,
        std::time::Duration::from_secs(30),
        &never,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        e.code,
        MediaErrorCode::MediaDecodeFailed,
        "partial PCM must not become a waveform"
    );
    let e = decode_audio(
        &tc,
        &fixture("tone_44k.wav"),
        &audio_req(44_100, 1, Ticks(0), Ticks(TICKS_PER_SECOND * 5)),
        DEFAULT_MAX_PCM_BYTES,
        std::time::Duration::from_secs(30),
        &never,
    )
    .unwrap_err();
    assert_eq!(e.code, MediaErrorCode::MediaDecodeFailed);
    let _ = std::fs::remove_dir_all(dir);
}
