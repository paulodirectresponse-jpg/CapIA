//! Goldens de áudio do mixer (8 casos + mute/solo/ganho/clip/retime): comparação com a senoide
//! analítica, tolerância absoluta documentada de **1e-4** (acúmulo em f32/f64).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_model::{ClipContent, TrackKind};
use capia_render::{AudioBuffer, RenderGraph, RenderSettings, mix_audio_range};
use capia_time::{FrameRate, Rational, TICKS_PER_SECOND, Ticks, TimeRange};
use common::*;

const TOL: f32 = 1e-4;
const SEC: i64 = TICKS_PER_SECOND;

fn tone_at(freq: f64, rate: u32, n: i64) -> f32 {
    if n < 0 {
        return 0.0;
    }
    (0.5 * (2.0 * std::f64::consts::PI * freq * n as f64 / f64::from(rate)).sin()) as f32
}

fn mix(d: &Dsl, src: &Synth, seq: &str, start: i64, dur: i64, rate: u32) -> AudioBuffer {
    let g = RenderGraph::compile(d.doc(), &seq.into()).unwrap();
    let mut s = RenderSettings::new(16, 16);
    s.audio_sample_rate = rate;
    let (buf, warnings) = mix_audio_range(
        &g,
        &seq.into(),
        TimeRange::new(Ticks(start), Ticks(dur)),
        &s,
        src,
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    buf
}

fn assert_close(buf: &AudioBuffer, f: impl Fn(i64) -> f32, what: &str) {
    let ch = buf.channels as usize;
    for n in 0..buf.frames() as i64 {
        let want = f(n);
        for c in 0..ch {
            let got = buf.samples[n as usize * ch + c];
            assert!(
                (got - want).abs() <= TOL,
                "{what}: frame {n} ch {c}: {got} vs {want}"
            );
        }
    }
}

fn setup() -> Dsl {
    let mut d = Dsl::new();
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "A1", TrackKind::Audio);
    d.track("S", "A2", TrackKind::Audio);
    d.register("a440", 10, false, true);
    d.register("a660", 10, false, true);
    d
}

fn synth() -> Synth {
    Synth::default()
        .tone("a440", 440.0, 10.0)
        .tone("a660", 660.0, 10.0)
}

#[test]
fn a1_single_sine() {
    let mut d = setup();
    d.media("A1", "c", "a440", 0, 30 * F, false, true);
    let buf = mix(&d, &synth(), "S", 0, SEC, 48_000);
    assert_eq!(buf.frames(), 48_000);
    assert_close(&buf, |n| tone_at(440.0, 48_000, n), "single");
}

#[test]
fn a2_two_overlapping_sines_sum() {
    let mut d = setup();
    d.media("A1", "c1", "a440", 0, 30 * F, false, true);
    d.media("A2", "c2", "a660", 0, 30 * F, false, true);
    let buf = mix(&d, &synth(), "S", 0, SEC, 48_000);
    assert_close(
        &buf,
        |n| (tone_at(440.0, 48_000, n) + tone_at(660.0, 48_000, n)).clamp(-1.0, 1.0),
        "sum",
    );
}

#[test]
fn a3_offset_clip_is_silent_before_it_starts() {
    let mut d = setup();
    d.media("A1", "c", "a440", 15 * F, 30 * F, false, true); // começa em 0,5 s
    let buf = mix(&d, &synth(), "S", 0, SEC, 48_000);
    assert_close(&buf, |n| tone_at(440.0, 48_000, n - 24_000), "offset");
    assert!(buf.samples[..2 * 23_999].iter().all(|v| *v == 0.0));
}

#[test]
fn a4_trim_uses_the_source_offset() {
    let mut d = setup();
    d.clip(
        "A1",
        "c",
        ClipContent::Media {
            asset: "a440".into(),
            has_video: false,
            has_audio: true,
        },
        0,
        30 * F,
        7 * F + F / 2,
        Rational::ONE,
    );
    // source_in = 7,5 frames = 0,25 s ⇒ 12.000 amostras
    let buf = mix(&d, &synth(), "S", 0, SEC / 2, 48_000);
    assert_close(&buf, |n| tone_at(440.0, 48_000, n + 12_000), "trim");
}

#[test]
fn a5_silence_when_nothing_plays() {
    let d = setup();
    let buf = mix(&d, &synth(), "S", 0, SEC, 48_000);
    assert!(buf.samples.iter().all(|v| *v == 0.0));
    assert_eq!(buf.samples.len(), 96_000);
}

#[test]
fn a6_nested_timeline_audio() {
    let mut d = Dsl::new();
    d.register("a440", 10, false, true);
    d.seq("CHILD", FrameRate::FPS_30);
    d.track("CHILD", "CA", TrackKind::Audio);
    d.media("CA", "cc", "a440", 6 * F, 30 * F, false, true); // filho: tom começa em 0,2 s
    d.seq("S", FrameRate::FPS_30);
    d.track("S", "A1", TrackKind::Audio);
    d.nested("A1", "n", "CHILD", 3 * F, 30 * F, 0); // pai: filho começa em 0,1 s
    let buf = mix(&d, &synth(), "S", 0, SEC, 48_000);
    // tom começa em 0,3 s = 14.400 amostras
    assert_close(&buf, |n| tone_at(440.0, 48_000, n - 14_400), "nested");
}

#[test]
fn a7_output_policy_44100() {
    let mut d = setup();
    d.media("A1", "c", "a440", 0, 30 * F, false, true);
    let buf = mix(&d, &synth(), "S", 0, SEC, 44_100);
    assert_eq!(buf.sample_rate, 44_100);
    assert_eq!(buf.frames(), 44_100);
    assert_close(&buf, |n| tone_at(440.0, 44_100, n), "44.1k");
}

#[test]
fn a8_adjacent_blocks_join_without_a_seam() {
    let mut d = setup();
    d.media("A1", "c", "a440", 0, 30 * F, false, true);
    let s = synth();
    let whole = mix(&d, &s, "S", 0, SEC, 48_000);
    let first = mix(&d, &s, "S", 0, SEC / 2, 48_000);
    let second = mix(&d, &s, "S", SEC / 2, SEC / 2, 48_000);
    let mut joined = first.samples.clone();
    joined.extend_from_slice(&second.samples);
    assert_eq!(
        joined, whole.samples,
        "blocks must be bit-identical to the whole render"
    );
}

#[test]
fn clipping_mute_solo_gain_and_retime() {
    let mut d = setup();
    d.track("S", "A3", TrackKind::Audio);
    d.media("A1", "c1", "a440", 0, 30 * F, false, true);
    d.media("A2", "c2", "a440", 0, 30 * F, false, true);
    d.media("A3", "c3", "a440", 0, 30 * F, false, true);
    let s = synth();
    // três tons iguais somam 1,5 de pico ⇒ hard clip em ±1
    let buf = mix(&d, &s, "S", 0, SEC / 100, 48_000);
    assert!(buf.samples.iter().all(|v| (-1.0..=1.0).contains(v)));
    assert!(
        buf.samples.iter().any(|v| *v == 1.0 || *v == -1.0),
        "peaks must be clipped, not wrapped"
    );
    // ganho: −6,0206 dB ≈ metade
    let mut g = setup();
    g.media("A1", "c", "a440", 0, 30 * F, false, true);
    g.prop("c", "volume_db", -6.020_599_913_279_624);
    let half = mix(&g, &s, "S", 0, SEC, 48_000);
    assert_close(&half, |n| tone_at(440.0, 48_000, n) * 0.5, "gain");
    // mute e solo
    let mut m = setup();
    m.media("A1", "c1", "a440", 0, 30 * F, false, true);
    m.media("A2", "c2", "a660", 0, 30 * F, false, true);
    m.run(capia_commands::Command::SetTrackFlags {
        track: "A1".into(),
        locked: None,
        hidden: None,
        muted: Some(true),
        solo: None,
        magnetic: None,
        sync_lock: None,
        group: None,
        clear_group: false,
        compact: false,
    });
    let muted = mix(&m, &s, "S", 0, SEC, 48_000);
    assert_close(&muted, |n| tone_at(660.0, 48_000, n), "mute A1 ⇒ só 660");
    m.run(capia_commands::Command::SetTrackFlags {
        track: "A2".into(),
        locked: None,
        hidden: None,
        muted: None,
        solo: Some(true),
        magnetic: None,
        sync_lock: None,
        group: None,
        clear_group: false,
        compact: false,
    });
    let solo = mix(&m, &s, "S", 0, SEC, 48_000);
    assert_close(&solo, |n| tone_at(660.0, 48_000, n), "solo A2");
    // retime 2×: a fonte anda o dobro (varispeed) ⇒ a amostra n lê a posição 2n
    let mut r = setup();
    r.clip(
        "A1",
        "c",
        ClipContent::Media {
            asset: "a440".into(),
            has_video: false,
            has_audio: true,
        },
        0,
        30 * F,
        0,
        Rational::from_int(2),
    );
    let fast = mix(&r, &s, "S", 0, SEC / 2, 48_000);
    assert_close(&fast, |n| tone_at(440.0, 48_000, 2 * n), "2x");
}
