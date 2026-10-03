//! Ripple com escopo (D-S7-3): só as tracks **participantes** são deslocadas.
//!
//! * a track do clip sempre participa;
//! * `Tracks` lista tracks extras explicitamente (ignora `sync_lock`);
//! * `Group` = tracks com o mesmo rótulo de grupo da track do clip, com `sync_lock` ligado;
//! * `Sequence` = todas as tracks com `sync_lock` ligado;
//! * tracks **travadas** (exceto a do clip, que já foi rejeitada antes) ficam intactas, com aviso;
//! * se um clip de uma track participante cruza o trecho removido/aberto, não há como preservar as
//!   invariantes sem cortar clips: `RIPPLE_CONFLICT` estruturado (nada é aplicado).

use crate::command::RippleScope;
use crate::ctx::Ctx;
use crate::error::{CommandError, Result};
use capia_model::{Clip, EntityKind, EntityRef, ErrorCode, SequenceId, Track, TrackId};
use capia_time::Ticks;
use serde_json::json;

/// Tracks que participam do ripple (a própria primeiro). Preenche avisos de tracks puladas.
pub(crate) fn participants(
    ctx: &mut Ctx,
    seq: &SequenceId,
    own: &Track,
    scope: &RippleScope,
) -> Result<Vec<TrackId>> {
    let tracks: Vec<Track> = ctx.sequence(seq)?.tracks().to_vec();
    let mut out = vec![own.id.clone()];
    let consider = |t: &Track, ctx: &mut Ctx, out: &mut Vec<TrackId>| {
        if t.id == own.id {
            return;
        }
        if t.locked {
            ctx.warnings.push(format!(
                "track {} is locked: left untouched by ripple",
                t.id
            ));
        } else {
            out.push(t.id.clone());
        }
    };
    match scope {
        RippleScope::Track => {}
        RippleScope::Tracks { tracks: wanted } => {
            for id in wanted {
                let t = tracks.iter().find(|t| &t.id == id).ok_or_else(|| {
                    CommandError::not_found("track", id).with_hint(
                        json!({ "reason": "ripple scope track must be in the same sequence" }),
                    )
                })?;
                consider(t, ctx, &mut out);
            }
        }
        RippleScope::Group => {
            if let Some(g) = &own.group {
                for t in tracks
                    .iter()
                    .filter(|t| t.group.as_ref() == Some(g) && t.sync_lock)
                {
                    consider(t, ctx, &mut out);
                }
            }
        }
        RippleScope::Sequence => {
            for t in tracks.iter().filter(|t| t.sync_lock) {
                consider(t, ctx, &mut out);
            }
        }
    }
    Ok(out)
}

/// Desloca em `delta` todos os clips com `start >= boundary` nas tracks participantes.
///
/// `delta < 0` fecha o trecho `[boundary+delta, boundary)`; `delta > 0` abre espaço em `boundary`.
/// `exclude` é o clip que originou o ripple (já alterado/removido pelo chamador).
pub(crate) fn ripple_shift(
    ctx: &mut Ctx,
    seq: &SequenceId,
    own: &Track,
    exclude: Option<&capia_model::ClipId>,
    boundary: Ticks,
    delta: Ticks,
    scope: &RippleScope,
) -> Result<()> {
    if delta == Ticks::ZERO {
        return Ok(());
    }
    let parts = participants(ctx, seq, own, scope)?;
    let lo = Ticks(boundary.0 + delta.0.min(0));
    let hi = boundary;

    let mut conflicts: Vec<(TrackId, Clip)> = Vec::new();
    let mut shifts: Vec<Clip> = Vec::new();
    {
        let s = ctx.sequence(seq)?;
        for tid in &parts {
            for clip in s.track_clips(tid) {
                if Some(&clip.id) == exclude {
                    continue;
                }
                if clip.start < hi && clip.end() > lo {
                    conflicts.push((tid.clone(), clip.clone()));
                } else if clip.start >= boundary {
                    shifts.push(clip.clone());
                }
            }
        }
    }
    if !conflicts.is_empty() {
        let entities = conflicts
            .iter()
            .map(|(_, c)| EntityRef::new(EntityKind::Clip, c.id.as_str()));
        let details: Vec<_> = conflicts
            .iter()
            .map(|(t, c)| json!({ "track": t.as_str(), "clip": c.id.as_str(), "start": c.start.0, "end": c.end().0 }))
            .collect();
        return Err(CommandError::new(
            ErrorCode::RippleConflict,
            format!(
                "ripple cannot preserve the invariants: {} clip(s) cross the affected range",
                conflicts.len()
            ),
        )
        .with_entities(entities)
        .with_hint(json!({
            "range_ticks": [lo.0, hi.0],
            "conflicts": details,
            "participating_tracks": parts.iter().map(TrackId::as_str).collect::<Vec<_>>(),
            "suggestion": "narrow ripple_scope, unlock/exclude those tracks (sync_lock=false), or move the conflicting clips first",
        })));
    }
    shifts.sort_by(|a, b| (a.start, &a.id).cmp(&(b.start, &b.id)));
    for old in shifts {
        let mut new = old.clone();
        new.start = old.start.checked_add(delta)?;
        if new.start < Ticks::ZERO || new.end().0 > capia_time::MAX_TIMELINE_TICKS {
            return Err(CommandError::new(
                ErrorCode::OutOfRange,
                format!("ripple would move clip {} outside [0, 24h]", old.id),
            )
            .with_entities([EntityRef::new(EntityKind::Clip, old.id.as_str())]));
        }
        ctx.replace_clip_op(seq, old, new)?;
    }
    Ok(())
}
