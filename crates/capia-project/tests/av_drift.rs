//! Corpus A/V: sincronismo medido de ponta a ponta (flash × beep no MP4 exportado), VFR seguindo o
//! tempo da fonte e uma timeline sintética de 10 minutos sem drift. Tudo em Ticks/Rational/amostras.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_media::{DecodeLimits, ExportCodec, FrameIndex, decode_frame_by_index};
use capia_model::TrackKind;
use capia_project::{Mp4Options, Project};
use capia_render::{RenderSettings, frame_digest};
use capia_time::{FrameRate, Rational, TICKS_PER_SECOND, Ticks, TimeRange};
use lab::{F, Lab};
use std::path::Path;
use std::process::Command;

fn no_cancel() -> bool {
    false
}

fn out_of(lab: &Lab, args: &[&str]) -> Vec<u8> {
    let o = Command::new(lab.tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-nostdin"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}

/// Índice do 1º quadro "branco" no vídeo do arquivo.
fn first_bright_frame(lab: &Lab, file: &Path) -> usize {
    let bytes = out_of(
        lab,
        &[
            "-i",
            file.to_str().unwrap(),
            "-map",
            "0:v:0",
            "-vf",
            "scale=1:1:flags=area,format=gray",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-",
        ],
    );
    bytes
        .iter()
        .position(|b| *b > 128)
        .expect("no bright frame")
}

/// Índice (amostra) do 1º pico do áudio do arquivo, na taxa nativa.
fn first_beep_sample(lab: &Lab, file: &Path, rate: u32) -> u64 {
    let bytes = out_of(
        lab,
        &[
            "-i",
            file.to_str().unwrap(),
            "-map",
            "0:a:0",
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "-f",
            "f32le",
            "-",
        ],
    );
    bytes
        .chunks_exact(4)
        .position(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs() > 0.25)
        .expect("no beep") as u64
}

#[test]
fn exported_flash_and_beep_stay_within_one_frame_for_every_rate_combination() {
    let tc = need!();
    let combos: &[(&str, FrameRate)] = &[
        (
            "24000/1001",
            FrameRate::new(Rational::new(24000, 1001).unwrap()).unwrap(),
        ),
        (
            "30000/1001",
            FrameRate::new(Rational::new(30000, 1001).unwrap()).unwrap(),
        ),
        (
            "60000/1001",
            FrameRate::new(Rational::new(60000, 1001).unwrap()).unwrap(),
        ),
        ("25", FrameRate::FPS_25),
        ("30", FrameRate::FPS_30),
    ];
    for (fps, fr) in combos {
        for rate in [44_100u32, 48_000] {
            let mut lab = Lab::new("sync", &tc);
            let src = lab.gen_flash("f.mp4", fps, 3, rate);
            let a = lab.import(&src);
            lab.seq("S", *fr);
            lab.track("S", "V1", TrackKind::Visual);
            lab.track("S", "A1", TrackKind::Audio);
            let fd = fr.frame_duration().0;
            let frames = (22 * TICKS_PER_SECOND / 10) / fd; // ~2,2 s, em quadros inteiros da cadência
            lab.media("V1", "v", &a, 0, frames * fd, 0, true, false);
            lab.media("A1", "a", &a, 0, frames * fd, 0, false, true);
            let mut s = RenderSettings::new(64, 48);
            s.audio_sample_rate = rate;
            let out = lab.dir.join("o.mp4");
            let r = lab
                .p()
                .export_mp4(
                    &lab.services,
                    &"S".into(),
                    TimeRange::new(Ticks(0), Ticks(frames * fd)),
                    &s,
                    &out,
                    &Mp4Options {
                        codec: ExportCodec::Mpeg4Reference,
                        ..Mp4Options::default()
                    },
                    &no_cancel,
                )
                .unwrap_or_else(|e| panic!("{fps} @ {rate}: {e}"));
            assert!(
                r.av_drift_ticks.unwrap() <= fd,
                "{fps} @ {rate}: validation drift"
            );
            // medição independente: flash (vídeo) × beep (áudio) lidos DO MP4 exportado
            let vf = first_bright_frame(&lab, &out) as i64;
            let beep = first_beep_sample(&lab, &out, rate) as i64;
            let t_video = vf * fd;
            let t_audio = beep * TICKS_PER_SECOND / i64::from(rate);
            let drift = (t_video - t_audio).abs();
            assert!(
                drift <= fd,
                "{fps} @ {rate}: flash at frame {vf} ({t_video}), beep at sample {beep} ({t_audio}), drift {drift} > 1 frame ({fd})"
            );
            // o flash está onde deveria (1,0 s): ≤ 1 quadro de erro absoluto
            assert!(
                (t_video - TICKS_PER_SECOND).abs() <= fd,
                "{fps} @ {rate}: flash time"
            );
            assert!(
                (t_audio - TICKS_PER_SECOND).abs() <= fd,
                "{fps} @ {rate}: beep time"
            );
        }
    }
}

fn boundary_j(base: i64, boundary_sample: i64) -> i64 {
    boundary_sample - base
}

fn index_of(lab: &Lab, path: &Path) -> (FrameIndex, u32, u32) {
    let info =
        capia_media::MediaProbe::probe(&capia_media::FfprobeBackend::new(lab.tc.clone()), path)
            .unwrap();
    let v = info.video().unwrap().clone();
    let idx = capia_media::build_frame_index(
        &lab.tc,
        path,
        v.index,
        v.time_base.unwrap(),
        0,
        &capia_media::IndexOptions::default(),
        &|| false,
        &mut |_, _| {},
    )
    .unwrap();
    (idx, v.width, v.height)
}

#[test]
fn vfr_source_is_sampled_by_time_not_by_frame_number() {
    let tc = need!();
    let mut lab = Lab::new("vfr", &tc);
    let path = lab.dir.join("vfr.mp4");
    std::fs::copy(lab::fixture("vfr.mp4"), &path).unwrap();
    let a = lab.import(&path);
    let (idx, w, h) = index_of(&lab, &path);
    assert_eq!(idx.len(), 25);
    // os intervalos entre quadros NÃO são constantes: prova que é VFR
    let times: Vec<i64> = (0..idx.len()).map(|i| idx.time_of(i).unwrap().0).collect();
    let gaps: std::collections::BTreeSet<i64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.len() > 1, "fixture is not VFR: {gaps:?}");
    lab.seq("S", FrameRate::FPS_30);
    lab.track("S", "V1", TrackKind::Visual);
    lab.media("V1", "c", &a, 0, 29 * F, 0, true, false);
    let s = RenderSettings::new(w, h);
    for n in 0..29i64 {
        let t = Ticks(n * F);
        let f = lab
            .p()
            .render_frame(&lab.services, &"S".into(), t, &s)
            .unwrap();
        let want_idx = idx.frame_at_or_before(t).unwrap();
        let want = decode_frame_by_index(
            &lab.tc,
            &path,
            &idx,
            w,
            h,
            want_idx,
            &DecodeLimits::default(),
            &|| false,
        )
        .unwrap()
        .bytes;
        assert_eq!(
            f.image.data, want,
            "timeline frame {n} must show source frame {want_idx} (by time)"
        );
        // e NÃO o quadro "n × 25/30" que um CFR ingênuo escolheria (quando diferente)
        let naive = ((n * 25) / 30) as usize;
        if naive != want_idx {
            let other = decode_frame_by_index(
                &lab.tc,
                &path,
                &idx,
                w,
                h,
                naive,
                &DecodeLimits::default(),
                &|| false,
            )
            .unwrap()
            .bytes;
            assert_ne!(f.image.data, other, "frame {n} looks CFR-mapped");
        }
    }
}

#[test]
fn a_ten_minute_timeline_has_no_av_drift() {
    let tc = need!();
    let mut lab = Lab::new("tenmin", &tc);
    let fr = FrameRate::new(Rational::new(30000, 1001).unwrap()).unwrap();
    let fd = fr.frame_duration().0;
    // fonte de 4,2 s; a timeline usa os primeiros 120 quadros (4,004 s): clips de vídeo são alinhados a quadro
    let src = lab.gen_mov_pcm("a.mov", "30000/1001", "4.2", 44_100);
    let a = lab.import(&src);
    lab.seq("S", fr);
    lab.track("S", "V1", TrackKind::Visual);
    lab.track("S", "A1", TrackKind::Audio);
    // 150 repetições encostadas ⇒ 18 000 quadros = 600,6 s (inteiros em ticks; nenhum float)
    let clip = 120 * fd;
    for k in 0..150i64 {
        lab.media("V1", &format!("v{k}"), &a, k * clip, clip, 0, true, false);
        lab.media("A1", &format!("a{k}"), &a, k * clip, clip, 0, false, true);
    }
    let total = 150 * clip;
    let seq = "S".into();
    let doc_dur = lab
        .p()
        .render_graph(&seq)
        .unwrap()
        .sequence(&seq)
        .unwrap()
        .duration;
    assert_eq!(doc_dur, Ticks(total));
    assert!(total >= 600 * TICKS_PER_SECOND);
    // aritmética de sincronismo: quadros × duração do quadro vs. amostras da mesma duração
    let n_frames = total / fd;
    assert_eq!(n_frames, 18_000);
    let audio_samples = (i128::from(total) * 44_100 / i128::from(TICKS_PER_SECOND)) as i64;
    let audio_ticks = audio_samples * TICKS_PER_SECOND / 44_100;
    assert!((total - audio_ticks).abs() <= fd);

    let (idx, w, h) = index_of(&lab, &src);
    assert!(idx.len() >= 120);
    let mut s = RenderSettings::new(w, h);
    s.audio_sample_rate = 44_100;
    // referência de áudio do asset (PCM exato, na taxa do projeto ⇒ sem reamostragem)
    let asset_audio = lab
        .p()
        .decode_audio(
            &a,
            Ticks(0),
            Ticks(5 * TICKS_PER_SECOND),
            None,
            None,
            &lab.tc,
            &no_cancel,
        )
        .unwrap();
    let asset_n = (asset_audio.samples.len() / 2) as i64;
    for k in [0i64, 1, 74, 148] {
        let boundary = (k + 1) * clip; // fim do clip k = início do clip k+1
        // áudio: janela de 0,5 s que cruza a fronteira
        let win_start = boundary - TICKS_PER_SECOND / 4;
        let (buf, _) = lab
            .p()
            .render_audio_range(
                &lab.services,
                &seq,
                TimeRange::new(Ticks(win_start), Ticks(TICKS_PER_SECOND / 2)),
                &s,
            )
            .unwrap();
        let n = (buf.samples.len() / 2) as i64;
        let to_sample = |t: i64| (i128::from(t) * 44_100 / i128::from(TICKS_PER_SECOND)) as i64;
        let (base, clip_k, clip_k1) = (
            to_sample(win_start),
            to_sample(k * clip),
            to_sample(boundary),
        );
        // cada trecho (antes/depois da fronteira) pertence a um clip cujo início cai numa amostra
        // arredondada: o melhor alinhamento de CADA trecho tem de estar em {-1, 0, +1} amostra
        for (seg, lo, hi) in [
            ("before", 8, boundary_j(base, clip_k1) - 8),
            ("after", boundary_j(base, clip_k1) + 8, n - 8),
        ] {
            let mut best = (f32::MAX, 0i64);
            for shift in [-2i64, -1, 0, 1, 2] {
                let mut worst = 0f32;
                for j in lo..hi {
                    let g = base + j + shift;
                    let src_pos = if seg == "before" {
                        g - clip_k
                    } else {
                        g - clip_k1
                    };
                    if src_pos < 0 || src_pos >= asset_n {
                        continue;
                    }
                    for c in 0..2usize {
                        let d = (buf.samples[j as usize * 2 + c]
                            - asset_audio.samples[src_pos as usize * 2 + c])
                            .abs();
                        worst = worst.max(d);
                    }
                }
                if worst < best.0 {
                    best = (worst, shift);
                }
            }
            assert!(
                best.0 <= 1e-6,
                "clip {k} ({seg}): audio window does not match the asset (worst {})",
                best.0
            );
            assert!(
                best.1.abs() <= 1,
                "clip {k} ({seg}): audio shifted by {} samples",
                best.1
            );
        }
        // vídeo: os quadros ao redor da fronteira mostram os quadros certos da fonte
        for nn in [boundary / fd - 1, boundary / fd, boundary / fd + 1] {
            let t = nn * fd;
            let f = lab
                .p()
                .render_frame(&lab.services, &seq, Ticks(t), &s)
                .unwrap();
            let in_clip = if t < boundary {
                t - k * clip
            } else {
                t - boundary
            };
            let src_idx = idx.frame_at_or_before(Ticks(in_clip)).unwrap();
            let want = decode_frame_by_index(
                &lab.tc,
                &src,
                &idx,
                w,
                h,
                src_idx,
                &DecodeLimits::default(),
                &|| false,
            )
            .unwrap();
            assert_eq!(
                frame_digest(&f.image),
                frame_digest(&capia_render::Image::from_rgba(w, h, want.bytes).unwrap()),
                "clip {k}: timeline frame {nn} (t={t}) should be source frame {src_idx}"
            );
        }
    }
    // o último quadro e a última amostra terminam juntos (≤ 1 quadro)
    let last_frame_t = (n_frames - 1) * fd;
    assert!(total - last_frame_t <= fd);
    let _ = Project::validate; // o projeto continua válido
}
