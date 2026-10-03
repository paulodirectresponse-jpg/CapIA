//! Resolução de movimento de grupo (função pura de UX): dado o delta pedido, devolve o delta
//! **permitido** — clamp em 0, contra obstáculos (não-membros), entre tracks da mesma família, com
//! snap opcional. O comando `move_clips` é estrito e valida o resultado.
//!
//! Fora do escopo (Fase 3): grupos que envolvem track magnética (semântica de *reorder*) —
//! retornam `UNSUPPORTED_COMMAND`.

use crate::error::{CommandError, Result};
use crate::snap::{collect_targets, rank};
use capia_model::{
    Clip, ClipId, EntityKind, EntityRef, ErrorCode, Sequence, Track, TrackId, TrackKind,
};
use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct GroupSnap<'a> {
    pub threshold: Ticks,
    pub markers: &'a [Ticks],
    pub playhead: Option<Ticks>,
}

#[derive(Clone, Debug)]
pub struct GroupMoveRequest<'a> {
    pub members: &'a [ClipId],
    pub delta_time: Ticks,
    pub delta_tracks: i32,
    pub snap: Option<GroupSnap<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMove {
    pub delta_time: Ticks,
    pub delta_tracks: i32,
}

struct Member<'a> {
    clip: &'a Clip,
    kind: TrackKind,
    /// Posição entre as tracks da mesma família (ordem da sequence).
    index: usize,
}

fn kind_tracks(seq: &Sequence, kind: TrackKind) -> Vec<&Track> {
    seq.tracks().iter().filter(|t| t.kind == kind).collect()
}

pub fn resolve_group_move(seq: &Sequence, req: &GroupMoveRequest<'_>) -> Result<GroupMove> {
    if req.members.is_empty() {
        return Err(CommandError::invalid(
            "group move needs at least one member",
        ));
    }
    let member_ids: BTreeSet<&ClipId> = req.members.iter().collect();
    let mut members: Vec<Member<'_>> = Vec::new();
    for id in &member_ids {
        let clip = seq
            .clip(id)
            .ok_or_else(|| CommandError::not_found("clip", id))?;
        let track = seq
            .track(&clip.track)
            .ok_or_else(|| CommandError::not_found("track", &clip.track))?;
        if track.locked {
            return Err(CommandError::new(
                ErrorCode::TrackLocked,
                format!("track {} is locked", track.id),
            )
            .with_entities([EntityRef::new(EntityKind::Track, track.id.as_str())]));
        }
        if track.magnetic {
            return Err(CommandError::new(
                ErrorCode::UnsupportedCommand,
                "group moves involving magnetic tracks are not supported yet",
            )
            .with_entities([EntityRef::new(EntityKind::Clip, clip.id.as_str())]));
        }
        let index = kind_tracks(seq, track.kind)
            .iter()
            .position(|t| t.id == track.id)
            .unwrap_or(0);
        members.push(Member {
            clip,
            kind: track.kind,
            index,
        });
    }
    let fr = seq.frame_rate();
    let has_visual = members.iter().any(|m| m.kind == TrackKind::Visual);

    // ---- delta de tracks: clamp aos extremos de cada família, depois recua até ser viável --------
    let lens = |k: TrackKind| i64::try_from(kind_tracks(seq, k).len()).unwrap_or(0);
    let idx = |m: &Member<'_>| i64::try_from(m.index).unwrap_or(0);
    let lo = members.iter().map(|m| -idx(m)).max().unwrap_or(0);
    let hi = members
        .iter()
        .map(|m| lens(m.kind) - 1 - idx(m))
        .min()
        .unwrap_or(0);
    let mut d = i64::from(req.delta_tracks).clamp(lo, hi);

    let dest_track = |m: &Member<'_>, d: i64| -> Option<TrackId> {
        let list = kind_tracks(seq, m.kind);
        let i = usize::try_from(idx(m) + d).ok()?;
        list.get(i).map(|t| t.id.clone())
    };
    let feasible = |d: i64| -> bool {
        if d == 0 {
            return true;
        }
        let mut occupied: Vec<(TrackId, Ticks, Ticks)> = Vec::new();
        for m in &members {
            let Some(dest) = dest_track(m, d) else {
                return false;
            };
            let Some(t) = seq.track(&dest) else {
                return false;
            };
            if t.locked || t.magnetic {
                return false;
            }
            let others_overlap = seq
                .clips_in(&dest, m.clip.start, m.clip.end())
                .iter()
                .any(|c| !member_ids.contains(&c.id));
            if others_overlap {
                return false;
            }
            if occupied
                .iter()
                .any(|(tid, s, e)| *tid == dest && m.clip.start < *e && *s < m.clip.end())
            {
                return false;
            }
            occupied.push((dest, m.clip.start, m.clip.end()));
        }
        true
    };
    while d != 0 && !feasible(d) {
        d -= d.signum();
    }

    // ---- delta de tempo: limites por obstáculo e por 0 ---------------------------------------
    let mut back_limit = members.iter().map(|m| m.clip.start.0).min().unwrap_or(0); // até o início da sequence
    let mut fwd_limit = i64::MAX;
    for m in &members {
        let dest = dest_track(m, d).unwrap_or_else(|| m.clip.track.clone());
        for x in seq
            .track_clips(&dest)
            .filter(|c| !member_ids.contains(&c.id))
        {
            if x.start >= m.clip.end() {
                fwd_limit = fwd_limit.min(x.start.0 - m.clip.end().0);
            }
            if x.end() <= m.clip.start {
                back_limit = back_limit.min(m.clip.start.0 - x.end().0);
            }
        }
        fwd_limit = fwd_limit.min(capia_time::MAX_TIMELINE_TICKS - m.clip.end().0);
    }
    let (lo_t, hi_t) = (-back_limit, fwd_limit);
    let align = |t: Ticks| -> Result<Ticks> {
        if has_visual {
            Ok(fr.align_half_up(t)?)
        } else {
            Ok(t)
        }
    };
    let mut dt = align(req.delta_time)?.0.clamp(lo_t, hi_t);

    if let Some(snap) = &req.snap {
        let targets = collect_targets(
            seq,
            &|id| member_ids.contains(id),
            snap.markers,
            snap.playhead,
        );
        let mut best: Option<((i128, u8, Ticks, i64), i64)> = None;
        for m in &members {
            for edge in [m.clip.start.0, m.clip.end().0] {
                let moved = edge + dt;
                for target in &targets {
                    let adjust = target.t.0 - moved;
                    if i128::from(adjust).abs() > i128::from(snap.threshold.0) {
                        continue;
                    }
                    let cand = align(Ticks(dt + adjust))?.0;
                    if cand < lo_t || cand > hi_t {
                        continue;
                    }
                    let (dist, prio, t) = rank(i128::from(adjust).abs(), *target);
                    let key = (dist, prio, t, cand);
                    if best.as_ref().is_none_or(|(k, _)| key < *k) {
                        best = Some((key, cand));
                    }
                }
            }
        }
        if let Some((_, cand)) = best {
            dt = cand;
        }
    }
    if has_visual {
        let f = fr.frame_duration().0;
        dt -= dt % f; // nunca ultrapassa um limite não alinhado (obstáculo de áudio)
    }
    Ok(GroupMove {
        delta_time: Ticks(dt),
        delta_tracks: i32::try_from(d)
            .map_err(|_| CommandError::invalid("track delta out of range"))?,
    })
}
