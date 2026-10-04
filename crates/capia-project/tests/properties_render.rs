//! Propriedade do render do projeto contra um ORÁCULO independente: timelines pequenas aleatórias
//! (posição, trim, velocidade, ordem de tracks, opacidade, transform, nested) renderizadas pelo
//! `Project` (decode service + cache + índice de áudio) têm de ser idênticas ao mesmo grafo
//! renderizado com decode direto do FFmpeg (sem serviço, sem cache) — em cache quente, depois de
//! reabrir (frio) e com caches minúsculos (evicção constante). Orçamento: `CAPIA_RENDER_PROP_CASES`
//! (padrão 6; Linux release no CI usa mais).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_media::{DecodeLimits, FrameIndex, decode_frame_by_index};
use capia_model::{AssetId, TrackKind};
use capia_project::{Image, RenderServices, SourceOptions};
use capia_render::{
    AudioBuffer, AudioRequest, MediaSource, RenderGraph, RenderSettings, SourceError, frame_digest,
    mix_audio_range, render_frame,
};
use capia_time::{FrameRate, Rational, TICKS_PER_SECOND, Ticks, TimeRange};
use lab::{F, Lab};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

struct Asset {
    id: AssetId,
    path: PathBuf,
    secs: i64,
    index: FrameIndex,
    w: u32,
    h: u32,
    audio: Vec<f32>,
}

/// Oráculo: decode direto, sem serviço, sem cache.
struct RefSource {
    tc: capia_media::MediaToolchain,
    assets: BTreeMap<String, Arc<Asset>>,
}

impl MediaSource for RefSource {
    fn video_frame(&self, asset: &AssetId, t: Ticks) -> Result<Option<Arc<Image>>, SourceError> {
        let a = &self.assets[asset.as_str()];
        let Some(i) = a.index.frame_at_or_before(t) else {
            return Ok(None);
        };
        let f = decode_frame_by_index(
            &self.tc,
            &a.path,
            &a.index,
            a.w,
            a.h,
            i,
            &DecodeLimits::default(),
            &|| false,
        )
        .map_err(|e| SourceError::new("REF", e.to_string()))?;
        Ok(Some(Arc::new(Image::from_rgba(a.w, a.h, f.bytes).unwrap())))
    }
    fn still_image(&self, _a: &AssetId) -> Result<Arc<Image>, SourceError> {
        Err(SourceError::new("REF", "no stills"))
    }
    fn audio(&self, asset: &AssetId, r: AudioRequest) -> Result<AudioBuffer, SourceError> {
        let a = &self.assets[asset.as_str()];
        let ch = r.channels as usize;
        let mut out = vec![0f32; r.frames as usize * ch];
        for j in 0..r.frames as usize {
            let g = r.start_sample as usize + j;
            if g * 2 + 1 < a.audio.len() {
                for c in 0..ch {
                    out[j * ch + c] = a.audio[g * 2 + c.min(1)];
                }
            }
        }
        Ok(AudioBuffer {
            sample_rate: r.sample_rate,
            channels: r.channels,
            samples: out,
        })
    }
}

fn load(lab: &mut Lab, name: &str, fps: &str, secs: u32) -> Arc<Asset> {
    let path = lab.gen_mov_pcm(name, fps, &secs.to_string(), 48_000);
    let id = lab.import(&path);
    let info =
        capia_media::MediaProbe::probe(&capia_media::FfprobeBackend::new(lab.tc.clone()), &path)
            .unwrap();
    let v = info.video().unwrap().clone();
    let index = capia_media::build_frame_index(
        &lab.tc,
        &path,
        v.index,
        v.time_base.unwrap(),
        0,
        &capia_media::IndexOptions::default(),
        &|| false,
        &mut |_, _| {},
    )
    .unwrap();
    let pcm = capia_media::decode_audio(
        &lab.tc,
        &path,
        &capia_media::AudioRequest {
            stream_index: info.audio().unwrap().index,
            sample_rate: 48_000,
            channels: 2,
            start: Ticks(0),
            duration: Ticks(i64::from(secs + 1) * TICKS_PER_SECOND),
        },
        capia_media::DEFAULT_MAX_PCM_BYTES,
        std::time::Duration::from_secs(60),
        &|| false,
    )
    .unwrap();
    Arc::new(Asset {
        id,
        path,
        secs: i64::from(secs),
        index,
        w: v.width,
        h: v.height,
        audio: pcm.samples,
    })
}

/// Preenche `track` com clips de vídeo aleatórios; devolve o fim ocupado.
fn fill_video(
    lab: &mut Lab,
    rng: &mut Rng,
    seq: &str,
    track: &str,
    tag: &str,
    assets: &[Arc<Asset>],
    max_frames: i64,
) {
    let mut at = rng.below(4) as i64;
    let mut k = 0;
    while at < max_frames && k < 3 {
        let a = &assets[rng.below(assets.len() as u64) as usize];
        let speed = rng.pick(&[(1i64, 1i64), (1, 1), (2, 1), (1, 2), (3, 2)]);
        let dur_frames = 3 + rng.below(10) as i64;
        let dur = dur_frames * F;
        let consumed = dur * speed.0 / speed.1;
        let asset_len = a.secs * TICKS_PER_SECOND;
        if consumed >= asset_len - F {
            break;
        }
        let max_in = (asset_len - consumed - F).max(0);
        let source_in = if max_in == 0 {
            0
        } else {
            (rng.below((max_in / F) as u64) as i64) * F
        };
        let id = format!("{tag}{k}");
        lab.clip(
            track,
            &id,
            capia_model::ClipContent::Media {
                asset: a.id.clone(),
                has_video: true,
                has_audio: false,
            },
            at * F,
            dur,
            source_in,
            Rational::new(speed.0, speed.1).unwrap(),
        );
        if rng.below(2) == 0 {
            lab.prop(&id, "opacity", rng.pick(&[0.25, 0.5, 0.75, 1.0]));
        }
        if rng.below(3) == 0 {
            lab.prop(&id, "scale", rng.pick(&[0.5, 0.75, 1.0, 1.5]));
            lab.prop(&id, "position_x", rng.pick(&[-10.0, 0.0, 7.0, 13.0]));
            lab.prop(&id, "position_y", rng.pick(&[-6.0, 0.0, 5.0]));
        }
        if rng.below(5) == 0 {
            lab.prop(&id, "rotation", rng.pick(&[90.0, 180.0, 270.0]));
        }
        at += dur_frames + rng.below(4) as i64;
        k += 1;
    }
    let _ = seq;
}

fn fill_audio(lab: &mut Lab, rng: &mut Rng, track: &str, assets: &[Arc<Asset>], max_frames: i64) {
    let mut at = rng.below(4) as i64;
    let mut k = 0;
    while at < max_frames && k < 2 {
        let a = &assets[rng.below(assets.len() as u64) as usize];
        let dur_frames = 5 + rng.below(12) as i64;
        let dur = dur_frames * F;
        let asset_len = a.secs * TICKS_PER_SECOND;
        let max_in = (asset_len - dur - F).max(0);
        let source_in = if max_in < F {
            0
        } else {
            (rng.below((max_in / F) as u64) as i64) * F
        };
        let id = format!("{track}c{k}");
        lab.media(track, &id, &a.id, at * F, dur, source_in, false, true);
        if rng.below(2) == 0 {
            lab.prop(&id, "volume_db", rng.pick(&[-6.0, -3.0, 0.0, 2.0]));
        }
        at += dur_frames + rng.below(6) as i64;
        k += 1;
    }
}

fn build_case(lab: &mut Lab, seed: u64, assets: &[Arc<Asset>]) -> i64 {
    let mut rng = Rng::new(seed);
    let total = 40i64;
    // sequence filha (nested) opcional
    let nested = rng.below(2) == 0;
    if nested {
        lab.seq("CHILD", FrameRate::FPS_30);
        lab.track("CHILD", "CV", TrackKind::Visual);
        fill_video(lab, &mut rng, "CHILD", "CV", "ch", assets, 18);
        lab.track("CHILD", "CA", TrackKind::Audio);
        fill_audio(lab, &mut rng, "CA", assets, 18);
    }
    lab.seq("S", FrameRate::FPS_30);
    let tracks = 1 + rng.below(3);
    for t in 0..tracks {
        let id = format!("V{t}");
        lab.track("S", &id, TrackKind::Visual);
        fill_video(lab, &mut rng, "S", &id, &format!("s{t}"), assets, total);
    }
    lab.track("S", "A0", TrackKind::Audio);
    fill_audio(lab, &mut rng, "A0", assets, total);
    if nested {
        lab.track("S", "VN", TrackKind::Visual);
        let at = rng.below(10) as i64;
        let dur = 6 + rng.below(10) as i64;
        lab.nested("VN", "nest", "CHILD", at * F, dur * F, 0);
        if rng.below(2) == 0 {
            lab.prop("nest", "opacity", 0.5);
        }
        if rng.below(2) == 0 {
            lab.prop("nest", "scale", 0.75);
        }
    }
    total
}

fn digests(
    graph: &RenderGraph,
    src: &dyn MediaSource,
    times: &[Ticks],
    s: &RenderSettings,
) -> Vec<String> {
    let seq = "S".into();
    times
        .iter()
        .map(|t| frame_digest(&render_frame(graph, &seq, *t, s, src).unwrap().image))
        .collect()
}

#[test]
fn project_render_equals_the_direct_decode_oracle_on_random_timelines() {
    let tc = need!();
    let cases: u64 = std::env::var("CAPIA_RENDER_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let base: u64 = std::env::var("CAPIA_RENDER_PROP_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let black = frame_digest(&Image::filled(80, 60, [0, 0, 0, 255]).unwrap());
    let (mut non_black, mut audible) = (0usize, 0usize);
    for case in 0..cases {
        let seed = base * 1000 + case;
        let mut lab = Lab::new(&format!("prop{case}"), &tc);
        let assets = vec![
            load(&mut lab, "a.mov", "30", 3),
            load(&mut lab, "b.mov", "25", 2),
        ];
        let total = build_case(&mut lab, seed, &assets);
        let seq = "S".into();
        let s = RenderSettings::new(80, 60);
        let oracle = RefSource {
            tc: lab.tc.clone(),
            assets: assets
                .iter()
                .map(|a| (a.id.as_str().to_owned(), Arc::clone(a)))
                .collect(),
        };
        let graph = lab.p().render_graph(&seq).unwrap();
        let mut rng = Rng::new(seed ^ 0xABCD);
        let times: Vec<Ticks> = (0..7)
            .map(|_| Ticks(rng.below(total as u64) as i64 * F))
            .collect();
        let want = digests(&graph, &oracle, &times, &s);
        let audio_range = TimeRange::new(Ticks(rng.below(10) as i64 * F), Ticks(20 * F));
        let (want_audio, _) = mix_audio_range(&graph, &seq, audio_range, &s, &oracle).unwrap();

        non_black += want.iter().filter(|d| **d != black).count();
        audible += usize::from(want_audio.samples.iter().any(|x| x.abs() > 1e-3));
        // 1) cache quente (mesmos serviços, 2 passadas)
        for pass in 0..2 {
            let src = lab
                .p()
                .render_source(&lab.services, &graph, SourceOptions::default())
                .unwrap();
            assert_eq!(
                digests(&graph, &src, &times, &s),
                want,
                "seed {seed} warm pass {pass}"
            );
        }
        let (got_audio, _) = lab
            .p()
            .render_audio_range(&lab.services, &seq, audio_range, &s)
            .unwrap();
        assert_eq!(got_audio.samples, want_audio.samples, "seed {seed} audio");
        // 2) depois de salvar/reabrir (frio)
        lab.reopen();
        let graph = lab.p().render_graph(&seq).unwrap();
        let src = lab
            .p()
            .render_source(&lab.services, &graph, SourceOptions::default())
            .unwrap();
        assert_eq!(
            digests(&graph, &src, &times, &s),
            want,
            "seed {seed} after reopen"
        );
        // 3) caches minúsculos, 1 sessão: evicção e reabertura constantes
        let mut cfg = capia_decode::DecodeConfig::new(lab.tc.clone());
        cfg.frame_cache_bytes = 2 * 80 * 60 * 4;
        cfg.max_sessions = 1;
        let tiny = Arc::new(RenderServices::with_config(cfg, 1 << 18));
        let src = lab
            .p()
            .render_source(&tiny, &graph, SourceOptions::default())
            .unwrap();
        assert_eq!(
            digests(&graph, &src, &times, &s),
            want,
            "seed {seed} tiny caches"
        );
        let (tiny_audio, _) = lab
            .p()
            .render_audio_range(&tiny, &seq, audio_range, &s)
            .unwrap();
        assert_eq!(
            tiny_audio.samples, want_audio.samples,
            "seed {seed} tiny audio"
        );
    }
    // o gerador produz conteúdo de verdade (não só preto/silêncio)
    assert!(
        non_black >= cases as usize,
        "random timelines were mostly empty ({non_black} non-black frames)"
    );
    assert!(audible >= 1, "no random timeline had audible audio");
}
