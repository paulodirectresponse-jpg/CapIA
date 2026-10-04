//! Serviço de decode: reaproveitamento de sessões, cache por bytes, prefetch, prioridade,
//! cancelamento, supersession e isolamento entre projetos — com ffmpeg real.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_decode::*;
use capia_media::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "{other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
            None
        }
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("capia-decode-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 96 quadros 64x48 a 30 fps, GOP 12 (cada quadro tem conteúdo distinto: testsrc2).
fn make_video(tc: &MediaToolchain, dir: &Path, name: &str, pattern: &str) -> PathBuf {
    let out = dir.join(name);
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("{pattern}=size=64x48:rate=30"))
        .args(["-t", "3.2", "-c:v", "mpeg4", "-g", "12", "-qscale:v", "3"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

fn source(
    tc: &MediaToolchain,
    path: &Path,
    ns: u64,
    content: &str,
    stream: Option<u32>,
) -> Arc<VideoSource> {
    let info = FfprobeBackend::new(tc.clone()).probe(path).unwrap();
    let v = match stream {
        None => info.video().unwrap().clone(),
        Some(i) => info
            .streams
            .iter()
            .find_map(|s| match s {
                StreamInfo::Video(v) if v.index == i => Some(v.clone()),
                _ => None,
            })
            .unwrap(),
    };
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
        namespace: ns,
        content: Arc::from(content),
        path: path.to_path_buf(),
        index: Arc::new(index),
        width: v.width,
        height: v.height,
    })
}

fn reference(tc: &MediaToolchain, s: &VideoSource, i: usize) -> Vec<u8> {
    decode_frame_by_index(
        tc,
        &s.path,
        &s.index,
        s.width,
        s.height,
        i,
        &DecodeLimits::default(),
        &|| false,
    )
    .unwrap()
    .bytes
}

fn svc(tc: &MediaToolchain, f: impl FnOnce(&mut DecodeConfig)) -> DecodeService {
    let mut c = DecodeConfig::new(tc.clone());
    f(&mut c);
    DecodeService::new(c)
}

const FRAME: u64 = 64 * 48 * 4;

#[test]
fn sequential_playback_reuses_one_session_and_matches_the_reference() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("seq");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    assert_eq!(s.index.len(), 96);
    let sv = svc(&tc, |_| {});
    for i in 0..40 {
        let f = sv.get_frame(&s, i, Priority::Playback).unwrap();
        assert_eq!(f.index, i);
        assert_eq!(f.bytes, reference(&tc, &s, i), "frame {i}");
    }
    let m = sv.metrics();
    assert_eq!(m.sessions_opened, 1, "{m:?}");
    assert!(m.sessions_reused >= 30, "{m:?}");
    assert!(m.reuse_rate() > 0.9, "{m:?}");
    assert_eq!(m.sessions_alive, 1);
}

#[test]
fn repeated_request_is_a_cache_hit_with_the_same_allocation() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("hit");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |_| {});
    let a = sv.get_frame(&s, 30, Priority::Interactive).unwrap();
    let before = sv.metrics();
    let b = sv.get_frame(&s, 30, Priority::Interactive).unwrap();
    let after = sv.metrics();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(after.sessions_opened, before.sessions_opened);
    assert_eq!(after.frames_decoded, before.frames_decoded);
    assert_eq!(after.cache.hits, before.cache.hits + 1);
}

#[test]
fn cache_respects_its_byte_budget_and_evicts_lru() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("lru");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |c| c.frame_cache_bytes = FRAME * 5 + 10);
    for i in 0..30 {
        sv.get_frame(&s, i, Priority::Playback).unwrap();
        assert!(sv.metrics().cache.bytes <= FRAME * 5 + 10);
    }
    let m = sv.metrics();
    assert_eq!(m.cache.entries, 5, "{m:?}");
    assert!(m.cache.evictions >= 25, "{m:?}");
    // os mais antigos saíram; os mais recentes ficaram
    assert!(sv.cached(&s, 0).is_none());
    assert!(sv.cached(&s, 29).is_some());
    // um orçamento menor que UM quadro nunca guarda nada (e nunca estoura)
    let tiny = svc(&tc, |c| c.frame_cache_bytes = FRAME - 1);
    let f = tiny.get_frame(&s, 3, Priority::Interactive).unwrap();
    assert_eq!(f.bytes.len() as u64, FRAME);
    assert_eq!(tiny.metrics().cache.bytes, 0);
}

#[test]
fn changed_content_never_reuses_a_cached_frame() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("content");
    let p = d.join("m.mp4");
    std::fs::copy(make_video(&tc, &d, "one.mp4", "testsrc2"), &p).unwrap();
    let s1 = source(&tc, &p, 1, "sha256:one", None);
    let sv = svc(&tc, |_| {});
    let f1 = sv.get_frame(&s1, 10, Priority::Interactive).unwrap();
    // mesmo caminho, outro conteúdo (outro hash)
    std::fs::copy(make_video(&tc, &d, "two.mp4", "mandelbrot"), &p).unwrap();
    let s2 = source(&tc, &p, 1, "sha256:two", None);
    assert!(sv.cached(&s2, 10).is_none());
    let f2 = sv.get_frame(&s2, 10, Priority::Interactive).unwrap();
    assert_ne!(f1.bytes, f2.bytes);
    assert_eq!(f2.bytes, reference(&tc, &s2, 10));
    // invalidar o conteúdo antigo remove só o dele
    sv.invalidate_content(1, "sha256:one");
    assert!(sv.cached(&s1, 10).is_none());
    assert!(sv.cached(&s2, 10).is_some());
}

#[test]
fn projects_never_share_frames_or_sessions() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("ns");
    let p = make_video(&tc, &d, "a.mp4", "testsrc2");
    let a = source(&tc, &p, 1, "sha256:same", None);
    let b = source(&tc, &p, 2, "sha256:same", None);
    let sv = svc(&tc, |_| {});
    sv.get_frame(&a, 5, Priority::Interactive).unwrap();
    assert!(sv.cached(&a, 5).is_some());
    assert!(
        sv.cached(&b, 5).is_none(),
        "another project must not see it"
    );
    let opened = sv.metrics().sessions_opened;
    sv.get_frame(&b, 5, Priority::Interactive).unwrap();
    assert_eq!(sv.metrics().sessions_opened, opened + 1);
    sv.invalidate_namespace(1);
    assert!(sv.cached(&a, 5).is_none());
    assert!(sv.cached(&b, 5).is_some());
}

#[test]
fn two_video_streams_of_one_file_are_cached_and_decoded_separately() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("multi");
    let out = d.join("two.mp4");
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=64x48:rate=30:duration=2")
        .args(["-f", "lavfi", "-i"])
        .arg("mandelbrot=size=64x48:rate=30")
        .args([
            "-t", "2", "-map", "0:v", "-map", "1:v", "-c:v", "mpeg4", "-g", "12",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    let s0 = source(&tc, &out, 1, "sha256:two", Some(0));
    let s1 = source(&tc, &out, 1, "sha256:two", Some(1));
    let sv = svc(&tc, |_| {});
    let f0 = sv.get_frame(&s0, 20, Priority::Interactive).unwrap();
    let f1 = sv.get_frame(&s1, 20, Priority::Interactive).unwrap();
    assert_ne!(f0.bytes, f1.bytes);
    assert_eq!(f0.bytes, reference(&tc, &s0, 20));
    assert_eq!(f1.bytes, reference(&tc, &s1, 20));
    assert_eq!(sv.metrics().sessions_opened, 2);
}

#[test]
fn concurrent_clients_get_correct_frames_within_the_session_limit() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("conc");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = Arc::new(svc(&tc, |c| c.max_sessions = 2));
    let refs: Arc<Vec<Vec<u8>>> = Arc::new((0..96).map(|i| reference(&tc, &s, i)).collect());
    let mut hs = Vec::new();
    for t in 0..6u64 {
        let (sv, s, refs) = (Arc::clone(&sv), Arc::clone(&s), Arc::clone(&refs));
        hs.push(std::thread::spawn(move || {
            let mut x = t * 7919 + 13;
            for _ in 0..12 {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let i = ((x >> 33) % 96) as usize;
                let f = sv.get_frame(&s, i, Priority::Interactive).unwrap();
                assert_eq!(f.bytes, refs[i], "thread {t} frame {i}");
                assert!(sv.metrics().sessions_alive <= 2);
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    assert!(sv.metrics().sessions_alive <= 2);
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(30), "timeout: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn prefetch_fills_the_cache_forward_and_backward_but_not_on_jump() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("pre");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |c| {
        c.prefetch_ahead = 6;
        c.backward_window = 5;
    });
    sv.get_frame(&s, 20, Priority::Interactive).unwrap();
    let t = sv.prefetch(&s, 20, Direction::Forward, None).unwrap();
    t.wait().unwrap();
    for i in 21..=26 {
        assert!(sv.cached(&s, i).is_some(), "forward {i}");
    }
    assert!(sv.cached(&s, 27).is_none());
    // scrub para trás limitado
    sv.get_frame(&s, 70, Priority::Interactive).unwrap();
    let t = sv.prefetch(&s, 70, Direction::Backward, None).unwrap();
    t.wait().unwrap();
    for i in 65..70 {
        assert!(sv.cached(&s, i).is_some(), "backward {i}");
    }
    // salto: nada a pré-carregar
    assert!(sv.prefetch(&s, 5, Direction::Jump, None).is_none());
    // fim do vídeo: não pede além do último quadro
    let last = s.index.len() - 1;
    assert!(sv.prefetch(&s, last, Direction::Forward, None).is_none());
    // o prefetch de leitura sequencial mantém a sessão viva (quadros seguintes = cache)
    let m = sv.metrics();
    assert!(m.frames_decoded > 0 && m.completed[Priority::Background as usize] >= 2);
}

#[test]
fn interactive_requests_outrank_background_ones() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("prio");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |c| c.max_sessions = 1);
    let bg: Vec<_> = [90usize, 10, 80, 20, 70, 30]
        .iter()
        .map(|i| sv.request(&s, *i, Priority::Background, None))
        .collect();
    let hot = sv.request(&s, 50, Priority::Interactive, None);
    hot.wait().unwrap();
    for t in &bg {
        t.wait().unwrap();
    }
    // o 1º background já estava em execução; do 2º em diante o interativo passa na frente
    let later = bg
        .iter()
        .skip(1)
        .filter(|t| t.completed_seq() < hot.completed_seq())
        .count();
    assert_eq!(
        later, 0,
        "interactive must finish before the queued background requests"
    );
}

#[test]
fn rapid_scrub_supersedes_obsolete_requests() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("scrub");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |c| c.max_sessions = 1);
    let lane = Some(Lane(7));
    let t: Vec<_> = [70usize, 80, 60, 20]
        .iter()
        .map(|i| sv.request(&s, *i, Priority::Interactive, lane))
        .collect();
    for old in &t[..3] {
        assert_eq!(old.wait().unwrap_err(), DecodeError::Superseded);
    }
    let last = t[3].wait().unwrap();
    assert_eq!(last.index, 20);
    assert_eq!(last.bytes, reference(&tc, &s, 20));
    assert_eq!(sv.metrics().superseded, 3);
    // pistas diferentes não se atropelam
    let a = sv.request(&s, 11, Priority::Interactive, Some(Lane(1)));
    let b = sv.request(&s, 12, Priority::Interactive, Some(Lane(2)));
    assert!(a.wait().is_ok() && b.wait().is_ok());
}

#[test]
fn cancelling_a_queued_request_never_decodes_it() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("cancel");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |c| c.max_sessions = 1);
    let first = sv.request(&s, 90, Priority::Interactive, None);
    let doomed = sv.request(&s, 10, Priority::Background, None);
    doomed.cancel();
    first.wait().unwrap();
    assert_eq!(doomed.wait().unwrap_err(), DecodeError::Cancelled);
    assert!(sv.cached(&s, 10).is_none());
    assert_eq!(sv.metrics().cancelled, 1);
}

#[test]
fn idle_sessions_are_evicted_and_the_pool_is_bounded_under_pressure() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("idle");
    let a = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let b = source(
        &tc,
        &make_video(&tc, &d, "b.mp4", "mandelbrot"),
        1,
        "sha256:b",
        None,
    );
    let c = source(
        &tc,
        &make_video(&tc, &d, "c.mp4", "rgbtestsrc"),
        1,
        "sha256:c",
        None,
    );
    let sv = svc(&tc, |cfg| {
        cfg.max_sessions = 2;
        cfg.idle_timeout = Duration::from_secs(3);
    });
    sv.get_frame(&a, 3, Priority::Interactive).unwrap();
    sv.get_frame(&b, 3, Priority::Interactive).unwrap();
    sv.get_frame(&c, 3, Priority::Interactive).unwrap();
    let m = sv.metrics();
    assert!(m.sessions_alive <= 2, "{m:?}");
    assert!(m.sessions_evicted_pressure >= 1, "{m:?}");
    wait_until("idle eviction", || sv.metrics().sessions_alive == 0);
    assert!(sv.metrics().sessions_evicted_idle >= 1);
    // depois de evictada, a próxima leitura abre outra e continua correta
    let f = sv.get_frame(&a, 40, Priority::Interactive).unwrap();
    assert_eq!(f.bytes, reference(&tc, &a, 40));
}

#[test]
fn shutdown_resolves_pending_requests_and_kills_sessions() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("down");
    let s = source(
        &tc,
        &make_video(&tc, &d, "a.mp4", "testsrc2"),
        1,
        "sha256:a",
        None,
    );
    let sv = svc(&tc, |c| c.max_sessions = 1);
    sv.get_frame(&s, 1, Priority::Interactive).unwrap();
    assert_eq!(sv.metrics().sessions_alive, 1);
    sv.shutdown();
    assert_eq!(sv.metrics().sessions_alive, 0);
    let t = sv.request(&s, 2, Priority::Interactive, None);
    assert_eq!(t.wait().unwrap_err(), DecodeError::Shutdown);
}
