//! Snapping (D-S7-4/5). O threshold nasce em **pixels** da UI e é convertido para distância
//! temporal em **ticks** conforme o zoom — sem `floor` para frames inteiros. O destino final
//! respeita o alinhamento de frame das tracks visuais (half-up, D-S7-8).
//!
//! Prioridade em empate de distância: `playhead > marcador > borda de clip > menor timestamp`.

use crate::error::{CommandError, Result};
use capia_model::{ClipId, Sequence, TrackKind};
use capia_time::{Rational, TICKS_PER_SECOND, Ticks};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapTargetKind {
    Playhead,
    Marker,
    ClipStart,
    ClipEnd,
    SequenceStart,
}

impl SnapTargetKind {
    /// Menor = maior prioridade em empate de distância.
    fn priority(self) -> u8 {
        match self {
            Self::Playhead => 0,
            Self::Marker => 1,
            Self::ClipStart | Self::ClipEnd | Self::SequenceStart => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapTarget {
    #[serde(rename = "type")]
    pub kind: SnapTargetKind,
    pub t: Ticks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapResult {
    pub start: Ticks,
    pub snapped_to: Option<SnapTarget>,
}

#[derive(Clone, Debug)]
pub struct SnapRequest<'a> {
    pub clip: &'a ClipId,
    pub proposed_start: Ticks,
    pub threshold: Ticks,
    pub markers: &'a [Ticks],
    pub playhead: Option<Ticks>,
    pub enabled: bool,
}

/// `px / (pixels por segundo)` em ticks, arredondado half-up. Racionais evitam float.
pub fn threshold_ticks(px: Rational, pixels_per_second: Rational) -> Result<Ticks> {
    if !pixels_per_second.is_positive() || px.num() < 0 {
        return Err(CommandError::invalid(
            "px must be >= 0 and pixels_per_second > 0",
        ));
    }
    // ticks = px/pps × TPS = (px.num × pps.den × TPS) / (px.den × pps.num) — aritmética checada:
    // px e pps vêm da UI/API e podem ser extremos.
    let overflow = || CommandError::from(capia_time::TimeError::Overflow);
    let num = i128::from(px.num())
        .checked_mul(i128::from(pixels_per_second.den()))
        .and_then(|v| v.checked_mul(i128::from(TICKS_PER_SECOND)))
        .ok_or_else(overflow)?;
    let den = i128::from(px.den()) * i128::from(pixels_per_second.num());
    let twice = num
        .checked_mul(2)
        .and_then(|v| v.checked_add(den))
        .ok_or_else(overflow)?;
    let q = twice.div_euclid(2 * den);
    i64::try_from(q)
        .map(Ticks)
        .map_err(|_| CommandError::from(capia_time::TimeError::Overflow))
}

/// Alvos de snap fora do conjunto `exclude`: playhead, marcadores, bordas de clips e início da
/// sequence. Retorna `(alvo)`; a prioridade vem de [`SnapTargetKind::priority`].
pub(crate) fn collect_targets(
    seq: &Sequence,
    exclude: &dyn Fn(&ClipId) -> bool,
    markers: &[Ticks],
    playhead: Option<Ticks>,
) -> Vec<SnapTarget> {
    let mut out = Vec::new();
    if let Some(t) = playhead {
        out.push(SnapTarget {
            kind: SnapTargetKind::Playhead,
            t,
        });
    }
    out.extend(markers.iter().map(|t| SnapTarget {
        kind: SnapTargetKind::Marker,
        t: *t,
    }));
    out.extend(seq.markers().map(|m| SnapTarget {
        kind: SnapTargetKind::Marker,
        t: m.time,
    }));
    for c in seq.clips().filter(|c| !exclude(&c.id)) {
        out.push(SnapTarget {
            kind: SnapTargetKind::ClipStart,
            t: c.start,
        });
        out.push(SnapTarget {
            kind: SnapTargetKind::ClipEnd,
            t: c.end(),
        });
    }
    out.push(SnapTarget {
        kind: SnapTargetKind::SequenceStart,
        t: Ticks::ZERO,
    });
    out
}

/// Chave de ordenação de um candidato: (distância, prioridade, instante do alvo, início resultante).
type SnapKey = (i128, u8, Ticks, Ticks);

pub(crate) fn rank(distance: i128, target: SnapTarget) -> (i128, u8, Ticks) {
    (distance, target.kind.priority(), target.t)
}

pub fn resolve_snap(seq: &Sequence, req: &SnapRequest<'_>) -> Result<SnapResult> {
    let clip = seq
        .clip(req.clip)
        .ok_or_else(|| CommandError::not_found("clip", req.clip))?;
    let fr = seq.frame_rate();
    let visual = seq
        .track(&clip.track)
        .is_some_and(|t| t.kind == TrackKind::Visual);
    let align = |t: Ticks| -> Result<Ticks> {
        if visual {
            Ok(fr.align_half_up(t)?)
        } else {
            Ok(t)
        }
    };
    let duration = clip.duration;

    if req.enabled {
        let targets = collect_targets(seq, &|id| id == req.clip, req.markers, req.playhead);
        let mut best: Option<(SnapKey, SnapTarget, Ticks)> = None;
        for target in targets {
            for edge_offset in [Ticks::ZERO, duration] {
                let edge = req.proposed_start.checked_add(edge_offset)?;
                let distance = (i128::from(edge.0) - i128::from(target.t.0)).abs();
                if distance > i128::from(req.threshold.0) {
                    continue;
                }
                let candidate = align(target.t.checked_sub(edge_offset)?)?;
                if candidate < Ticks::ZERO {
                    continue;
                }
                let end = candidate.checked_add(duration)?;
                let blocked = seq
                    .clips_in(&clip.track, candidate, end)
                    .iter()
                    .any(|c| &c.id != req.clip);
                if blocked {
                    continue;
                }
                let (d, p, t) = rank(distance, target);
                let key = (d, p, t, candidate);
                if best.as_ref().is_none_or(|(k, _, _)| key < *k) {
                    best = Some((key, target, candidate));
                }
            }
        }
        if let Some((_, target, start)) = best {
            return Ok(SnapResult {
                start,
                snapped_to: Some(target),
            });
        }
    }
    let start = align(req.proposed_start)?.max(Ticks::ZERO);
    Ok(SnapResult {
        start,
        snapped_to: None,
    })
}

/// Snap de um **ponto** (borda em trim, playhead em scrub, marcador): o alvo mais próximo dentro do
/// `threshold`, com a mesma prioridade do snap de clips (`playhead > marcador > borda > menor t`).
/// Clips em `exclude` não são alvos (o clip arrastado). Sem alvo no limiar, devolve `None`.
pub fn resolve_point_snap(
    seq: &Sequence,
    exclude: &[ClipId],
    t: Ticks,
    threshold: Ticks,
    markers: &[Ticks],
    playhead: Option<Ticks>,
) -> Option<SnapTarget> {
    let targets = collect_targets(seq, &|id| exclude.contains(id), markers, playhead);
    targets
        .into_iter()
        .filter_map(|target| {
            let d = (i128::from(t.0) - i128::from(target.t.0)).abs();
            (d <= i128::from(threshold.0)).then(|| (rank(d, target), target))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, target)| target)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn threshold_is_exact_and_not_floored_to_frames() {
        // 10 px @ 50 px/s = 0,2 s ; @ 40 px/s = 0,25 s ; @ 20 px/s = 0,5 s
        assert_eq!(
            threshold_ticks(r(10, 1), r(50, 1)).unwrap(),
            Ticks(141_120_000)
        );
        assert_eq!(
            threshold_ticks(r(10, 1), r(40, 1)).unwrap(),
            Ticks(176_400_000)
        );
        assert_eq!(
            threshold_ticks(r(10, 1), r(20, 1)).unwrap(),
            Ticks(352_800_000)
        );
        // 7,5 frames a 30 fps = 176.400.000 ticks: o threshold mantém a meia-frame
        assert_eq!(176_400_000, 7 * 23_520_000 + 23_520_000 / 2);
        // fracionário (DPI): 10,5 px
        assert_eq!(
            threshold_ticks(r(21, 2), r(50, 1)).unwrap(),
            Ticks(148_176_000)
        );
    }

    #[test]
    fn threshold_never_panics_on_extreme_inputs() {
        let extremes = [0, 1, 2, i64::MAX - 1, i64::MAX];
        for &a in &extremes {
            for &b in &extremes {
                for &c in &extremes {
                    for &d in &extremes {
                        if let (Ok(px), Ok(pps)) =
                            (Rational::new(a, b.max(1)), Rational::new(c, d.max(1)))
                        {
                            let _ = threshold_ticks(px, pps);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn threshold_rejects_nonsense() {
        assert!(threshold_ticks(r(10, 1), r(0, 1)).is_err());
        assert!(threshold_ticks(r(-1, 1), r(50, 1)).is_err());
    }
}
