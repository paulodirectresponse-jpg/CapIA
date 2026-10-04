//! Medidas do serviço de decode e do PCM por blocos (só IMPRIME; nada de metas inventadas).
//! `cargo test --release -p capia-decode --test perf -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_decode::*;
use capia_media::*;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn toolchain() -> Option<MediaToolchain> {
    MediaToolchain::locate(&MediaConfig::default())
        .ok()
        .filter(|t| t.ffmpeg.is_some())
}

fn ff(tc: &MediaToolchain, args: &[&str]) {
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .unwrap();
    assert!(st.success());
}

fn video_source(tc: &MediaToolchain, path: &Path, content: &str) -> Arc<VideoSource> {
    let info = FfprobeBackend::new(tc.clone()).probe(path).unwrap();
    let v = info.video().unwrap().clone();
    let index = build_frame_index(
        tc,
        path,
        v.index,
        v.time_base.unwrap(),
        0,
        &IndexOptions::default(),
        &|| false,
        &mut |_, _| {},
    )
    .unwrap();
    Arc::new(VideoSource {
        namespace: 1,
        content: Arc::from(content),
        path: path.to_path_buf(),
        index: Arc::new(index),
        width: v.width,
        height: v.height,
    })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[test]
#[ignore = "medição: rode com --release --ignored --nocapture"]
fn decode_service_timings() {
    let tc = toolchain().expect("ffmpeg required");
    let d = std::env::temp_dir().join(format!("capia-decode-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let hd: PathBuf = d.join("hd.mp4");
    ff(
        &tc,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30",
            "-t",
            "10",
            "-c:v",
            "mpeg4",
            "-g",
            "30",
            "-qscale:v",
            "4",
            hd.to_str().unwrap(),
        ],
    );
    let s = video_source(&tc, &hd, "sha256:hd");
    let frame_mb = (1920 * 1080 * 4) as f64 / 1_048_576.0;
    eprintln!(
        "PERF decode source: 1920x1080 mpeg4 GOP30, {} frames (frame = {frame_mb:.1} MiB)",
        s.index.len()
    );

    // frio: primeira leitura (abre sessão + keyframe) e quente: o mesmo quadro de novo (cache)
    let sv = DecodeService::new(DecodeConfig::new(tc.clone()));
    let t0 = Instant::now();
    sv.get_frame(&s, 45, Priority::Interactive).unwrap();
    let cold = t0.elapsed();
    let t0 = Instant::now();
    sv.get_frame(&s, 45, Priority::Interactive).unwrap();
    let warm = t0.elapsed();
    eprintln!(
        "PERF decode cold first frame (session open + GOP): {:.1} ms; warm (cache hit): {:.3} ms",
        ms(cold),
        ms(warm)
    );

    // sequencial: 120 quadros em ordem (uma sessão)
    let sv = DecodeService::new(DecodeConfig::new(tc.clone()));
    let t0 = Instant::now();
    for i in 0..120 {
        sv.get_frame(&s, i, Priority::Playback).unwrap();
    }
    let el = t0.elapsed();
    let m = sv.metrics();
    eprintln!(
        "PERF decode sequential 120 frames: {:.1} ms total, {:.2} ms/frame, {:.1} fps; sessions opened {}, reused {}, reuse rate {:.2}",
        ms(el),
        ms(el) / 120.0,
        120.0 / el.as_secs_f64(),
        m.sessions_opened,
        m.sessions_reused,
        m.reuse_rate()
    );

    // seek aleatório com cache minúsculo (cada pedido é um seek de verdade)
    let mut cfg = DecodeConfig::new(tc.clone());
    cfg.frame_cache_bytes = 1;
    cfg.max_sessions = 1;
    let sv = DecodeService::new(cfg);
    let mut x = 12345u64;
    let t0 = Instant::now();
    let n = 24;
    for _ in 0..n {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let i = ((x >> 33) % s.index.len() as u64) as usize;
        sv.get_frame(&s, i, Priority::Interactive).unwrap();
    }
    let el = t0.elapsed();
    let m = sv.metrics();
    eprintln!(
        "PERF decode random seek ({n} requests, no cache): {:.1} ms/seek; sessions opened {}, reused {}",
        ms(el) / f64::from(n),
        m.sessions_opened,
        m.sessions_reused
    );

    // VFR (fixture versionada): sequencial por tempo
    let vfr = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/media/vfr.mp4");
    let sv_vfr = video_source(&tc, &vfr, "sha256:vfr");
    let sv = DecodeService::new(DecodeConfig::new(tc.clone()));
    let t0 = Instant::now();
    for i in 0..sv_vfr.index.len() {
        sv.get_frame(&sv_vfr, i, Priority::Playback).unwrap();
    }
    eprintln!(
        "PERF decode VFR fixture ({} frames, 64x48): {:.1} ms total",
        sv_vfr.index.len(),
        ms(t0.elapsed())
    );

    // áudio: 1 s em t = 0/60/300 s e blocos sequenciais (PCM s16 mono 44,1 kHz de 330 s)
    let wav = d.join("long.wav");
    ff(
        &tc,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=sin(2*PI*t*(300+40*t)):s=44100:d=330:c=mono",
            "-c:a",
            "pcm_s16le",
            wav.to_str().unwrap(),
        ],
    );
    let info = FfprobeBackend::new(tc.clone()).probe(&wav).unwrap();
    let a = info.audio().unwrap().clone();
    let ix = build_audio_index(
        &tc,
        &wav,
        a.index,
        a.sample_rate,
        a.channels,
        a.time_base.unwrap(),
        true,
        Duration::from_secs(120),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let src = AudioSource {
        namespace: 1,
        content: Arc::from("sha256:wav"),
        path: wav.clone(),
        index: Arc::new(ix),
    };
    let rate = u64::from(a.sample_rate);
    for t in [0u64, 60, 300] {
        let cache = PcmCache::new(tc.clone(), 64 << 20);
        let t0 = Instant::now();
        let p = cache.read(&src, t * rate, rate, &|| false).unwrap();
        assert_eq!(p.frames, rate);
        eprintln!(
            "PERF audio 1 s at t={t:>3} s (cold): {:.1} ms; decoded {} frames",
            ms(t0.elapsed()),
            cache.metrics().decoded_frames
        );
    }
    let cache = PcmCache::new(tc.clone(), 64 << 20);
    let t0 = Instant::now();
    for k in 0..30u64 {
        cache
            .read(&src, 100 * rate + k * rate, rate, &|| false)
            .unwrap();
    }
    let m = cache.metrics();
    eprintln!(
        "PERF audio sequential 30 x 1 s blocks: {:.1} ms/block; decode calls {}, cache hits {}, cache bytes {}",
        ms(t0.elapsed()) / 30.0,
        m.decode_calls,
        m.cache.hits,
        m.cache.bytes
    );
    let t0 = Instant::now();
    cache.read(&src, 100 * rate, rate, &|| false).unwrap();
    eprintln!("PERF audio warm 1 s (cache): {:.3} ms", ms(t0.elapsed()));
    let _ = (Ticks(0), TICKS_PER_SECOND);
    let _ = std::fs::remove_dir_all(d);
}
