//! Detecção local de silêncio (energia RMS por janela de 20 ms, determinística, sem rede) e plano
//! de remoção como comandos comuns (`split_clip` + `delete_clip` com ripple), via `preview →
//! apply_plan`. A remoção nunca corta fala: as bordas do corte são alinhadas ao frame **para dentro**
//! do silêncio e preservam um respiro (`pad_us`) em cada lado.

use crate::ctx::IntelCtx;
use crate::error::{IntelError, IntelResult};
use crate::transcript::{asset_file, ticks_from_us, us_from_ticks};
use capia_ai::CancelToken;
use capia_media::{FfprobeBackend, Flow, MediaProbe, decode_pcm_s16_mono};
use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

pub const SAMPLE_RATE: u32 = 16_000;
/// 20 ms a 16 kHz.
const WINDOW: usize = 320;
pub const WINDOW_US: i64 = 20_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SilenceParams {
    /// Abaixo disto (dBFS RMS) a janela é silêncio.
    pub threshold_db: f32,
    /// Silêncio mínimo para virar corte.
    pub min_silence_us: i64,
    /// Respiro preservado de cada lado do silêncio.
    pub pad_us: i64,
}

impl Default for SilenceParams {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            min_silence_us: 500_000,
            pad_us: 120_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start_us: i64,
    pub end_us: i64,
}

/// RMS em dBFS de blocos de 20 ms (`-120` para silêncio digital).
#[derive(Debug, Default)]
pub struct RmsMeter {
    acc: f64,
    n: usize,
    pub db: Vec<f32>,
}

impl RmsMeter {
    pub fn push(&mut self, samples: &[i16]) {
        for s in samples {
            let v = f64::from(*s) / 32768.0;
            self.acc += v * v;
            self.n += 1;
            if self.n == WINDOW {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        let rms = (self.acc / self.n.max(1) as f64).sqrt();
        self.db.push(if rms <= 1e-6 {
            -120.0
        } else {
            (20.0 * rms.log10()) as f32
        });
        self.acc = 0.0;
        self.n = 0;
    }

    pub fn finish(mut self) -> Vec<f32> {
        if self.n >= WINDOW / 2 {
            self.flush();
        }
        self.db
    }
}

/// Faixas de silêncio (tempo relativo ao início da análise), já sem o respiro de cada lado.
pub fn find_silences(db: &[f32], p: &SilenceParams) -> Vec<Range> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < db.len() {
        if db[i] < p.threshold_db {
            let s = i;
            while i < db.len() && db[i] < p.threshold_db {
                i += 1;
            }
            let (a, b) = (s as i64 * WINDOW_US, i as i64 * WINDOW_US);
            if b - a >= p.min_silence_us {
                let (a, b) = (a + p.pad_us, b - p.pad_us);
                if b > a {
                    out.push(Range {
                        start_us: a,
                        end_us: b,
                    });
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Mede o áudio de `[start, start+duration)` do asset (local; cancelável).
pub fn measure_asset(
    ctx: &IntelCtx,
    asset_id: &str,
    start: Ticks,
    duration: Ticks,
    cancel: &CancelToken,
) -> IntelResult<Vec<f32>> {
    let file = asset_file(ctx, asset_id)?;
    let tc = ctx
        .engine
        .toolchain()
        .ok_or_else(|| IntelError::new("FFMPEG_NOT_FOUND", "ffmpeg is not available"))?;
    let info = FfprobeBackend::new(tc.clone()).probe(&file.path)?;
    let idx = info
        .audio()
        .map(|a| a.index)
        .ok_or_else(|| IntelError::new("NO_AUDIO", "the asset has no audio stream"))?;
    let mut meter = RmsMeter::default();
    let c = cancel.clone();
    decode_pcm_s16_mono(
        &tc,
        &file.path,
        idx,
        start,
        duration,
        SAMPLE_RATE,
        Duration::from_secs(1800),
        &move || c.is_cancelled(),
        &mut |s| {
            meter.push(s);
            Ok(Flow::Continue)
        },
    )?;
    if cancel.is_cancelled() {
        return Err(IntelError::cancelled());
    }
    Ok(meter.finish())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cut {
    pub clip: String,
    /// Em tempo da timeline.
    pub start: Ticks,
    pub end: Ticks,
}

/// Cortes (timeline) de um clip, em ordem **decrescente** de tempo (para que os índices anteriores
/// não se desloquem durante a transação). `silences` em µs do asset.
pub fn cuts_for_clip(
    clip_id: &str,
    clip_start: Ticks,
    clip_dur: Ticks,
    source_in: Ticks,
    speed: (i64, i64),
    silences: &[Range],
    frame: i64,
) -> Vec<Cut> {
    let frame = frame.max(1);
    let (n, d) = (speed.0.max(1), speed.1.max(1));
    let win_start = us_from_ticks(source_in);
    let content = i128::from(clip_dur.0) * i128::from(n) / i128::from(d);
    let win_end = win_start + us_from_ticks(Ticks(content as i64));
    let to_tl = |us: i64| -> i64 {
        clip_start.0
            + (i128::from(ticks_from_us(us - win_start).0) * i128::from(d) / i128::from(n)) as i64
    };
    let clip_end = clip_start.0 + clip_dur.0;
    let mut cuts: Vec<Cut> = Vec::new();
    for r in silences {
        let (s, e) = (r.start_us.max(win_start), r.end_us.min(win_end));
        if e <= s {
            continue;
        }
        // alinha PARA DENTRO do silêncio: início sobe ao próximo frame, fim desce ao anterior
        let a = (to_tl(s) + frame - 1).div_euclid(frame) * frame;
        let b = to_tl(e).div_euclid(frame) * frame;
        let (a, b) = (a.max(clip_start.0 + frame), b.min(clip_end - frame));
        if b - a >= 2 * frame {
            cuts.push(Cut {
                clip: clip_id.to_owned(),
                start: Ticks(a),
                end: Ticks(b),
            });
        }
    }
    cuts.sort_by_key(|c| std::cmp::Reverse(c.start));
    cuts
}

/// Comandos: para cada corte (do fim para o começo) `split` no início, `split` no fim e `delete`
/// com ripple do trecho do meio. Ids das metades direitas são determinísticos.
pub fn cut_commands(key: &str, cuts: &[Cut], ripple_scope: &str) -> Value {
    let mut cmds: Vec<Value> = Vec::new();
    // cada clip original perde o "resto à direita" a cada split: como processamos de trás para
    // frente, o clip original sempre é o que contém o início do corte
    for (i, c) in cuts.iter().enumerate() {
        let mid = format!("sil_{key}_{i}_m");
        let tail = format!("sil_{key}_{i}_t");
        cmds.push(json!({
            "operation_id": format!("{key}:s1:{i}"),
            "type": "split_clip",
            "clip": c.clip,
            "at": c.start,
            "new_id": mid,
        }));
        cmds.push(json!({
            "operation_id": format!("{key}:s2:{i}"),
            "type": "split_clip",
            "clip": mid,
            "at": c.end,
            "new_id": tail,
        }));
        let scope = if ripple_scope == "sequence" {
            json!({ "type": "sequence" })
        } else {
            json!({ "type": "track" })
        };
        cmds.push(json!({
            "operation_id": format!("{key}:del:{i}"),
            "type": "delete_clip",
            "clip": mid,
            "ripple": true,
            "scope": scope,
        }));
    }
    Value::Array(cmds)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SilencePlan {
    pub cut_count: usize,
    pub removed_us: i64,
    pub plan_token: Option<String>,
    pub preview: Value,
    pub silences: Vec<Range>,
}

/// Mede o áudio do clip, acha os silêncios e faz o **preview** do corte (nada gravado).
pub fn plan_silence_cut(
    ctx: &IntelCtx,
    asset_id: &str,
    sequence: &str,
    p: &SilenceParams,
    ripple_scope: &str,
    task_id: &str,
    only_clip: Option<&str>,
    cancel: &CancelToken,
) -> IntelResult<SilencePlan> {
    let seq = ctx
        .engine
        .read("sequence.get", json!({ "sequence": sequence }))?;
    let frame = seq["frame_ticks"].as_i64().unwrap_or(0);
    if frame <= 0 {
        return Err(IntelError::new(
            "SEQUENCE_INVALID",
            "the sequence has no frame rate",
        ));
    }
    let mut clips = crate::captions::clips_of_asset(&seq, asset_id);
    if let Some(only) = only_clip {
        clips.retain(|c| c.clip_id == only);
    }
    if clips.is_empty() {
        return Err(IntelError::new(
            "NO_CLIPS",
            "the asset is not used by any enabled clip in this sequence",
        ));
    }
    let mut all_cuts: Vec<Cut> = Vec::new();
    let mut silences_out: Vec<Range> = Vec::new();
    for c in &clips {
        let content = Ticks(
            (i128::from(c.duration.0) * i128::from(c.speed_num) / i128::from(c.speed_den.max(1)))
                as i64,
        );
        let db = measure_asset(ctx, asset_id, c.source_in, content, cancel)?;
        let base = us_from_ticks(c.source_in);
        let sil: Vec<Range> = find_silences(&db, p)
            .into_iter()
            .map(|r| Range {
                start_us: r.start_us + base,
                end_us: r.end_us + base,
            })
            .collect();
        silences_out.extend(sil.iter().copied());
        all_cuts.extend(cuts_for_clip(
            &c.clip_id,
            c.start,
            c.duration,
            c.source_in,
            (c.speed_num, c.speed_den),
            &sil,
            frame,
        ));
    }
    if all_cuts.is_empty() {
        return Err(IntelError::new(
            "NO_SILENCE",
            "no silence longer than the configured minimum was found",
        ));
    }
    // ordem global decrescente por (clip, início): dentro de cada clip é o que importa; clips
    // diferentes não se afetam (ripple só na track, por padrão)
    all_cuts.sort_by(|a, b| a.clip.cmp(&b.clip).then(b.start.cmp(&a.start)));
    let removed: i64 = all_cuts
        .iter()
        .map(|c| us_from_ticks(Ticks(c.end.0 - c.start.0)))
        .sum();
    let key = crate::records::key(&[task_id, sequence, asset_id, "silence"]);
    let cmds = cut_commands(&key[..12], &all_cuts, ripple_scope);
    let preview = ctx.engine.preview(&ctx.actor, "Remover silêncios", cmds)?;
    Ok(SilencePlan {
        cut_count: all_cuts.len(),
        removed_us: removed,
        plan_token: preview["plan_token"].as_str().map(str::to_owned),
        preview,
        silences: silences_out,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn tone(n: usize, amp: f64) -> Vec<i16> {
        (0..n)
            .map(|i| ((i as f64 * 0.3).sin() * amp * 32767.0) as i16)
            .collect()
    }

    #[test]
    fn meter_measures_dbfs_per_20ms_window() {
        let mut m = RmsMeter::default();
        m.push(&tone(16_000, 0.5)); // 1 s
        m.push(&vec![0i16; 16_000]);
        let db = m.finish();
        assert_eq!(db.len(), 100);
        assert!(db[10] > -12.0 && db[10] < -5.0, "{}", db[10]);
        assert!(db[75] <= -100.0);
    }

    #[test]
    fn short_pauses_are_kept_long_ones_cut_with_padding() {
        let p = SilenceParams::default();
        // 0.5 s fala, 0.3 s pausa (curta), 0.5 s fala, 1.2 s silêncio, 0.5 s fala
        let mut db = vec![-10.0f32; 25];
        db.extend(vec![-90.0; 15]);
        db.extend(vec![-10.0; 25]);
        db.extend(vec![-90.0; 60]);
        db.extend(vec![-10.0; 25]);
        let s = find_silences(&db, &p);
        assert_eq!(s.len(), 1, "{s:?}");
        // silêncio de 65*20ms=1.3 s a partir de 1.3 s → respiro de 120 ms em cada lado
        assert_eq!(s[0].start_us, 65 * 20_000 + 120_000);
        assert_eq!(s[0].end_us, 125 * 20_000 - 120_000);
    }

    #[test]
    fn cuts_are_inside_the_silence_frame_aligned_and_descending() {
        let frame = 23_520_000i64; // 30 fps
        let sil = vec![
            Range {
                start_us: 1_000_000,
                end_us: 2_000_000,
            },
            Range {
                start_us: 4_000_000,
                end_us: 5_500_000,
            },
        ];
        let cuts = cuts_for_clip(
            "c",
            Ticks(0),
            Ticks(8 * capia_time::TICKS_PER_SECOND),
            Ticks(0),
            (1, 1),
            &sil,
            frame,
        );
        assert_eq!(cuts.len(), 2);
        assert!(cuts[0].start > cuts[1].start, "ordem decrescente");
        for c in &cuts {
            assert_eq!(c.start.0 % frame, 0);
            assert_eq!(c.end.0 % frame, 0);
        }
        // dentro do silêncio original (nunca come fala)
        let s0 = ticks_from_us(4_000_000).0;
        let e0 = ticks_from_us(5_500_000).0;
        assert!(cuts[0].start.0 >= s0 && cuts[0].end.0 <= e0);
    }

    #[test]
    fn commands_are_deterministic_and_split_before_delete() {
        let cuts = vec![Cut {
            clip: "c".into(),
            start: Ticks(100),
            end: Ticks(300),
        }];
        let a = cut_commands("k", &cuts, "track");
        assert_eq!(a, cut_commands("k", &cuts, "track"));
        let kinds: Vec<&str> = a
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["split_clip", "split_clip", "delete_clip"]);
        assert_eq!(a[2]["clip"], "sil_k_0_m");
    }
}
