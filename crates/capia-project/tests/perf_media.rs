//! Medições do pipeline de mídia (ignoradas por padrão; FFmpeg real):
//! `cargo test --release -p capia-project --test perf_media -- --ignored --nocapture`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::{hash_file, testing::StaticProbe};
use capia_commands::Actor;
use capia_jobs::{JobState, Priority};
use capia_media::{FfprobeBackend, MediaConfig, MediaToolchain, ProxyAudio, ProxyProfileV1};
use capia_project::{PipelineOptions, Project, PumpEvent};
use capia_store::{StoreOptions, Synchronous};
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn tc() -> MediaToolchain {
    MediaToolchain::locate(&MediaConfig::default()).expect("ffmpeg/ffprobe")
}

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        ..StoreOptions::default()
    }
}

fn user() -> Actor {
    Actor::user("perf")
}

fn never() -> bool {
    false
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("capia-perfm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

#[test]
#[ignore = "medição"]
fn import_returns_before_the_full_hash() {
    let d = dir("import");
    let mut p = Project::create(&d.join("p.capia"), &opts()).unwrap();
    p.start_pipeline(PipelineOptions::new(tc())).unwrap();
    let big = d.join("big.bin");
    {
        let mut f = std::fs::File::create(&big).unwrap();
        let block: Vec<u8> = (0..1024 * 1024).map(|i| (i % 253) as u8).collect();
        for _ in 0..256 {
            f.write_all(&block).unwrap();
        }
    }
    let mut returns = Vec::new();
    let mut tickets = Vec::new();
    for _ in 0..5 {
        // arquivos distintos para não deduplicar o job: só o 1º tem hash cheio concorrente
        let t0 = Instant::now();
        let t = p.import_asset_async(&big).unwrap();
        returns.push(t0.elapsed());
        tickets.push(t);
    }
    returns.sort();
    let median = returns[returns.len() / 2];
    for t in &tickets {
        p.cancel_ticket(&t.ticket_id).unwrap();
    }
    let t0 = Instant::now();
    while !tickets.iter().all(|t| {
        p.ticket(&t.ticket_id)
            .unwrap()
            .is_some_and(|r| r.state != capia_project::TicketState::Pending)
    }) {
        let _ = p.pump(&user());
        assert!(t0.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(5));
    }
    let t = Instant::now();
    let _ = hash_file(&big).unwrap();
    let full = t.elapsed();
    println!(
        "PERF import.return_latency_256MiB: median {median:?} (max {:?}) vs full SHA-256 {full:?} → {:.0}× faster",
        returns.last().unwrap(),
        full.as_secs_f64() / median.as_secs_f64()
    );
    // import completo (hash + probe) de um arquivo pequeno real, síncrono × assíncrono
    let f = d.join("va.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &f).unwrap();
    let pr = FfprobeBackend::new(tc());
    let t = Instant::now();
    p.import_asset(&user(), &f, &pr).unwrap();
    println!("PERF import.sync_small_real_file: {:?}", t.elapsed());
    let f2 = d.join("cfr.mp4");
    std::fs::copy(fixture("cfr_gop.mp4"), &f2).unwrap();
    let t = Instant::now();
    let tk = p.import_asset_async(&f2).unwrap();
    let ret = t.elapsed();
    loop {
        if p.pump(&user())
            .unwrap()
            .iter()
            .any(|e| matches!(e, PumpEvent::ImportFinalized { .. }))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    println!(
        "PERF import.async_small_real_file: return {ret:?}, finalized after {:?} ({})",
        t.elapsed(),
        tk.ticket_id
    );
    let _ = StaticProbe;
    let _ = std::fs::remove_dir_all(d);
}

fn make_long(tc: &MediaToolchain, path: &Path, seconds: u32) {
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=640x360:rate=30:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency=330:sample_rate=48000:duration={seconds}"
        ))
        .args([
            "-c:v",
            "mpeg4",
            "-q:v",
            "8",
            "-g",
            "60",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-shortest",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(st.success());
}

#[test]
#[ignore = "medição"]
fn index_decode_audio_waveform_proxy_and_cache() {
    let tc = tc();
    let d = dir("derive");
    let mut p = Project::create(&d.join("p.capia"), &opts()).unwrap();
    p.start_pipeline(PipelineOptions::new(tc.clone())).unwrap();
    let pr = FfprobeBackend::new(tc.clone());
    // fixture pequena
    let small = d.join("cfr.mp4");
    std::fs::copy(fixture("cfr_gop.mp4"), &small).unwrap();
    let small_id = p.import_asset(&user(), &small, &pr).unwrap().asset_id;
    let t = Instant::now();
    p.submit_frame_index(&small_id, Priority::Normal)
        .unwrap()
        .handle
        .wait();
    println!("PERF index.fixture_50_frames: {:?}", t.elapsed());
    // vídeo longo sintético: 10 min · 30 fps = 18.000 quadros
    let long = d.join("long.mp4");
    let t = Instant::now();
    make_long(&tc, &long, 600);
    println!(
        "PERF (setup) synthetic 10 min file generated in {:?}",
        t.elapsed()
    );
    let id = p.import_asset(&user(), &long, &pr).unwrap().asset_id;
    let t = Instant::now();
    let snap = p
        .submit_frame_index(&id, Priority::Normal)
        .unwrap()
        .handle
        .wait();
    assert_eq!(snap.state, JobState::Completed);
    let t_idx = t.elapsed();
    let src = p.frame_source(&id, &tc, &never).unwrap();
    println!(
        "PERF index.long_18000_frames: {t_idx:?} ({} frames, {} keyframes, {} B index file)",
        src.index().len(),
        src.index().keyframe_count(),
        std::fs::metadata(snap.result.unwrap()["path"].as_str().unwrap())
            .unwrap()
            .len()
    );
    // leitura/decodificação do índice do cache (warm)
    let t = Instant::now();
    for _ in 0..20 {
        let _ = p.frame_source(&id, &tc, &never).unwrap();
    }
    println!(
        "PERF index.load_from_cache_18000_frames: {:?}/load",
        t.elapsed() / 20
    );
    // lookups em memória
    let ix = src.index();
    let t = Instant::now();
    let mut acc = 0usize;
    for i in 0..100_000i64 {
        acc += ix
            .frame_at_or_before(Ticks(i * TICKS_PER_SECOND / 170))
            .unwrap_or(0);
    }
    println!(
        "PERF index.frame_at_or_before: {:?}/lookup (acc {acc})",
        t.elapsed() / 100_000
    );
    // decode frio, "quente" (mesmo quadro) e consecutivos
    let t = Instant::now();
    let f0 = src.frame_by_index(9000, &never).unwrap();
    println!("PERF decode.cold_frame_mid_file: {:?}", t.elapsed());
    let t = Instant::now();
    let f1 = src.frame_by_index(9000, &never).unwrap();
    println!("PERF decode.repeat_same_frame: {:?}", t.elapsed());
    assert_eq!(f0.bytes, f1.bytes);
    let t = Instant::now();
    for i in 9001..9031 {
        src.frame_by_index(i, &never).unwrap();
    }
    println!(
        "PERF decode.30_consecutive_frames: {:?} ({:?}/frame)",
        t.elapsed(),
        t.elapsed() / 30
    );
    let t = Instant::now();
    let mut got = 0;
    src.frames(9031, 30, &never, &mut |_| {
        got += 1;
        capia_media::Flow::Continue
    })
    .unwrap();
    println!(
        "PERF decode.range_30_frames_one_process: {:?} ({:?}/frame, {got} frames)",
        t.elapsed(),
        t.elapsed() / 30
    );
    let t = Instant::now();
    src.frame_by_index(17_990, &never).unwrap();
    println!("PERF decode.frame_near_end: {:?}", t.elapsed());
    // áudio
    let t = Instant::now();
    let pcm = p
        .decode_audio(
            &id,
            Ticks(300 * TICKS_PER_SECOND),
            Ticks(TICKS_PER_SECOND),
            None,
            None,
            &tc,
            &never,
        )
        .unwrap();
    println!(
        "PERF audio.decode_1s_at_300s: {:?} ({} samples/ch @ {} Hz)",
        t.elapsed(),
        pcm.frames,
        pcm.sample_rate
    );
    let t = Instant::now();
    p.decode_audio(
        &id,
        Ticks::ZERO,
        Ticks(TICKS_PER_SECOND),
        None,
        None,
        &tc,
        &never,
    )
    .unwrap();
    println!("PERF audio.decode_1s_at_start: {:?}", t.elapsed());
    // waveform
    let t = Instant::now();
    let snap = p
        .submit_waveform(&id, Priority::Normal)
        .unwrap()
        .handle
        .wait();
    assert_eq!(snap.state, JobState::Completed);
    let t_wf = t.elapsed();
    let t = Instant::now();
    let w = p.waveform(&id, &tc, &never).unwrap();
    let t_read = t.elapsed();
    let t = Instant::now();
    for _ in 0..1000 {
        let _ = w.query(Ticks::ZERO, w.duration(), 2000);
    }
    println!(
        "PERF waveform.generate_10min: {t_wf:?} · read+validate {t_read:?} · query(2000 buckets) {:?}",
        t.elapsed() / 1000
    );
    // proxy: velocidade vs tempo real
    let t = Instant::now();
    let snap = p
        .submit_proxy(
            &id,
            ProxyProfileV1 {
                max_width: 640,
                max_height: 360,
                audio: ProxyAudio::Aac(96),
                ..Default::default()
            },
            Priority::Background,
        )
        .unwrap()
        .handle
        .wait();
    assert_eq!(snap.state, JobState::Completed, "{:?}", snap.error);
    let t_px = t.elapsed();
    println!(
        "PERF proxy.mjpeg_640x360_10min: {t_px:?} → {:.1}× real time ({} B)",
        600.0 / t_px.as_secs_f64(),
        std::fs::metadata(snap.result.unwrap()["path"].as_str().unwrap())
            .unwrap()
            .len()
    );
    // cache: lookup (hit) e hits concorrentes
    let t = Instant::now();
    for _ in 0..1000 {
        p.submit_frame_index(&id, Priority::Interactive)
            .unwrap()
            .handle
            .wait();
    }
    println!("PERF cache.hit_via_job_x1000: {:?}/job", t.elapsed() / 1000);
    let cache = p.cache_dir();
    let usage = cache.usage().unwrap();
    println!(
        "PERF cache.usage: {} files, {} bytes",
        usage.files, usage.bytes
    );
    let _ = std::fs::remove_dir_all(d);
}
