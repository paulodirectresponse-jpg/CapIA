//! Nested sequences (ADR-010/045): inserir, retargetar, `follow_length` e a reconciliação que faz
//! o clip acompanhar a duração da sequence filha.

use super::check::{check_clip, track_locked};
use super::clips;
use super::ripple::ripple_shift;
use crate::command::RippleScope;
use crate::ctx::{CommandOutput, Ctx};
use crate::error::{CommandError, Result};
use capia_model::{
    Clip, ClipContent, ClipId, EntityKind, EntityRef, ErrorCode, MAX_NESTING_DEPTH, SequenceId,
    Track, TrackId, TrackKind, check_nested_edge,
};
use capia_time::{MAX_TIMELINE_TICKS, Ticks};
use serde_json::json;

fn clip_ref(id: &ClipId) -> EntityRef {
    EntityRef::new(EntityKind::Clip, id.as_str())
}

/// `parent → target` mantém o DAG e a profundidade? Erro com índice/caminho **antes** de escrever.
pub(crate) fn guard_edge(ctx: &Ctx, parent: &SequenceId, target: &SequenceId) -> Result<()> {
    match check_nested_edge(&ctx.doc, parent, target) {
        Some(v) => Err(CommandError::from_violation(&v)
            .with_hint(json!({ "max_depth": MAX_NESTING_DEPTH, "parent": parent.as_str(), "target": target.as_str() }))),
        None => Ok(()),
    }
}

/// Duração natural de um nested: `duração(filha) − source_in`, alinhada ao frame do pai (half-up,
/// mínimo 1 frame) em track visual; em áudio, tick inteiro ≥ 1.
fn natural_duration(
    ctx: &Ctx,
    parent: &SequenceId,
    kind: TrackKind,
    target: &SequenceId,
    source_in: Ticks,
) -> Result<Ticks> {
    let child = ctx.sequence(target)?;
    let raw = child.duration().checked_sub(source_in)?;
    let fr = ctx.sequence(parent)?.frame_rate();
    Ok(match kind {
        TrackKind::Visual => fr.align_duration_half_up(raw.max(Ticks::ZERO))?,
        TrackKind::Audio => raw.max(Ticks(1)),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_nested(
    ctx: &mut Ctx,
    track_id: &TrackId,
    start: Ticks,
    target: &SequenceId,
    id: Option<&ClipId>,
    name: &str,
    duration: Option<Ticks>,
    source_in: Ticks,
    follow_length: bool,
    split_at_insert: bool,
    split_new_id: Option<&ClipId>,
) -> Result<CommandOutput> {
    let (seq_id, track) = ctx.locate_track(track_id)?;
    ctx.sequence(target)
        .map_err(|_| CommandError::not_found("sequence", target))?;
    let duration = match duration {
        Some(d) => d,
        None => natural_duration(ctx, &seq_id, track.kind, target, source_in)?,
    };
    let nc = crate::command::NewClip {
        id: id.cloned(),
        name: name.to_owned(),
        duration,
        content: ClipContent::Nested {
            sequence: target.clone(),
            follow_length,
        },
        source_in,
        speed: capia_time::Rational::ONE,
        reversed: false,
        properties: Default::default(),
    };
    clips::insert_clip(ctx, track_id, start, &nc, split_at_insert, split_new_id)
}

fn nested_clip(ctx: &Ctx, clip_id: &ClipId) -> Result<(SequenceId, Clip, Track, SequenceId, bool)> {
    let (seq, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    match &clip.content {
        ClipContent::Nested {
            sequence,
            follow_length,
        } => {
            let (target, follow) = (sequence.clone(), *follow_length);
            Ok((seq, clip, track, target, follow))
        }
        _ => Err(
            CommandError::invalid(format!("clip {clip_id} is not a nested sequence"))
                .with_entities([clip_ref(clip_id)]),
        ),
    }
}

pub(crate) fn set_nested_target(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    new_target: &SequenceId,
) -> Result<CommandOutput> {
    let (seq_id, clip, _track, old_target, follow) = nested_clip(ctx, clip_id)?;
    ctx.sequence(new_target)
        .map_err(|_| CommandError::not_found("sequence", new_target))?;
    if &old_target == new_target {
        return Ok(ctx.take_output(None));
    }
    // o grafo é avaliado SEM a aresta antiga (senão a profundidade/ciclo seriam superestimados)
    let mut without = ctx.doc.clone();
    without.apply_op(&capia_model::PrimitiveOp::Clip {
        sequence: seq_id.clone(),
        id: clip.id.clone(),
        old: Some(clip.clone()),
        new: None,
    })?;
    if let Some(v) = check_nested_edge(&without, &seq_id, new_target) {
        return Err(CommandError::from_violation(&v));
    }
    let mut new = clip.clone();
    new.content = ClipContent::Nested {
        sequence: new_target.clone(),
        follow_length: follow,
    };
    check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &new)?;
    ctx.replace_clip_op(&seq_id, clip, new)?;
    Ok(ctx.take_output(None))
}

pub(crate) fn set_follow_length(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    follow: bool,
) -> Result<CommandOutput> {
    let (seq_id, clip, _track, target, _old) = nested_clip(ctx, clip_id)?;
    let mut new = clip.clone();
    new.content = ClipContent::Nested {
        sequence: target,
        follow_length: follow,
    };
    ctx.replace_clip_op(&seq_id, clip, new)?;
    Ok(ctx.take_output(None))
}

/// Reconciliação de `follow_length` (idempotente): todo nested com a flag passa a ter a duração
/// natural da filha. Track magnética ⇒ ripple dos posteriores; track livre ⇒ limitada ao espaço
/// livre (com aviso; tenta de novo no próximo comando); track travada ⇒ intocada (aviso).
pub(crate) fn reconcile_follow(ctx: &mut Ctx) -> Result<()> {
    let rounds = ctx.doc.sequence_count() + 2;
    for _ in 0..rounds {
        let follow: Vec<(SequenceId, ClipId, SequenceId)> = ctx
            .doc
            .sequences()
            .flat_map(|(sid, s)| {
                s.nested_refs()
                    .filter(|(_, n)| n.follow_length)
                    .map(move |(cid, n)| (sid.clone(), cid.clone(), n.target.clone()))
            })
            .collect();
        if follow.is_empty() {
            return Ok(());
        }
        let mut changed = false;
        for (seq_id, clip_id, target) in follow {
            let (_, clip, track) = ctx.locate_clip(&clip_id)?;
            let desired = natural_duration(ctx, &seq_id, track.kind, &target, clip.source_in)?;
            if desired == clip.duration {
                continue;
            }
            if track.locked {
                let msg = format!(
                    "track {} is locked: nested clip {clip_id} did not follow its sequence",
                    track.id
                );
                if !ctx.warnings.contains(&msg) {
                    ctx.warnings.push(msg);
                }
                continue;
            }
            let fr = ctx.sequence(&seq_id)?.frame_rate();
            let mut desired = desired;
            if clip.start.checked_add(desired)?.0 > MAX_TIMELINE_TICKS {
                desired = Ticks(MAX_TIMELINE_TICKS - clip.start.0);
                if track.kind == TrackKind::Visual {
                    desired = fr.align_floor(desired);
                }
                if desired <= Ticks::ZERO {
                    continue;
                }
            }
            if !track.magnetic && desired > clip.duration {
                let limit = ctx
                    .sequence(&seq_id)?
                    .track_clips(&track.id)
                    .filter(|c| c.start >= clip.end() && c.id != clip.id)
                    .map(|c| c.start)
                    .min()
                    .map_or(Ticks(MAX_TIMELINE_TICKS), |t| t);
                let room = limit.checked_sub(clip.start)?;
                if room < desired {
                    let msg = format!(
                        "nested clip {clip_id} cannot follow its sequence fully: limited by the next clip on track {}",
                        track.id
                    );
                    if !ctx.warnings.contains(&msg) {
                        ctx.warnings.push(msg);
                    }
                    desired = room;
                    if track.kind == TrackKind::Visual {
                        desired = fr.align_floor(desired);
                    }
                    if desired <= clip.duration {
                        continue;
                    }
                }
            }
            let old_end = clip.end();
            let mut new = clip.clone();
            new.duration = desired;
            check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &new)?;
            let delta = desired.checked_sub(clip.duration)?;
            ctx.replace_clip_op(&seq_id, clip, new)?;
            if track.magnetic {
                ripple_shift(
                    ctx,
                    &seq_id,
                    &track,
                    Some(&clip_id),
                    old_end,
                    delta,
                    &RippleScope::Track,
                )?;
            }
            changed = true;
        }
        if !changed {
            return Ok(());
        }
    }
    Err(CommandError::new(
        ErrorCode::InvariantViolation,
        "follow_length reconciliation did not converge",
    ))
}
