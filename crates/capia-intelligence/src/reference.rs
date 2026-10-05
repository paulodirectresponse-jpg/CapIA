//! Reference Analyzer: transforma um vídeo de referência numa **`ReferenceGrammar`** — dados
//! estruturais (ritmo de cortes, transições, energia do áudio, densidade de fala, estrutura
//! hook/corpo/CTA) — nunca no conteúdo protegido. É determinística (mesma mídia + parâmetros ⇒ mesmo
//! JSON, mesmo digest), local-first (cenas e áudio rodam sem rede) e persistida como registro
//! derivado com proveniência. A transcrição é opcional: sem provider, a gramática sai sem a seção
//! de fala (com a nota `no_stt_available`).

use crate::ctx::IntelCtx;
use crate::error::{IntelError, IntelResult};
use crate::records::{self, KIND_REFERENCE, now_ms};
use crate::scenes::{self, BoundaryKind, SceneParams};
use crate::silence;
use crate::transcript::{TranscribeParams, asset_file, transcribe_asset, us_from_ticks};
use capia_ai::dispatcher::TaskCtx;
use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const GRAMMAR_SCHEMA_VERSION: u32 = 1;
pub const PRODUCER: &str = "capia-reference-analyzer/1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceOptions {
    /// Taxa de amostragem da varredura de cenas (quadros/s, inteira).
    pub scan_fps: u32,
    /// Janela do gancho (µs).
    pub hook_us: i64,
    /// Fração final do vídeo tratada como CTA.
    pub cta_fraction_permille: u32,
    /// Tenta transcrever (se houver STT roteável). Falha de roteamento não derruba a análise.
    pub with_transcript: bool,
    pub silence_threshold_db: f32,
    pub scene: SceneParams,
    pub force: bool,
}

impl Default for ReferenceOptions {
    fn default() -> Self {
        Self {
            scan_fps: 25,
            hook_us: 3_000_000,
            cta_fraction_permille: 150,
            with_transcript: true,
            silence_threshold_db: -40.0,
            scene: SceneParams::default(),
            force: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub start_us: i64,
    pub end_us: i64,
    /// Como o plano começa: `start`, `cut`, `dissolve` ou `fade`.
    pub entry: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutRhythm {
    pub shot_count: usize,
    pub cuts_per_minute: f64,
    pub mean_shot_us: i64,
    pub median_shot_us: i64,
    pub p10_shot_us: i64,
    pub p90_shot_us: i64,
    pub shortest_us: i64,
    pub longest_us: i64,
    /// Contagem de planos por faixa: <0,5 s, 0,5–1 s, 1–2 s, 2–4 s, ≥4 s.
    pub histogram: [u32; 5],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionMix {
    pub cuts: u32,
    pub dissolves: u32,
    pub fades: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioProfile {
    pub speech_ratio_permille: u32,
    pub silence_ratio_permille: u32,
    pub mean_db: f32,
    pub peak_db: f32,
    /// p95 − p10 (dB) sobre as janelas de 20 ms.
    pub dynamic_range_db: f32,
    /// Loudness médio por janela de 10 s.
    pub db_per_10s: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeechProfile {
    pub language: Option<String>,
    pub word_count: usize,
    pub words_per_minute: f64,
    pub first_word_us: Option<i64>,
    pub hook_text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beat {
    pub name: String,
    pub start_us: i64,
    pub end_us: i64,
    pub shots: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub producer: String,
    pub asset_id: String,
    pub params_digest: String,
    pub created_ms: u64,
    /// `true` se nenhum dado saiu da máquina para produzir a gramática.
    pub fully_local: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceGrammar {
    pub schema_version: u32,
    pub duration_us: i64,
    pub scan_fps: u32,
    pub shots: Vec<Shot>,
    pub cut_rhythm: CutRhythm,
    pub transitions: TransitionMix,
    pub audio: Option<AudioProfile>,
    pub speech: Option<SpeechProfile>,
    pub structure: Vec<Beat>,
    pub provenance: Provenance,
}

impl ReferenceGrammar {
    /// Digest do conteúdo estrutural (exclui carimbo de tempo): prova de determinismo.
    pub fn content_digest(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(p) = v.get_mut("provenance").and_then(|p| p.as_object_mut()) {
            p.remove("created_ms");
        }
        let mut h = Sha256::new();
        h.update(capia_ai::types::canonical_json(&v).as_bytes());
        capia_ai::types::hex(&h.finalize())
    }
}

fn percentile(sorted: &[i64], p: usize) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(sorted.len() - 1) * p / 100]
}

/// Planos a partir das fronteiras de cena (tempos em µs, no relógio de varredura).
pub fn shots_from_boundaries(
    boundaries: &[scenes::Boundary],
    scan_fps: u32,
    duration_us: i64,
) -> Vec<Shot> {
    let fps = i64::from(scan_fps.max(1));
    let at = |frame: u64| (frame as i64) * 1_000_000 / fps;
    let mut shots: Vec<Shot> = Vec::new();
    let mut start = 0i64;
    let mut entry = "start".to_owned();
    for b in boundaries {
        let t = at(b.frame).clamp(0, duration_us);
        if t > start {
            shots.push(Shot {
                start_us: start,
                end_us: t,
                entry: entry.clone(),
            });
            start = t;
        }
        entry = match b.kind {
            BoundaryKind::Cut => "cut",
            BoundaryKind::Dissolve => "dissolve",
            BoundaryKind::Fade => "fade",
        }
        .to_owned();
    }
    if duration_us > start {
        shots.push(Shot {
            start_us: start,
            end_us: duration_us,
            entry,
        });
    }
    shots
}

pub fn rhythm(shots: &[Shot], duration_us: i64) -> CutRhythm {
    let mut lens: Vec<i64> = shots.iter().map(|s| s.end_us - s.start_us).collect();
    lens.sort_unstable();
    let n = lens.len();
    let sum: i64 = lens.iter().sum();
    let mut histogram = [0u32; 5];
    for l in &lens {
        let i = match *l {
            x if x < 500_000 => 0,
            x if x < 1_000_000 => 1,
            x if x < 2_000_000 => 2,
            x if x < 4_000_000 => 3,
            _ => 4,
        };
        histogram[i] += 1;
    }
    let minutes = duration_us.max(1) as f64 / 60_000_000.0;
    CutRhythm {
        shot_count: n,
        cuts_per_minute: if n > 1 {
            ((n - 1) as f64 / minutes * 100.0).round() / 100.0
        } else {
            0.0
        },
        mean_shot_us: if n > 0 { sum / n as i64 } else { 0 },
        median_shot_us: percentile(&lens, 50),
        p10_shot_us: percentile(&lens, 10),
        p90_shot_us: percentile(&lens, 90),
        shortest_us: lens.first().copied().unwrap_or(0),
        longest_us: lens.last().copied().unwrap_or(0),
        histogram,
    }
}

pub fn audio_profile(db: &[f32], silence_threshold_db: f32) -> Option<AudioProfile> {
    if db.is_empty() {
        return None;
    }
    let silent = db.iter().filter(|d| **d < silence_threshold_db).count();
    let mean = db.iter().map(|d| f64::from(*d)).sum::<f64>() / db.len() as f64;
    let mut sorted: Vec<f32> = db.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let q = |p: usize| sorted[(sorted.len() - 1) * p / 100];
    let per10 = (10_000_000 / silence::WINDOW_US) as usize;
    let db_per_10s = db
        .chunks(per10)
        .map(|c| (c.iter().map(|d| f64::from(*d)).sum::<f64>() / c.len() as f64) as f32)
        .map(|v| (v * 10.0).round() / 10.0)
        .collect();
    let permille = |n: usize| (n * 1000 / db.len()) as u32;
    Some(AudioProfile {
        speech_ratio_permille: permille(db.len() - silent),
        silence_ratio_permille: permille(silent),
        mean_db: ((mean * 10.0).round() / 10.0) as f32,
        peak_db: sorted.last().copied().unwrap_or(-120.0),
        dynamic_range_db: ((q(95) - q(10)) * 10.0).round() / 10.0,
        db_per_10s,
    })
}

pub fn speech_profile(t: &capia_ai::stt::Transcript, hook_us: i64) -> SpeechProfile {
    let mut words: Vec<&capia_ai::stt::Word> = t.segments.iter().flat_map(|s| &s.words).collect();
    let word_count = if words.is_empty() {
        t.segments
            .iter()
            .map(|s| s.text.split_whitespace().count())
            .sum()
    } else {
        words.len()
    };
    words.sort_by_key(|w| w.start_us);
    let first = words
        .first()
        .map(|w| w.start_us)
        .or_else(|| t.segments.first().map(|s| s.start_us));
    let dur = t.duration_us.unwrap_or(0).max(1);
    let hook_text: String = if words.is_empty() {
        t.segments
            .iter()
            .filter(|s| s.start_us < hook_us)
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        words
            .iter()
            .filter(|w| w.start_us < hook_us)
            .map(|w| w.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    };
    SpeechProfile {
        language: t.language.clone(),
        word_count,
        words_per_minute: (word_count as f64 / (dur as f64 / 60_000_000.0) * 10.0).round() / 10.0,
        first_word_us: first,
        hook_text,
    }
}

/// Hook / corpo / CTA por janelas fixas (estrutura de DR: o gancho prende, o CTA fecha).
pub fn structure(shots: &[Shot], duration_us: i64, o: &ReferenceOptions) -> Vec<Beat> {
    if duration_us <= 0 {
        return Vec::new();
    }
    let hook_end = o.hook_us.min(duration_us);
    let cta_start = (duration_us - duration_us * i64::from(o.cta_fraction_permille) / 1000)
        .max(hook_end)
        .min(duration_us);
    let count = |a: i64, b: i64| {
        shots
            .iter()
            .filter(|s| s.start_us >= a && s.start_us < b)
            .count() as u32
    };
    let mut v = vec![Beat {
        name: "hook".into(),
        start_us: 0,
        end_us: hook_end,
        shots: count(0, hook_end),
    }];
    if cta_start > hook_end {
        v.push(Beat {
            name: "body".into(),
            start_us: hook_end,
            end_us: cta_start,
            shots: count(hook_end, cta_start),
        });
    }
    if duration_us > cta_start {
        v.push(Beat {
            name: "cta".into(),
            start_us: cta_start,
            end_us: duration_us,
            shots: count(cta_start, duration_us),
        });
    }
    v
}

pub fn transition_mix(boundaries: &[scenes::Boundary]) -> TransitionMix {
    let mut m = TransitionMix {
        cuts: 0,
        dissolves: 0,
        fades: 0,
    };
    for b in boundaries {
        match b.kind {
            BoundaryKind::Cut => m.cuts += 1,
            BoundaryKind::Dissolve => m.dissolves += 1,
            BoundaryKind::Fade => m.fades += 1,
        }
    }
    m
}

/// Analisa uma referência (local-first). `progress(etapa, total)`.
pub async fn analyze_reference(
    ctx: &IntelCtx,
    task: &TaskCtx,
    asset_id: &str,
    o: &ReferenceOptions,
    progress: &(dyn Fn(&str, u32, u32) + Send + Sync),
) -> IntelResult<(ReferenceGrammar, bool)> {
    let file = asset_file(ctx, asset_id)?;
    if !file.has_video {
        return Err(IntelError::new(
            "NO_VIDEO",
            "the reference asset has no video stream",
        ));
    }
    let tc = ctx
        .engine
        .toolchain()
        .ok_or_else(|| IntelError::new("FFMPEG_NOT_FOUND", "ffmpeg is not available"))?;
    // `force` só controla o cache: não faz parte da identidade do resultado
    let mut keyed = o.clone();
    keyed.force = false;
    let params_digest =
        records::key(&[&serde_json::to_string(&keyed).unwrap_or_default(), PRODUCER]);
    let id = format!("ref_{}", records::key(&[asset_id, &params_digest]));
    if !o.force
        && let Some(g) = ctx
            .records()?
            .latest::<ReferenceGrammar>(KIND_REFERENCE, &id)?
    {
        return Ok((g, true));
    }
    let duration_us = us_from_ticks(file.duration).max(1);
    let mut notes: Vec<String> = Vec::new();

    progress("scenes", 0, 3);
    let boundaries = {
        let (tc, path, params, cancel) =
            (tc.clone(), file.path.clone(), o.scene, task.cancel.clone());
        let fps = o.scan_fps.clamp(5, 60);
        tokio::task::spawn_blocking(move || {
            scenes::detect_media(
                &tc,
                &path,
                0,
                (fps, 1),
                &params,
                Duration::from_secs(1800),
                &move || cancel.is_cancelled(),
            )
        })
        .await
        .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??
    };
    if task.cancel.is_cancelled() {
        return Err(IntelError::cancelled());
    }
    let shots = shots_from_boundaries(&boundaries, o.scan_fps.clamp(5, 60), duration_us);

    progress("audio", 1, 3);
    let audio = if file.has_audio {
        let db = {
            let (ctx2, aid, cancel) = (ctx.clone(), asset_id.to_owned(), task.cancel.clone());
            let dur = file.duration;
            tokio::task::spawn_blocking(move || {
                silence::measure_asset(&ctx2, &aid, Ticks::ZERO, dur, &cancel)
            })
            .await
            .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??
        };
        audio_profile(&db, o.silence_threshold_db)
    } else {
        notes.push("no_audio".into());
        None
    };

    progress("speech", 2, 3);
    let mut fully_local = true;
    let speech = if o.with_transcript && file.has_audio {
        match transcribe_asset(ctx, task, &TranscribeParams::new(asset_id), &|_, _| {}).await {
            Ok(rec) => {
                fully_local = rec.local;
                Some(speech_profile(&rec.transcript, o.hook_us))
            }
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => {
                notes.push(
                    if e.code == "NO_CAPABLE_MODEL" || e.code == "NOT_CONFIGURED" {
                        "no_stt_available".to_owned()
                    } else {
                        format!("stt_failed:{}", e.code)
                    },
                );
                None
            }
        }
    } else {
        None
    };
    progress("done", 3, 3);

    let g = ReferenceGrammar {
        schema_version: GRAMMAR_SCHEMA_VERSION,
        duration_us,
        scan_fps: o.scan_fps.clamp(5, 60),
        cut_rhythm: rhythm(&shots, duration_us),
        transitions: transition_mix(&boundaries),
        structure: structure(&shots, duration_us, o),
        shots,
        audio,
        speech,
        provenance: Provenance {
            producer: PRODUCER.into(),
            asset_id: asset_id.to_owned(),
            params_digest,
            created_ms: now_ms(),
            fully_local,
            notes,
        },
    };
    ctx.records()?
        .put(KIND_REFERENCE, &id, 1, Some(asset_id), &g)?;
    Ok((g, false))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::scenes::Boundary;

    fn cut(frame: u64) -> Boundary {
        Boundary {
            frame,
            kind: BoundaryKind::Cut,
            span: 1,
            score: 1.0,
        }
    }

    #[test]
    fn shots_cover_the_whole_duration_without_gaps() {
        let b = vec![
            cut(25),
            cut(75),
            Boundary {
                frame: 100,
                kind: BoundaryKind::Dissolve,
                span: 10,
                score: 1.0,
            },
        ];
        let shots = shots_from_boundaries(&b, 25, 6_000_000);
        assert_eq!(shots.first().unwrap().start_us, 0);
        assert_eq!(shots.last().unwrap().end_us, 6_000_000);
        for w in shots.windows(2) {
            assert_eq!(w[0].end_us, w[1].start_us);
        }
        assert_eq!(shots[1].entry, "cut");
        assert_eq!(shots[3].entry, "dissolve");
    }

    #[test]
    fn rhythm_statistics_are_exact() {
        let shots: Vec<Shot> = [1_000_000i64, 2_000_000, 3_000_000, 500_000]
            .iter()
            .scan(0i64, |t, l| {
                let s = *t;
                *t += l;
                Some(Shot {
                    start_us: s,
                    end_us: *t,
                    entry: "cut".into(),
                })
            })
            .collect();
        let r = rhythm(&shots, 6_500_000);
        assert_eq!(r.shot_count, 4);
        assert_eq!(r.shortest_us, 500_000);
        assert_eq!(r.longest_us, 3_000_000);
        assert_eq!(r.mean_shot_us, 1_625_000);
        assert_eq!(r.histogram, [0, 1, 1, 2, 0]);
        // 3 cortes em 6,5 s = 27,69 cortes/min
        assert!(
            (r.cuts_per_minute - 27.69).abs() < 0.01,
            "{}",
            r.cuts_per_minute
        );
    }

    #[test]
    fn audio_profile_separates_speech_from_silence() {
        let mut db = vec![-12.0f32; 300];
        db.extend(vec![-90.0f32; 200]);
        let a = audio_profile(&db, -40.0).unwrap();
        assert_eq!(a.speech_ratio_permille, 600);
        assert_eq!(a.silence_ratio_permille, 400);
        assert!(a.dynamic_range_db > 60.0);
        assert_eq!(a.db_per_10s.len(), 1);
        assert!(audio_profile(&[], -40.0).is_none());
    }

    #[test]
    fn structure_is_hook_body_cta_and_tiles_the_duration() {
        let o = ReferenceOptions::default();
        let shots = shots_from_boundaries(&[cut(25), cut(100), cut(200)], 25, 10_000_000);
        let s = structure(&shots, 10_000_000, &o);
        assert_eq!(
            s.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            ["hook", "body", "cta"]
        );
        assert_eq!(s[0].start_us, 0);
        assert_eq!(s.last().unwrap().end_us, 10_000_000);
        for w in s.windows(2) {
            assert_eq!(w[0].end_us, w[1].start_us);
        }
        // vídeo mais curto que o gancho
        assert_eq!(structure(&[], 2_000_000, &o).len(), 1);
    }

    #[test]
    fn digest_ignores_the_timestamp_only() {
        let o = ReferenceOptions::default();
        let shots = shots_from_boundaries(&[cut(50)], 25, 4_000_000);
        let mk = |t: u64| ReferenceGrammar {
            schema_version: 1,
            duration_us: 4_000_000,
            scan_fps: 25,
            cut_rhythm: rhythm(&shots, 4_000_000),
            transitions: transition_mix(&[cut(50)]),
            structure: structure(&shots, 4_000_000, &o),
            shots: shots.clone(),
            audio: None,
            speech: None,
            provenance: Provenance {
                producer: PRODUCER.into(),
                asset_id: "a".into(),
                params_digest: "p".into(),
                created_ms: t,
                fully_local: true,
                notes: vec![],
            },
        };
        assert_eq!(mk(1).content_digest(), mk(999).content_digest());
        let mut other = mk(1);
        other.duration_us += 1;
        assert_ne!(mk(1).content_digest(), other.content_digest());
    }
}
