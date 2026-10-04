//! Comandos de clip: inserir, mover, apagar, recortar, dividir e retime.

use super::check::{check_clip, check_no_overlap, track_locked};
use super::ripple::ripple_shift;
use crate::command::{ClipMove, Edge, NewClip, RippleScope};
use crate::ctx::{CommandOutput, Ctx};
use crate::error::{CommandError, Result};
use capia_model::{
    Clip, ClipContent, ClipId, EntityKind, EntityRef, ErrorCode, TrackId, TrackKind,
};
use capia_time::{Rational, Ticks};
use serde_json::json;
use std::collections::BTreeSet;

fn clip_ref(id: &ClipId) -> EntityRef {
    EntityRef::new(EntityKind::Clip, id.as_str())
}

fn fresh_clip_id(ctx: &mut Ctx, wanted: Option<ClipId>) -> Result<ClipId> {
    let id = wanted.unwrap_or_else(|| ClipId(ctx.derive_id("clip")));
    if ctx.doc.find_clip(&id).is_some() {
        return Err(
            CommandError::invalid(format!("clip id {id} already exists"))
                .with_entities([clip_ref(&id)]),
        );
    }
    Ok(id)
}

// ---------------------------------------------------------------------------------------------
// insert_clip
// ---------------------------------------------------------------------------------------------

pub(crate) fn insert_clip(
    ctx: &mut Ctx,
    track_id: &TrackId,
    start: Ticks,
    nc: &NewClip,
    split_at_insert: bool,
    split_new_id: Option<&ClipId>,
) -> Result<CommandOutput> {
    let (seq_id, track) = ctx.locate_track(track_id)?;
    track_locked(&track)?;
    if start < Ticks::ZERO {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "start must be >= 0",
        ));
    }
    let id = fresh_clip_id(ctx, nc.id.clone())?;

    // Track magnética não tem gaps: um start além do fim vira "no fim".
    let mut start = start;
    if track.magnetic {
        start = start.min(ctx.sequence(&seq_id)?.track_end(track_id));
    }
    let clip = Clip {
        id: id.clone(),
        track: track_id.clone(),
        start,
        duration: nc.duration,
        name: nc.name.clone(),
        enabled: true,
        content: nc.content.clone(),
        source_in: nc.source_in,
        speed: nc.speed,
        reversed: nc.reversed,
        properties: nc.properties.clone(),
        group: None,
        transition_in: None,
    };
    if let ClipContent::Nested {
        sequence: target, ..
    } = &nc.content
    {
        super::nested::guard_edge(ctx, &seq_id, target)?;
    }
    check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &clip)?;

    if track.magnetic {
        let inside = ctx
            .sequence(&seq_id)?
            .clip_at(track_id, start)
            .filter(|c| c.start < start)
            .map(|c| c.id.clone());
        if let Some(inside_id) = inside {
            if !split_at_insert {
                return Err(CommandError::new(
                    ErrorCode::NotOnBoundary,
                    format!("insert point is inside clip {inside_id} on magnetic track {track_id}"),
                )
                .with_entities([clip_ref(&inside_id)])
                .with_hint(
                    json!({ "resolve": "set split_at_insert=true or insert at a clip boundary" }),
                ));
            }
            split_core(ctx, &inside_id, start, split_new_id.cloned())?;
        }
        // abre espaço: tudo que começa em `start` ou depois avança `duration`
        ripple_shift(
            ctx,
            &seq_id,
            &track,
            None,
            start,
            nc.duration,
            &RippleScope::Track,
        )?;
    } else {
        let s = ctx.sequence(&seq_id)?;
        check_no_overlap(s, track_id, start, clip.end(), &BTreeSet::new())?;
    }
    ctx.insert_clip_op(&seq_id, clip)?;
    ctx.note_created(EntityKind::Clip, id.as_str());
    Ok(ctx.take_output(Some(id.0)))
}

// ---------------------------------------------------------------------------------------------
// delete_clip
// ---------------------------------------------------------------------------------------------

pub(crate) fn delete_clip(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    ripple: Option<bool>,
    scope: &RippleScope,
) -> Result<CommandOutput> {
    let (seq_id, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    if track.magnetic && ripple == Some(false) {
        return Err(CommandError::invalid(format!(
            "track {} is magnetic: deleting without ripple would leave a gap",
            track.id
        ))
        .with_entities([clip_ref(clip_id)]));
    }
    let ripple = ripple.unwrap_or(track.magnetic);
    ctx.remove_clip_op(&seq_id, clip.clone())?;
    if ripple {
        let delta = Ticks(-clip.duration.0);
        ripple_shift(
            ctx,
            &seq_id,
            &track,
            Some(clip_id),
            clip.end(),
            delta,
            scope,
        )?;
    }
    Ok(ctx.take_output(None))
}

// ---------------------------------------------------------------------------------------------
// trim_clip
// ---------------------------------------------------------------------------------------------

pub(crate) fn trim_clip(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    edge: Edge,
    to: Ticks,
    ripple: Option<bool>,
    scope: &RippleScope,
) -> Result<CommandOutput> {
    let (seq_id, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    if track.magnetic && ripple == Some(false) {
        return Err(CommandError::invalid(format!(
            "track {} is magnetic: trimming always ripples",
            track.id
        ))
        .with_entities([clip_ref(clip_id)]));
    }
    let ripple = ripple.unwrap_or(track.magnetic);
    let (speed_n, speed_d) = (clip.speed.num(), clip.speed.den());

    let mut new = clip.clone();
    match edge {
        Edge::Out => {
            if to <= clip.start {
                return Err(CommandError::new(
                    ErrorCode::OutOfRange,
                    "trim would leave no duration",
                )
                .with_entities([clip_ref(clip_id)]));
            }
            new.duration = to.checked_sub(clip.start)?;
            if clip.reversed {
                // a borda de saída da timeline é a ponta *baixa* da fonte
                let grown = new
                    .duration
                    .checked_sub(clip.duration)?
                    .mul_div_round(speed_n, speed_d)?;
                new.source_in = clip.source_in.checked_sub(grown)?;
            }
        }
        Edge::In => {
            if to >= clip.end() {
                return Err(CommandError::new(
                    ErrorCode::OutOfRange,
                    "trim would leave no duration",
                )
                .with_entities([clip_ref(clip_id)]));
            }
            let delta = to.checked_sub(clip.start)?;
            new.duration = clip.duration.checked_sub(delta)?;
            if !clip.reversed {
                new.source_in = clip
                    .source_in
                    .checked_add(delta.mul_div_round(speed_n, speed_d)?)?;
            }
            if !ripple {
                new.start = to;
            }
        }
    }
    check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &new)?;
    if !ripple {
        let ignore = BTreeSet::from([clip_id.clone()]);
        check_no_overlap(
            ctx.sequence(&seq_id)?,
            &track.id,
            new.start,
            new.end(),
            &ignore,
        )?;
    }
    let delta_len = new.duration.checked_sub(clip.duration)?;
    ctx.replace_clip_op(&seq_id, clip.clone(), new)?;
    if ripple {
        ripple_shift(
            ctx,
            &seq_id,
            &track,
            Some(clip_id),
            clip.end(),
            delta_len,
            scope,
        )?;
    }
    Ok(ctx.take_output(None))
}

// ---------------------------------------------------------------------------------------------
// split_clip
// ---------------------------------------------------------------------------------------------

/// Divide o clip em `at`; devolve o id da metade direita. Também usado por `insert_clip` com
/// `split_at_insert`.
///
/// Mapeamento de fonte (D-S7-6/8): a metade direita começa em `source_in + (at−start)×speed`
/// (reverso: a esquerda toca o trecho *mais tardio* da fonte). Os keyframes são **particionados no
/// ponto de divisão** com valor interpolado nas duas metades (`Animatable::split_at`); conteúdos
/// sem fonte externa (texto, imagem, sólido) re-basam o tempo local da metade direita em 0.
pub(crate) fn split_core(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    at: Ticks,
    new_id: Option<ClipId>,
) -> Result<ClipId> {
    let (seq_id, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    if at <= clip.start || at >= clip.end() {
        return Err(CommandError::new(
            ErrorCode::InvalidSplitPoint,
            format!("split point must be strictly inside clip {clip_id}"),
        )
        .with_entities([clip_ref(clip_id)])
        .with_hint(json!({ "clip_range_ticks": [clip.start.0, clip.end().0] })));
    }
    let right_id = fresh_clip_id(ctx, new_id)?;
    let offset = at.checked_sub(clip.start)?;
    let right_duration = clip.duration.checked_sub(offset)?;
    let (n, d) = (clip.speed.num(), clip.speed.den());
    let boundary = clip.content_time(at)?;

    let mut left = clip.clone();
    left.duration = offset;
    let mut right = clip.clone();
    right.id = right_id.clone();
    right.start = at;
    right.duration = right_duration;
    if clip.reversed {
        left.source_in = clip
            .source_in
            .checked_add(right_duration.mul_div_round(n, d)?)?;
    } else {
        right.source_in = clip.source_in.checked_add(offset.mul_div_round(n, d)?)?;
    }
    let rebase = !clip.content.has_source_time();
    if rebase {
        right.source_in = Ticks::ZERO;
    }
    for (name, anim) in &clip.properties {
        let split = anim.split_at(boundary, rebase);
        let (l, r) = if clip.reversed {
            (split.right, split.left)
        } else {
            (split.left, split.right)
        };
        left.properties.insert(name.clone(), l);
        right.properties.insert(name.clone(), r);
    }
    let s = ctx.sequence(&seq_id)?;
    check_clip(&ctx.doc, s, &left)?;
    check_clip(&ctx.doc, s, &right)?;
    ctx.replace_clip_op(&seq_id, clip, left)?;
    ctx.insert_clip_op(&seq_id, right)?;
    ctx.note_created(EntityKind::Clip, right_id.as_str());
    Ok(right_id)
}

pub(crate) fn split_clip(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    at: Ticks,
    new_id: Option<ClipId>,
) -> Result<CommandOutput> {
    let right = split_core(ctx, clip_id, at, new_id)?;
    Ok(ctx.take_output(Some(right.0)))
}

// ---------------------------------------------------------------------------------------------
// set_clip_speed
// ---------------------------------------------------------------------------------------------

pub(crate) fn set_clip_speed(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    speed: Rational,
    ripple: Option<bool>,
    scope: &RippleScope,
) -> Result<CommandOutput> {
    let (seq_id, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    if !clip.content.supports_retime() {
        return Err(
            CommandError::invalid("only media and nested clips support speed changes")
                .with_entities([clip_ref(clip_id)]),
        );
    }
    if !speed.is_positive() || !capia_model::speed_in_range(speed) {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            format!("speed {speed} is outside [1/100, 100]"),
        )
        .with_hint(json!({ "min": "1/100", "max": "100" })));
    }
    if track.magnetic && ripple == Some(false) {
        return Err(
            CommandError::invalid("track is magnetic: retime always ripples")
                .with_entities([clip_ref(clip_id)]),
        );
    }
    let ripple = ripple.unwrap_or(track.magnetic);

    // duração nova = duração × speed_antiga / speed_nova (D-S7-8)
    let ratio = clip.speed.checked_div(speed)?;
    let frame_rate = ctx.sequence(&seq_id)?.frame_rate();
    let new_duration = match track.kind {
        TrackKind::Visual => {
            frame_rate.scale_duration_half_up(clip.duration, ratio.num(), ratio.den())?
        }
        TrackKind::Audio => clip
            .duration
            .mul_div_round(ratio.num(), ratio.den())?
            .max(Ticks(1)),
    };
    let mut new = clip.clone();
    new.speed = speed;
    new.duration = new_duration;
    check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &new)?;
    if !ripple {
        let ignore = BTreeSet::from([clip_id.clone()]);
        check_no_overlap(
            ctx.sequence(&seq_id)?,
            &track.id,
            new.start,
            new.end(),
            &ignore,
        )?;
    }
    let delta_len = new.duration.checked_sub(clip.duration)?;
    ctx.replace_clip_op(&seq_id, clip.clone(), new)?;
    if ripple {
        ripple_shift(
            ctx,
            &seq_id,
            &track,
            Some(clip_id),
            clip.end(),
            delta_len,
            scope,
        )?;
    }
    Ok(ctx.take_output(None))
}

// ---------------------------------------------------------------------------------------------
// move_clips (estrito)
// ---------------------------------------------------------------------------------------------

/// Reorder de/para track magnética (ver `Command::ReorderClip`).
pub(crate) fn reorder_clip(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    dest_track: Option<&TrackId>,
    before: Option<&ClipId>,
    start: Option<Ticks>,
) -> Result<CommandOutput> {
    let (seq_id, clip, src) = ctx.locate_clip(clip_id)?;
    let dest_id = dest_track.cloned().unwrap_or_else(|| src.id.clone());
    let (dest_seq, dest) = ctx.locate_track(&dest_id)?;
    if dest_seq != seq_id {
        return Err(
            CommandError::invalid("clips can only move within their sequence")
                .with_entities([clip_ref(clip_id)]),
        );
    }
    track_locked(&src)?;
    track_locked(&dest)?;
    if !src.magnetic && !dest.magnetic {
        return Err(CommandError::invalid(
            "reorder_clip is for magnetic tracks; use move_clips between free tracks",
        )
        .with_entities([clip_ref(clip_id)]));
    }
    if before == Some(clip_id) {
        return Err(
            CommandError::invalid("a clip cannot be placed before itself")
                .with_entities([clip_ref(clip_id)]),
        );
    }
    // 1) retira da origem (fechando o gap se for magnética)
    ctx.remove_clip_op(&seq_id, clip.clone())?;
    if src.magnetic {
        ripple_shift(
            ctx,
            &seq_id,
            &src,
            Some(clip_id),
            clip.end(),
            Ticks(-clip.duration.0),
            &RippleScope::Track,
        )?;
    }
    // 2) destino
    let mut new = clip.clone();
    new.track = dest.id.clone();
    if dest.magnetic {
        let pos = match before {
            Some(b) => {
                let s = ctx.sequence(&seq_id)?;
                let target = s.clip(b).filter(|c| c.track == dest.id).ok_or_else(|| {
                    CommandError::invalid(format!(
                        "clip {b} is not on the destination track {}",
                        dest.id
                    ))
                    .with_entities([clip_ref(b)])
                })?;
                target.start
            }
            None => ctx.sequence(&seq_id)?.track_end(&dest.id),
        };
        ripple_shift(
            ctx,
            &seq_id,
            &dest,
            None,
            pos,
            clip.duration,
            &RippleScope::Track,
        )?;
        new.start = pos;
    } else {
        let s = start.ok_or_else(|| {
            CommandError::invalid("`start` is required to place a clip on a free track")
        })?;
        if s < Ticks::ZERO {
            return Err(
                CommandError::new(ErrorCode::OutOfRange, "start must be >= 0")
                    .with_entities([clip_ref(clip_id)]),
            );
        }
        new.start = s;
        check_no_overlap(
            ctx.sequence(&seq_id)?,
            &dest.id,
            new.start,
            new.end(),
            &BTreeSet::new(),
        )?;
    }
    check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &new)?;
    ctx.insert_clip_op(&seq_id, new)?;
    Ok(ctx.take_output(None))
}

pub(crate) fn move_clips(ctx: &mut Ctx, moves: &[ClipMove]) -> Result<CommandOutput> {
    if moves.is_empty() {
        return Err(CommandError::invalid("move_clips needs at least one move"));
    }
    let mut seen = BTreeSet::new();
    let mut planned: Vec<(capia_model::SequenceId, Clip, Clip)> = Vec::new();
    for mv in moves {
        if !seen.insert(mv.clip.clone()) {
            return Err(CommandError::invalid(format!(
                "clip {} listed twice",
                mv.clip
            )));
        }
        let (seq_id, clip, src) = ctx.locate_clip(&mv.clip)?;
        let dest_id = mv.track.clone().unwrap_or_else(|| src.id.clone());
        let (dest_seq, dest) = ctx.locate_track(&dest_id)?;
        if dest_seq != seq_id {
            return Err(
                CommandError::invalid("clips can only move within their sequence")
                    .with_entities([clip_ref(&mv.clip)]),
            );
        }
        track_locked(&src)?;
        track_locked(&dest)?;
        if src.magnetic || dest.magnetic {
            return Err(CommandError::new(
                ErrorCode::UnsupportedCommand,
                "moving clips into/out of magnetic tracks (reorder semantics) is not supported yet",
            )
            .with_entities([clip_ref(&mv.clip)]));
        }
        if mv.start < Ticks::ZERO {
            return Err(
                CommandError::new(ErrorCode::OutOfRange, "start must be >= 0")
                    .with_entities([clip_ref(&mv.clip)]),
            );
        }
        let mut new = clip.clone();
        new.track = dest.id;
        new.start = mv.start;
        if let Some((first_seq, _, _)) = planned.first()
            && first_seq != &seq_id
        {
            return Err(CommandError::invalid(
                "all moved clips must belong to one sequence",
            ));
        }
        planned.push((seq_id, clip, new));
    }
    for (seq_id, old, new) in &planned {
        ctx.replace_clip_op(seq_id, old.clone(), new.clone())?;
    }
    for (seq_id, _, new) in &planned {
        let s = ctx.sequence(seq_id)?;
        check_clip(&ctx.doc, s, new)?;
        let ignore = BTreeSet::from([new.id.clone()]);
        check_no_overlap(s, &new.track, new.start, new.end(), &ignore)?;
    }
    Ok(ctx.take_output(None))
}
