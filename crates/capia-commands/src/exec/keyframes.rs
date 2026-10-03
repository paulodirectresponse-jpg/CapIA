//! Propriedades e keyframes. O comando recebe o instante no tempo da **sequence**; o keyframe é
//! guardado em tempo de **conteúdo** (`Clip::content_time`), assim mover o clip preserva a animação
//! e recortar nunca a apaga (D-S7-6).

use super::check::{check_clip, track_locked};
use crate::ctx::{CommandOutput, Ctx};
use crate::error::{CommandError, Result};
use capia_model::{
    Animatable, Clip, ClipId, EntityKind, EntityRef, ErrorCode, Interp, Keyframe, PropertySpec,
    SequenceId, Track, TrackKind, property_spec,
};
use capia_time::Ticks;

struct Target {
    seq: SequenceId,
    clip: Clip,
    track: Track,
    spec: &'static PropertySpec,
}

fn target(ctx: &Ctx, clip_id: &ClipId, prop: &str) -> Result<Target> {
    let (seq, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    let spec = property_spec(prop).ok_or_else(|| {
        CommandError::invalid(format!("unknown property {prop}"))
            .with_entities([EntityRef::new(EntityKind::Clip, clip_id.as_str())])
    })?;
    Ok(Target {
        seq,
        clip,
        track,
        spec,
    })
}

fn check_value(spec: &PropertySpec, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(CommandError::invalid("value must be finite"));
    }
    if !spec.contains(value) {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            format!("{} must be within [{}, {}]", spec.name, spec.min, spec.max),
        ));
    }
    Ok(())
}

/// Valida o instante (dentro do clip, `end` inclusivo; alinhado a frame em track visual) e o
/// converte para tempo de conteúdo.
fn content_time_at(ctx: &Ctx, t: &Target, at: Ticks) -> Result<Ticks> {
    if at < t.clip.start || at > t.clip.end() {
        return Err(CommandError::new(
            ErrorCode::OutOfClipRange,
            format!(
                "keyframe time must be within clip {} [start, end]",
                t.clip.id
            ),
        )
        .with_entities([EntityRef::new(EntityKind::Clip, t.clip.id.as_str())])
        .with_hint(serde_json::json!({ "clip_range_ticks": [t.clip.start.0, t.clip.end().0] })));
    }
    if t.track.kind == TrackKind::Visual && !ctx.sequence(&t.seq)?.frame_rate().is_aligned(at) {
        return Err(CommandError::new(
            ErrorCode::NotFrameAligned,
            "keyframe time is not frame-aligned",
        )
        .with_entities([EntityRef::new(EntityKind::Clip, t.clip.id.as_str())]));
    }
    Ok(t.clip.content_time(at)?)
}

fn animatable(clip: &Clip, spec: &PropertySpec) -> Animatable {
    clip.properties
        .get(spec.name)
        .cloned()
        .unwrap_or(Animatable::Static(spec.default))
}

fn commit(ctx: &mut Ctx, t: Target, anim: Animatable) -> Result<CommandOutput> {
    let mut new = t.clip.clone();
    new.properties.insert(t.spec.name.to_owned(), anim);
    check_clip(&ctx.doc, ctx.sequence(&t.seq)?, &new)?;
    ctx.replace_clip_op(&t.seq, t.clip, new)?;
    Ok(ctx.take_output(None))
}

pub(crate) fn set_property(
    ctx: &mut Ctx,
    clip: &ClipId,
    prop: &str,
    value: f64,
) -> Result<CommandOutput> {
    let t = target(ctx, clip, prop)?;
    check_value(t.spec, value)?;
    if matches!(animatable(&t.clip, t.spec), Animatable::Animated(_)) {
        return Err(CommandError::invalid(format!(
            "{prop} is animated: edit its keyframes instead of setting a static value"
        )));
    }
    commit(ctx, t, Animatable::Static(value))
}

pub(crate) fn add_keyframe(
    ctx: &mut Ctx,
    clip: &ClipId,
    prop: &str,
    at: Ticks,
    value: f64,
    interp: Option<Interp>,
) -> Result<CommandOutput> {
    let t = target(ctx, clip, prop)?;
    check_value(t.spec, value)?;
    let interp = interp.unwrap_or_default();
    interp.validate().map_err(CommandError::invalid)?;
    let time = content_time_at(ctx, &t, at)?;
    let mut anim = animatable(&t.clip, t.spec);
    anim.set_keyframe(Keyframe {
        time,
        value,
        interp,
    });
    commit(ctx, t, anim)
}

pub(crate) fn move_keyframe(
    ctx: &mut Ctx,
    clip: &ClipId,
    prop: &str,
    from: Ticks,
    to: Ticks,
) -> Result<CommandOutput> {
    let t = target(ctx, clip, prop)?;
    let from_time = t.clip.content_time(from)?;
    let to_time = content_time_at(ctx, &t, to)?;
    let mut anim = animatable(&t.clip, t.spec);
    let Some(kf) = anim
        .keyframes()
        .iter()
        .find(|k| k.time == from_time)
        .copied()
    else {
        return Err(
            CommandError::not_found("keyframe", format!("{prop}@{}", from.0))
                .with_entities([EntityRef::new(EntityKind::Clip, clip.as_str())]),
        );
    };
    if to_time != from_time && anim.keyframes().iter().any(|k| k.time == to_time) {
        return Err(CommandError::new(
            ErrorCode::KeyframeTimeConflict,
            format!("a keyframe already exists at {} on {prop}", to.0),
        )
        .with_entities([EntityRef::new(EntityKind::Clip, clip.as_str())]));
    }
    anim.remove_keyframe(from_time);
    // `remove_keyframe` pode ter virado Static (era o único): set_keyframe reanima.
    anim.set_keyframe(Keyframe {
        time: to_time,
        ..kf
    });
    commit(ctx, t, anim)
}

pub(crate) fn delete_keyframe(
    ctx: &mut Ctx,
    clip: &ClipId,
    prop: &str,
    at: Ticks,
) -> Result<CommandOutput> {
    let t = target(ctx, clip, prop)?;
    let time = t.clip.content_time(at)?;
    let mut anim = animatable(&t.clip, t.spec);
    if anim.remove_keyframe(time).is_none() {
        return Err(
            CommandError::not_found("keyframe", format!("{prop}@{}", at.0))
                .with_entities([EntityRef::new(EntityKind::Clip, clip.as_str())]),
        );
    }
    commit(ctx, t, anim)
}

pub(crate) fn set_keyframe_interp(
    ctx: &mut Ctx,
    clip: &ClipId,
    prop: &str,
    at: Ticks,
    interp: Interp,
) -> Result<CommandOutput> {
    let t = target(ctx, clip, prop)?;
    interp.validate().map_err(CommandError::invalid)?;
    let time = t.clip.content_time(at)?;
    let mut anim = animatable(&t.clip, t.spec);
    let Some(kf) = anim.keyframes().iter().find(|k| k.time == time).copied() else {
        return Err(
            CommandError::not_found("keyframe", format!("{prop}@{}", at.0))
                .with_entities([EntityRef::new(EntityKind::Clip, clip.as_str())]),
        );
    };
    anim.set_keyframe(Keyframe { interp, ..kf });
    commit(ctx, t, anim)
}
