//! Legendas automáticas: transcrição → cues (linhas legíveis) → clips de texto **comuns** na
//! timeline (`insert_clip` com `content.text`), 100% editáveis à mão. O plano passa por
//! `preview → apply_plan` (ator `Agent`); `operation_id` e ids são determinísticos por tarefa, então
//! repetir não duplica (ADR-030).

use crate::ctx::IntelCtx;
use crate::error::{IntelError, IntelResult};
use crate::transcript::{TranscriptRecord, ticks_from_us, us_from_ticks};
use capia_ai::stt::Transcript;
use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionOptions {
    /// Caracteres por linha de legenda (9:16 ≈ 28–32).
    pub max_chars: usize,
    pub max_cue_us: i64,
    pub min_cue_us: i64,
    /// Silêncio entre palavras que força nova legenda.
    pub gap_break_us: i64,
    /// Altura relativa do preset de legenda (‰ da altura do quadro, a partir do centro).
    pub offset_permille: i64,
}

impl Default for CaptionOptions {
    fn default() -> Self {
        Self {
            max_chars: 32,
            max_cue_us: 3_500_000,
            min_cue_us: 700_000,
            gap_break_us: 700_000,
            offset_permille: 315,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cue {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
}

const MAX_CUES: usize = 20_000;

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn ends_sentence(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', ')', '”'])
        .ends_with(['.', '?', '!', '…'])
}

/// Agrupa palavras/segmentos em cues legíveis (tempos do **asset**, µs).
pub fn build_cues(t: &Transcript, o: &CaptionOptions) -> Vec<Cue> {
    let mut cues: Vec<Cue> = Vec::new();
    for seg in &t.segments {
        if seg.words.is_empty() {
            // sem tempo por palavra: reparte o segmento proporcionalmente ao tamanho do texto
            let words: Vec<&str> = seg.text.split_whitespace().collect();
            if words.is_empty() {
                continue;
            }
            let total_chars: usize = words.iter().map(|w| w.chars().count() + 1).sum();
            let span = (seg.end_us - seg.start_us).max(1);
            let mut cur = String::new();
            let mut cur_start = seg.start_us;
            let mut consumed = 0usize;
            for w in &words {
                let wl = w.chars().count() + 1;
                if !cur.is_empty() && cur.chars().count() + wl > o.max_chars {
                    let end = seg.start_us + span * consumed as i64 / total_chars as i64;
                    cues.push(Cue {
                        start_us: cur_start,
                        end_us: end.max(cur_start + 1),
                        text: clean(&cur),
                    });
                    cur.clear();
                    cur_start = end;
                }
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(w);
                consumed += wl;
            }
            if !cur.is_empty() {
                cues.push(Cue {
                    start_us: cur_start,
                    end_us: seg.end_us.max(cur_start + 1),
                    text: clean(&cur),
                });
            }
            continue;
        }
        let mut cur = String::new();
        let mut start = 0i64;
        let mut end = 0i64;
        for w in &seg.words {
            let text = clean(&w.text);
            if text.is_empty() {
                continue;
            }
            let would_len =
                cur.chars().count() + usize::from(!cur.is_empty()) + text.chars().count();
            let flush = !cur.is_empty()
                && (would_len > o.max_chars
                    || w.end_us - start > o.max_cue_us
                    || w.start_us - end > o.gap_break_us
                    || (ends_sentence(&cur) && cur.chars().count() >= 12));
            if flush {
                cues.push(Cue {
                    start_us: start,
                    end_us: end,
                    text: std::mem::take(&mut cur),
                });
            }
            if cur.is_empty() {
                start = w.start_us;
            } else {
                cur.push(' ');
            }
            cur.push_str(&text);
            end = w.end_us;
        }
        if !cur.is_empty() {
            cues.push(Cue {
                start_us: start,
                end_us: end,
                text: cur,
            });
        }
    }
    cues.retain(|c| !c.text.is_empty());
    cues.sort_by_key(|c| (c.start_us, c.end_us));
    // duração mínima legível sem invadir a próxima; sem sobreposição
    for i in 0..cues.len() {
        let next_start = cues.get(i + 1).map_or(i64::MAX, |n| n.start_us);
        let want = cues[i].start_us + o.min_cue_us;
        if cues[i].end_us < want {
            cues[i].end_us = want.min(next_start);
        }
        if cues[i].end_us > next_start {
            cues[i].end_us = next_start;
        }
        if cues[i].end_us <= cues[i].start_us {
            cues[i].end_us = cues[i].start_us + 1;
        }
    }
    cues.truncate(MAX_CUES);
    cues
}

/// Um clip de mídia da sequence que usa o asset transcrito.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipMap {
    pub clip_id: String,
    pub start: Ticks,
    pub duration: Ticks,
    pub source_in: Ticks,
    pub speed_num: i64,
    pub speed_den: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub start: Ticks,
    pub duration: Ticks,
    pub text: String,
    pub from_clip: String,
}

/// Mapeia cues (tempo do asset) para a timeline, respeitando corte/velocidade e alinhando ao frame.
/// Sem sobreposição entre legendas (uma track).
pub fn place_cues(cues: &[Cue], clips: &[ClipMap], frame: i64) -> Vec<Placed> {
    let frame = frame.max(1);
    let mut out: Vec<Placed> = Vec::new();
    for c in clips {
        if c.speed_num <= 0 || c.speed_den <= 0 {
            continue;
        }
        let win_start = us_from_ticks(c.source_in);
        // duração de conteúdo = duração na timeline × velocidade
        let content_ticks =
            i128::from(c.duration.0) * i128::from(c.speed_num) / i128::from(c.speed_den);
        let win_end = win_start + us_from_ticks(Ticks(content_ticks as i64));
        for cue in cues {
            let s = cue.start_us.max(win_start);
            let e = cue.end_us.min(win_end);
            if e <= s {
                continue;
            }
            let to_tl = |us: i64| -> i64 {
                let d = ticks_from_us(us - win_start).0;
                c.start.0
                    + (i128::from(d) * i128::from(c.speed_den) / i128::from(c.speed_num)) as i64
            };
            let start = to_tl(s).div_euclid(frame) * frame;
            let end = (to_tl(e) + frame - 1).div_euclid(frame) * frame;
            out.push(Placed {
                start: Ticks(start),
                duration: Ticks((end - start).max(frame)),
                text: cue.text.clone(),
                from_clip: c.clip_id.clone(),
            });
        }
    }
    out.sort_by_key(|p| (p.start, p.text.clone()));
    // sem sobreposição: empurra o início ao fim da anterior e descarta o que não cabe
    let mut res: Vec<Placed> = Vec::with_capacity(out.len());
    let mut last_end = i64::MIN;
    for mut p in out {
        if p.start.0 < last_end {
            let end = p.start.0 + p.duration.0;
            p.start = Ticks(last_end);
            if end - last_end < frame {
                continue;
            }
            p.duration = Ticks(end - last_end);
        }
        last_end = p.start.0 + p.duration.0;
        res.push(p);
    }
    res
}

/// Comandos (JSON) da legenda automática: uma track `captions` nova + um `insert_clip` por cue
/// (+ posição vertical). `key` torna `operation_id` e ids determinísticos por tarefa.
pub fn caption_commands(
    sequence: &str,
    key: &str,
    placed: &[Placed],
    frame_height: u32,
    o: &CaptionOptions,
) -> Value {
    let track = format!("cap_{key}");
    let mut cmds = vec![json!({
        "operation_id": format!("{key}:track"),
        "type": "add_track",
        "sequence": sequence,
        "id": track,
        "kind": "visual",
        "role": "captions",
        "name": "Captions (auto)",
    })];
    let pos_y = (o.offset_permille * i64::from(frame_height)) / 1000;
    for (i, p) in placed.iter().enumerate() {
        let id = format!("cap_{key}_{i}");
        cmds.push(json!({
            "operation_id": format!("{key}:clip:{i}"),
            "type": "insert_clip",
            "track": track,
            "start": p.start,
            "clip": {
                "id": id,
                "name": "",
                "duration": p.duration,
                "content": {
                    "type": "text",
                    "text": p.text,
                    "style": {
                        "size_permille": 55,
                        "weight": 700,
                        "color": "#FFFFFF",
                        "background": "#000000B3"
                    }
                },
                "source_in": 0,
                "speed": "1",
                "reversed": false,
                "properties": {}
            }
        }));
        cmds.push(json!({
            "operation_id": format!("{key}:pos:{i}"),
            "type": "set_property",
            "clip": id,
            "prop": "position_y",
            "value": pos_y as f64,
        }));
    }
    Value::Array(cmds)
}

/// Resultado do planejamento (nada gravado ainda).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CaptionPlan {
    pub cue_count: usize,
    pub preview: Value,
    pub plan_token: Option<String>,
    pub key: String,
}

fn parse_speed(v: &Value) -> (i64, i64) {
    match v {
        Value::String(s) => match s.split_once('/') {
            Some((a, b)) => (a.parse().unwrap_or(1), b.parse().unwrap_or(1)),
            None => (s.parse().unwrap_or(1), 1),
        },
        Value::Number(n) => (n.as_i64().unwrap_or(1), 1),
        _ => (1, 1),
    }
}

/// Clips de mídia (habilitados, não reversos) que usam `asset_id` na sequence.
pub fn clips_of_asset(seq: &Value, asset_id: &str) -> Vec<ClipMap> {
    let mut v: Vec<ClipMap> = Vec::new();
    let Some(clips) = seq["clips"].as_object() else {
        return v;
    };
    for (id, c) in clips {
        if c["content"]["type"].as_str() != Some("media")
            || c["content"]["asset"].as_str() != Some(asset_id)
            || c["enabled"].as_bool() == Some(false)
            || c["reversed"].as_bool() == Some(true)
        {
            continue;
        }
        let (n, d) = parse_speed(&c["speed"]);
        v.push(ClipMap {
            clip_id: id.clone(),
            start: Ticks(c["start"].as_i64().unwrap_or(0)),
            duration: Ticks(c["duration"].as_i64().unwrap_or(0)),
            source_in: Ticks(c["source_in"].as_i64().unwrap_or(0)),
            speed_num: n,
            speed_den: d,
        });
    }
    v.sort_by_key(|c| (c.start, c.clip_id.clone()));
    v
}

/// Planeja as legendas de uma sequence a partir de uma transcrição: monta os comandos e faz o
/// **preview** (gate do Agent). Devolve o token para `apply`.
pub fn plan_captions(
    ctx: &IntelCtx,
    rec: &TranscriptRecord,
    sequence: &str,
    o: &CaptionOptions,
    task_id: &str,
) -> IntelResult<CaptionPlan> {
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
    let clips = clips_of_asset(&seq, &rec.asset_id);
    if clips.is_empty() {
        return Err(IntelError::new(
            "NO_CLIPS",
            "the asset is not used by any enabled clip in this sequence",
        ));
    }
    let cues = build_cues(&rec.transcript, o);
    let placed = place_cues(&cues, &clips, frame);
    if placed.is_empty() {
        return Err(IntelError::new(
            "NO_CAPTIONS",
            "the transcript has no speech inside the used part of the media",
        ));
    }
    let key = crate::records::key(&[task_id, sequence, &rec.id]);
    let height = seq["header"]["height"].as_u64().unwrap_or(1080) as u32;
    let cmds = caption_commands(sequence, &key[..12], &placed, height, o);
    let preview = ctx
        .engine
        .preview(&ctx.actor, "Legendas automáticas", cmds)?;
    Ok(CaptionPlan {
        cue_count: placed.len(),
        plan_token: preview["plan_token"].as_str().map(str::to_owned),
        preview,
        key,
    })
}

/// Aplica o plano revisado (por token).
pub fn apply_plan(ctx: &IntelCtx, token: &str) -> IntelResult<Value> {
    ctx.engine.apply(&ctx.actor, token)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use capia_ai::stt::{Segment, Word};

    fn w(t: &str, s: i64, e: i64) -> Word {
        Word {
            start_us: s,
            end_us: e,
            text: t.into(),
            confidence: None,
        }
    }

    fn tr(words: Vec<Word>) -> Transcript {
        let end = words.last().map_or(0, |w| w.end_us);
        Transcript {
            schema_version: 1,
            language: Some("pt".into()),
            duration_us: Some(end),
            segments: vec![Segment {
                start_us: words.first().map_or(0, |w| w.start_us),
                end_us: end,
                text: String::new(),
                confidence: None,
                speaker: None,
                words,
            }],
        }
    }

    #[test]
    fn cues_break_on_length_sentence_and_gap_without_overlap() {
        let words = vec![
            w("Olá", 0, 300_000),
            w("pessoal,", 300_000, 700_000),
            w("hoje", 700_000, 1_000_000),
            w("eu", 1_000_000, 1_200_000),
            w("vou", 1_200_000, 1_400_000),
            w("mostrar.", 1_400_000, 2_000_000),
            w("Agora", 2_100_000, 2_400_000),
            w("sim", 2_400_000, 2_600_000),
            // longo silêncio → nova cue
            w("Fim", 5_000_000, 5_400_000),
        ];
        let c = build_cues(&tr(words), &CaptionOptions::default());
        assert!(c.len() >= 3, "{c:?}");
        for p in c.windows(2) {
            assert!(p[0].end_us <= p[1].start_us, "sobreposição: {c:?}");
        }
        assert!(c.iter().all(|x| x.text.chars().count() <= 32), "{c:?}");
        assert_eq!(c.last().unwrap().text, "Fim");
    }

    #[test]
    fn segments_without_word_times_are_split_proportionally() {
        let t = Transcript {
            schema_version: 1,
            language: None,
            duration_us: Some(4_000_000),
            segments: vec![Segment {
                start_us: 0,
                end_us: 4_000_000,
                text: "esta é uma frase bem longa para dividir em duas legendas".into(),
                confidence: None,
                speaker: None,
                words: vec![],
            }],
        };
        let c = build_cues(&t, &CaptionOptions::default());
        assert!(c.len() >= 2, "{c:?}");
        assert_eq!(c[0].start_us, 0);
        assert!(c.windows(2).all(|p| p[0].end_us <= p[1].start_us));
    }

    #[test]
    fn placement_honours_trim_speed_and_frame_alignment() {
        let frame = 23_520_000; // 30 fps
        let cues = vec![
            Cue {
                start_us: 0,
                end_us: 1_000_000,
                text: "antes do corte".into(),
            },
            Cue {
                start_us: 2_000_000,
                end_us: 3_000_000,
                text: "dentro".into(),
            },
        ];
        // clip mostra o conteúdo de 1,5 s a 3,5 s (source_in), na timeline em 10 s, velocidade 1
        let clips = vec![ClipMap {
            clip_id: "c1".into(),
            start: Ticks(10 * capia_time::TICKS_PER_SECOND),
            duration: Ticks(2 * capia_time::TICKS_PER_SECOND),
            source_in: ticks_from_us(1_500_000),
            speed_num: 1,
            speed_den: 1,
        }];
        let p = place_cues(&cues, &clips, frame);
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0].text, "dentro");
        // 2,0 s no asset = 0,5 s depois do início visível → 10,5 s na timeline
        let expect = 10 * capia_time::TICKS_PER_SECOND + capia_time::TICKS_PER_SECOND / 2;
        assert_eq!(
            p[0].start.0 / frame * frame,
            p[0].start.0,
            "alinhado ao frame"
        );
        assert!((p[0].start.0 - expect).abs() <= frame, "{:?}", p[0]);
    }

    #[test]
    fn double_speed_halves_timeline_duration_and_overlaps_are_resolved() {
        let frame = 23_520_000;
        let cues = vec![Cue {
            start_us: 0,
            end_us: 2_000_000,
            text: "x".into(),
        }];
        let clips = vec![
            ClipMap {
                clip_id: "a".into(),
                start: Ticks(0),
                duration: Ticks(capia_time::TICKS_PER_SECOND),
                source_in: Ticks(0),
                speed_num: 2,
                speed_den: 1,
            },
            // outro clip do mesmo asset sobreposto no tempo → a segunda legenda não pode sobrepor
            ClipMap {
                clip_id: "b".into(),
                start: Ticks(capia_time::TICKS_PER_SECOND / 2),
                duration: Ticks(capia_time::TICKS_PER_SECOND),
                source_in: Ticks(0),
                speed_num: 1,
                speed_den: 1,
            },
        ];
        let p = place_cues(&cues, &clips, frame);
        assert!(!p.is_empty());
        for w in p.windows(2) {
            assert!(w[0].start.0 + w[0].duration.0 <= w[1].start.0, "{p:?}");
        }
        assert!(
            p[0].duration.0 <= capia_time::TICKS_PER_SECOND + frame,
            "{p:?}"
        );
    }

    #[test]
    fn commands_are_deterministic_and_carry_stable_ids() {
        let placed = vec![Placed {
            start: Ticks(0),
            duration: Ticks(23_520_000 * 30),
            text: "olá".into(),
            from_clip: "c".into(),
        }];
        let a = caption_commands("S", "k1", &placed, 1920, &CaptionOptions::default());
        let b = caption_commands("S", "k1", &placed, 1920, &CaptionOptions::default());
        assert_eq!(a, b);
        assert_eq!(a[0]["operation_id"], "k1:track");
        assert_eq!(a[1]["clip"]["id"], "cap_k1_0");
        assert_eq!(a[2]["prop"], "position_y");
        assert_eq!(a[2]["value"], 604.0);
    }
}
