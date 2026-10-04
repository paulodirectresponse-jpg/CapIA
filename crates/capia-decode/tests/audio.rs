//! Cache de PCM por blocos: exatidão por amostra, fronteiras de bloco, cache, orçamento,
//! invalidação e custo independente do início.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_decode::*;
use capia_media::*;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    let d = std::env::temp_dir().join(format!("capia-pcm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn make(
    tc: &MediaToolchain,
    dir: &Path,
    name: &str,
    rate: u32,
    ch: u32,
    secs: u32,
    codec: &[&str],
) -> PathBuf {
    let out = dir.join(name);
    let (layout, expr) = if ch == 1 {
        ("mono", "sin(2*PI*t*(300+40*t))")
    } else {
        ("stereo", "sin(2*PI*t*(300+40*t))|sin(2*PI*t*(500+25*t))")
    };
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("aevalsrc={expr}:s={rate}:d={secs}:c={layout}"))
        .args(codec)
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

fn source(tc: &MediaToolchain, path: &Path, ns: u64, content: &str) -> (AudioSource, AudioPcm) {
    let info = FfprobeBackend::new(tc.clone()).probe(path).unwrap();
    let a = info.audio().unwrap().clone();
    let ix = build_audio_index(
        tc,
        path,
        a.index,
        a.sample_rate,
        a.channels,
        a.time_base.unwrap(),
        container_supports_exact_seek(&info.container.formats),
        std::time::Duration::from_secs(120),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let full = decode_audio(
        tc,
        path,
        &AudioRequest {
            stream_index: a.index,
            sample_rate: a.sample_rate,
            channels: a.channels,
            start: Ticks(0),
            duration: Ticks(60 * TICKS_PER_SECOND),
        },
        DEFAULT_MAX_PCM_BYTES,
        std::time::Duration::from_secs(120),
        &|| false,
    )
    .unwrap();
    assert_eq!(ix.total_samples(), full.frames);
    (
        AudioSource {
            namespace: ns,
            content: Arc::from(content),
            path: path.to_path_buf(),
            index: Arc::new(ix),
        },
        full,
    )
}

fn close(a: &[f32], b: &[f32], exact: bool) {
    assert_eq!(a.len(), b.len());
    let worst = a
        .iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0f32, f32::max);
    if exact {
        assert_eq!(worst, 0.0);
    } else {
        assert!(worst <= 1e-2, "worst {worst}");
    }
}

fn check_ranges(tc: &MediaToolchain, src: &AudioSource, full: &AudioPcm, exact: bool) {
    let cache = PcmCache::new(tc.clone(), 64 * 1024 * 1024);
    let ch = full.channels as usize;
    let total = full.frames;
    let b = PCM_BLOCK_FRAMES;
    let cases = [
        (0, 1),
        (0, 777),
        (b - 1, 2),      // cruza a fronteira de bloco
        (b, b),          // bloco inteiro alinhado
        (b - 5, b + 10), // cruza 2 fronteiras
        (3 * b + 17, 5000),
        (total - 100, 100),  // últimas amostras
        (total - 100, 5000), // além do fim: curta
        (total, 10),         // começa no fim: vazio
        (total + 99, 10),    // além do fim: vazio
    ];
    for (s, n) in cases {
        let got = cache.read(src, s, n, &|| false).unwrap();
        let want_end = (s + n).min(total);
        let want = if s >= total { 0 } else { want_end - s };
        assert_eq!(got.frames, want, "range {s}+{n}");
        if want > 0 {
            close(
                &got.samples,
                &full.samples[s as usize * ch..(s + want) as usize * ch],
                exact,
            );
        }
    }
    // blocos adjacentes concatenados == leitura única
    let one = cache.read(src, 5 * b - 100, 2 * b, &|| false).unwrap();
    let mut cat = cache
        .read(src, 5 * b - 100, b + 100, &|| false)
        .unwrap()
        .samples;
    cat.extend(cache.read(src, 6 * b, b - 100, &|| false).unwrap().samples);
    assert_eq!(one.samples, cat);
}

#[test]
fn pcm_wav_44k_mono_reads_are_bit_exact() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("wav");
    let p = make(&tc, &d, "a.wav", 44_100, 1, 8, &["-c:a", "pcm_s16le"]);
    let (src, full) = source(&tc, &p, 1, "sha256:w");
    check_ranges(&tc, &src, &full, true);
}

#[test]
fn aac_48k_stereo_reads_are_sample_exact_within_codec_tolerance() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("aac");
    let p = make(
        &tc,
        &d,
        "a.m4a",
        48_000,
        2,
        8,
        &["-c:a", "aac", "-b:a", "128k"],
    );
    let (src, full) = source(&tc, &p, 1, "sha256:a");
    check_ranges(&tc, &src, &full, false);
}

#[test]
fn short_clip_shorter_than_one_block_works() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("short");
    let p = make(&tc, &d, "s.wav", 48_000, 2, 1, &["-c:a", "pcm_s16le"]);
    // 0,05 s
    let p2 = d.join("short.wav");
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-i"])
        .arg(&p)
        .args(["-t", "0.05"])
        .arg(&p2)
        .status()
        .unwrap();
    assert!(st.success());
    let (src, full) = source(&tc, &p2, 1, "sha256:s");
    assert_eq!(full.frames, 2400);
    let cache = PcmCache::new(tc.clone(), 1 << 20);
    let got = cache.read(&src, 0, 100_000, &|| false).unwrap();
    assert_eq!(got.frames, 2400);
    assert_eq!(got.samples, full.samples);
}

#[test]
fn repeated_reads_hit_the_cache_without_decoding_again() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("hit");
    let p = make(&tc, &d, "a.wav", 48_000, 2, 6, &["-c:a", "pcm_s16le"]);
    let (src, _) = source(&tc, &p, 1, "sha256:h");
    let cache = PcmCache::new(tc.clone(), 64 << 20);
    let a = cache.read(&src, 10_000, 50_000, &|| false).unwrap();
    let m1 = cache.metrics();
    let b = cache.read(&src, 10_000, 50_000, &|| false).unwrap();
    let m2 = cache.metrics();
    assert_eq!(a.samples, b.samples);
    assert_eq!(m2.decode_calls, m1.decode_calls);
    assert!(m2.cache.hits > m1.cache.hits);
    // um pedido contíguo reaproveita os blocos já lidos (e o read-ahead): no máx. uma chamada nova
    cache.read(&src, 10_000, 100_000, &|| false).unwrap();
    assert!(cache.metrics().decode_calls <= m1.decode_calls + 1);
    // leitura sequencial: o read-ahead faz vários pedidos seguidos custarem UMA chamada
    let seq = PcmCache::new(tc.clone(), 64 << 20);
    for k in 0..4u64 {
        seq.read(&src, k * 40_000, 40_000, &|| false).unwrap();
    }
    assert!(seq.metrics().decode_calls <= 2, "{:?}", seq.metrics());
}

#[test]
fn byte_budget_is_respected_and_content_change_invalidates() {
    let Some(tc) = toolchain() else { return };
    let d = tmp("budget");
    let p = make(&tc, &d, "a.wav", 48_000, 2, 10, &["-c:a", "pcm_s16le"]);
    let (src, full) = source(&tc, &p, 1, "sha256:b");
    let block_bytes = PCM_BLOCK_FRAMES * 2 * 4;
    let cache = PcmCache::new(tc.clone(), block_bytes * 3);
    for k in 0..12u64 {
        cache
            .read(&src, k * PCM_BLOCK_FRAMES, PCM_BLOCK_FRAMES, &|| false)
            .unwrap();
        assert!(cache.metrics().cache.bytes <= block_bytes * 3);
    }
    assert!(cache.metrics().cache.evictions >= 9);
    // outro conteúdo no mesmo caminho: chave diferente ⇒ não reaproveita
    let other = AudioSource {
        content: Arc::from("sha256:other"),
        ..src.clone()
    };
    let before = cache.metrics().decode_calls;
    cache
        .read(&other, 11 * PCM_BLOCK_FRAMES, 100, &|| false)
        .unwrap();
    assert_eq!(cache.metrics().decode_calls, before + 1);
    // isolamento entre projetos
    let ns2 = AudioSource {
        namespace: 2,
        ..src.clone()
    };
    let before = cache.metrics().decode_calls;
    let g = cache
        .read(&ns2, 11 * PCM_BLOCK_FRAMES, 100, &|| false)
        .unwrap();
    assert_eq!(cache.metrics().decode_calls, before + 1);
    let ch = 2usize;
    assert_eq!(
        g.samples,
        full.samples
            [11 * PCM_BLOCK_FRAMES as usize * ch..(11 * PCM_BLOCK_FRAMES as usize + 100) * ch]
    );
    cache.invalidate_namespace(2);
    let before = cache.metrics().decode_calls;
    cache
        .read(&ns2, 11 * PCM_BLOCK_FRAMES, 100, &|| false)
        .unwrap();
    assert_eq!(cache.metrics().decode_calls, before + 1);
}
