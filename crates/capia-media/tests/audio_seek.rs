//! Seek de áudio por índice de frames: o decode saltando é **idêntico bit a bit** ao decode desde o
//! início, em AAC e PCM, a 44,1 e 48 kHz, mono e estéreo, em amostras de fronteira de frame.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_media::*;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

fn never() -> bool {
    false
}

/// Gera um áudio com chirp (nenhuma amostra se repete ⇒ qualquer desalinhamento aparece).
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
    let layout = if ch == 1 { "mono" } else { "stereo" };
    let expr = if ch == 1 {
        "sin(2*PI*t*(300+40*t))".to_owned()
    } else {
        "sin(2*PI*t*(300+40*t))|sin(2*PI*t*(500+25*t))".to_owned()
    };
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("aevalsrc={expr}:s={rate}:d={secs}:c={layout}"))
        .args(codec)
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg failed to make {name}");
    out
}

fn index_of(tc: &MediaToolchain, path: &Path) -> (AudioIndex, u32) {
    let info = FfprobeBackend::new(tc.clone()).probe(path).unwrap();
    let a = info.audio().unwrap().clone();
    let tb = a.time_base.unwrap();
    let ix = build_audio_index(
        tc,
        path,
        a.index,
        a.sample_rate,
        a.channels,
        tb,
        container_supports_exact_seek(&info.container.formats),
        Duration::from_secs(120),
        &never,
        &mut |_| {},
    )
    .unwrap();
    (ix, a.channels)
}

fn full_decode(tc: &MediaToolchain, path: &Path, ix: &AudioIndex, ch: u32) -> AudioPcm {
    decode_audio(
        tc,
        path,
        &AudioRequest {
            stream_index: ix.stream_index(),
            sample_rate: ix.sample_rate(),
            channels: ch,
            start: Ticks(0),
            duration: Ticks(60 * TICKS_PER_SECOND),
        },
        DEFAULT_MAX_PCM_BYTES,
        Duration::from_secs(120),
        &never,
    )
    .unwrap()
}

/// PCM: idêntico bit a bit. Codecs com perdas (AAC): a **posição** é exata e os valores coincidem
/// até o ruído de estado do decoder (PNS/overlap, ≈ 1e-5): um deslocamento de UMA amostra num chirp
/// de amplitude ~0,9 daria ≥ 1e-2, então a tolerância de 1e-2 ainda pega qualquer desalinhamento.
fn assert_same(got: &[f32], want: &[f32], exact: bool, msg: &str) {
    assert_eq!(got.len(), want.len(), "{msg}: length");
    if exact {
        assert_eq!(got, want, "{msg}");
        return;
    }
    let worst = got
        .iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst <= 1e-2, "{msg}: worst difference {worst}");
}

fn check(tc: &MediaToolchain, path: &Path, what: &str) {
    let exact = path.extension().is_some_and(|e| e == "wav");
    let (ix, ch) = index_of(tc, path);
    assert!(
        ix.fast_seek(),
        "{what}: this container must use the fast path"
    );
    let full = full_decode(tc, path, &ix, ch);
    assert_eq!(
        ix.total_samples(),
        full.frames,
        "{what}: the index must account for exactly the decoded samples"
    );
    let total = full.frames;
    let starts = [
        0,
        1,
        1023,
        1024,
        1025,
        4097,
        20_000,
        total / 2,
        total / 2 + 1,
        total - 5000,
        total - 1,
    ];
    for &s in &starts {
        for &n in &[1u64, 777, 4096] {
            let got =
                decode_audio_indexed(tc, path, &ix, s, n, ch, Duration::from_secs(60), &never)
                    .unwrap();
            let want_frames = n.min(total - s);
            assert_eq!(got.frames, want_frames, "{what}: start {s} n {n}");
            let (a, b) = (
                (s * u64::from(ch)) as usize,
                ((s + want_frames) * u64::from(ch)) as usize,
            );
            assert_same(
                &got.samples,
                &full.samples[a..b],
                exact,
                &format!("{what}: start {s} n {n}"),
            );
        }
    }
    // além do fim: vazio, sem erro
    let beyond = decode_audio_indexed(
        tc,
        path,
        &ix,
        total + 10,
        100,
        ch,
        Duration::from_secs(60),
        &never,
    )
    .unwrap();
    assert_eq!(beyond.frames, 0);
    // blocos adjacentes se juntam sem emenda
    let mut joined = Vec::new();
    let mut s = 30_000u64;
    for _ in 0..6 {
        let blk = decode_audio_indexed(tc, path, &ix, s, 3000, ch, Duration::from_secs(60), &never)
            .unwrap();
        joined.extend_from_slice(&blk.samples);
        s += 3000;
    }
    assert_same(
        &joined,
        &full.samples[(30_000 * ch as usize)..(30_000 + 18_000) * ch as usize],
        exact,
        &format!("{what}: adjacent blocks"),
    );
    // sanidade do critério: a mesma janela deslocada em UMA amostra NÃO passa na tolerância
    if !exact {
        let a = 30_000 * ch as usize;
        let shifted = &full.samples[a + ch as usize..a + ch as usize + 3000 * ch as usize];
        let blk = decode_audio_indexed(
            tc,
            path,
            &ix,
            30_000,
            3000,
            ch,
            Duration::from_secs(60),
            &never,
        )
        .unwrap();
        let worst = blk
            .samples
            .iter()
            .zip(shifted)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst > 1e-2,
            "{what}: the tolerance must be able to see a 1-sample shift (worst {worst})"
        );
    }
}

#[test]
fn aac_48k_stereo_seek_is_sample_exact() {
    let Some(tc) = toolchain() else { return };
    let d = std::env::temp_dir().join(format!("capia-aseek-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = make(
        &tc,
        &d,
        "a48.m4a",
        48_000,
        2,
        20,
        &["-c:a", "aac", "-b:a", "96k"],
    );
    check(&tc, &f, "aac 48k stereo");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn aac_44k_mono_seek_is_sample_exact() {
    let Some(tc) = toolchain() else { return };
    let d = std::env::temp_dir().join(format!("capia-aseek44-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = make(
        &tc,
        &d,
        "a44.m4a",
        44_100,
        1,
        20,
        &["-c:a", "aac", "-b:a", "64k"],
    );
    check(&tc, &f, "aac 44.1k mono");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn pcm_wav_44k_and_48k_seek_is_sample_exact() {
    let Some(tc) = toolchain() else { return };
    let d = std::env::temp_dir().join(format!("capia-aseekpcm-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    check(
        &tc,
        &make(&tc, &d, "p44.wav", 44_100, 2, 15, &["-c:a", "pcm_s16le"]),
        "pcm 44.1k",
    );
    check(
        &tc,
        &make(&tc, &d, "p48.wav", 48_000, 1, 15, &["-c:a", "pcm_s16le"]),
        "pcm 48k mono",
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn aac_in_mp4_with_video_and_a_nonzero_start() {
    let Some(tc) = toolchain() else { return };
    let d = std::env::temp_dir().join(format!("capia-aseekvid-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join(if std::env::var_os("CAPIA_ASEEK_MKV").is_some() {
        "av.mkv"
    } else {
        "av.mp4"
    });
    // áudio com deslocamento de 1,5 s no container (start ≠ 0)
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25:duration=10",
            "-itsoffset",
            "1.5",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("aevalsrc=sin(2*PI*t*(300+40*t))|sin(2*PI*t*(500+25*t)):s=48000:d=10:c=stereo")
        .args(["-c:v", "mpeg4", "-c:a", "aac", "-b:a", "96k"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    // o stream de áudio é o índice 1
    let info = FfprobeBackend::new(tc.clone()).probe(&out).unwrap();
    let a = info.audio().unwrap().clone();
    let ix = build_audio_index(
        &tc,
        &out,
        a.index,
        a.sample_rate,
        a.channels,
        a.time_base.unwrap(),
        container_supports_exact_seek(&info.container.formats),
        Duration::from_secs(60),
        &never,
        &mut |_| {},
    )
    .unwrap();
    let full = decode_audio(
        &tc,
        &out,
        &AudioRequest {
            stream_index: a.index,
            sample_rate: a.sample_rate,
            channels: a.channels,
            start: Ticks(0),
            duration: Ticks(20 * TICKS_PER_SECOND),
        },
        DEFAULT_MAX_PCM_BYTES,
        Duration::from_secs(60),
        &never,
    )
    .unwrap();
    assert_eq!(ix.total_samples(), full.frames);
    for s in [0u64, 777, 48_000, 200_000, full.frames - 3000] {
        let got = decode_audio_indexed(
            &tc,
            &out,
            &ix,
            s,
            2000,
            a.channels,
            Duration::from_secs(60),
            &never,
        )
        .unwrap();
        let ch = a.channels as usize;
        let n = got.frames as usize;
        assert_same(
            &got.samples,
            &full.samples[s as usize * ch..(s as usize + n) * ch],
            false,
            &format!("start {s}"),
        );
    }
    let _ = std::fs::remove_dir_all(d);
}
