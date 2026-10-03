//! Comandos de estrutura: assets, sequences, tracks e marcadores.

use super::check::track_locked;
use crate::ctx::{CommandOutput, Ctx};
use crate::error::{CommandError, Result};
use capia_model::{
    Asset, EntityKind, EntityRef, ErrorCode, MAX_TRACKS_PER_SEQUENCE, Marker, MarkerId,
    PrimitiveOp, SequenceHeader, SequenceId, Track, TrackId, TrackKind, TrackRole, TrackSlot,
};
use capia_time::{FrameRate, Ticks};

pub(crate) fn register_asset(ctx: &mut Ctx, asset: &Asset) -> Result<CommandOutput> {
    if ctx.doc.asset(&asset.id).is_some() {
        return Err(CommandError::invalid(format!(
            "asset {} already exists",
            asset.id
        )));
    }
    if asset.duration.is_some_and(|d| d <= Ticks::ZERO) {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "asset duration must be positive",
        ));
    }
    ctx.emit(PrimitiveOp::Asset {
        id: asset.id.clone(),
        old: None,
        new: Some(asset.clone()),
    })?;
    ctx.note_created(EntityKind::Asset, asset.id.as_str());
    Ok(ctx.take_output(Some(asset.id.0.clone())))
}

pub(crate) fn create_sequence(
    ctx: &mut Ctx,
    id: Option<&SequenceId>,
    name: &str,
    frame_rate: FrameRate,
    sample_rate: Option<u32>,
) -> Result<CommandOutput> {
    let id = id
        .cloned()
        .unwrap_or_else(|| SequenceId(ctx.derive_id("seq")));
    if ctx.doc.sequence(&id).is_some() {
        return Err(CommandError::invalid(format!(
            "sequence {id} already exists"
        )));
    }
    let sample_rate = sample_rate.unwrap_or(48_000);
    if !(8_000..=384_000).contains(&sample_rate) {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "sample_rate must be within [8000, 384000]",
        ));
    }
    let header = SequenceHeader {
        name: name.to_owned(),
        frame_rate,
        sample_rate,
    };
    ctx.emit(PrimitiveOp::Sequence {
        id: id.clone(),
        old: None,
        new: Some(header),
    })?;
    ctx.note_created(EntityKind::Sequence, id.as_str());
    Ok(ctx.take_output(Some(id.0)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add_track(
    ctx: &mut Ctx,
    sequence: &SequenceId,
    id: Option<&TrackId>,
    kind: TrackKind,
    name: Option<&str>,
    role: Option<&TrackRole>,
    magnetic: bool,
    index: Option<usize>,
) -> Result<CommandOutput> {
    let count = ctx.sequence(sequence)?.tracks().len();
    if count >= MAX_TRACKS_PER_SEQUENCE {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "too many tracks",
        ));
    }
    if magnetic && kind != TrackKind::Visual {
        return Err(CommandError::invalid("only visual tracks can be magnetic"));
    }
    let id = id.cloned().unwrap_or_else(|| TrackId(ctx.derive_id("trk")));
    if ctx.doc.find_track(&id).is_some() {
        return Err(CommandError::invalid(format!("track {id} already exists")));
    }
    let index = index.unwrap_or(count);
    if index > count {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            format!("index {index} > {count}"),
        ));
    }
    let mut track = Track::new(id.clone(), kind);
    track.name = name.unwrap_or_default().to_owned();
    track.magnetic = magnetic;
    if let Some(r) = role {
        track.role = r.clone();
    }
    ctx.emit(PrimitiveOp::Track {
        sequence: sequence.clone(),
        id: id.clone(),
        old: None,
        new: Some(TrackSlot { index, track }),
    })?;
    ctx.note_created(EntityKind::Track, id.as_str());
    Ok(ctx.take_output(Some(id.0)))
}

pub(crate) struct TrackFlags<'a> {
    pub locked: Option<bool>,
    pub hidden: Option<bool>,
    pub muted: Option<bool>,
    pub solo: Option<bool>,
    pub magnetic: Option<bool>,
    pub sync_lock: Option<bool>,
    pub group: Option<&'a str>,
    pub clear_group: bool,
    pub compact: bool,
}

pub(crate) fn set_track_flags(
    ctx: &mut Ctx,
    track_id: &TrackId,
    f: &TrackFlags<'_>,
) -> Result<CommandOutput> {
    let (seq_id, old) = ctx.locate_track(track_id)?;
    let only_unlocking = f.locked == Some(false)
        && f.hidden.is_none()
        && f.muted.is_none()
        && f.solo.is_none()
        && f.magnetic.is_none()
        && f.sync_lock.is_none()
        && f.group.is_none()
        && !f.clear_group;
    if !only_unlocking && f.locked != Some(false) {
        track_locked(&old)?;
    }
    let mut new = old.clone();
    if let Some(v) = f.locked {
        new.locked = v;
    }
    if let Some(v) = f.hidden {
        new.hidden = v;
    }
    if let Some(v) = f.muted {
        new.muted = v;
    }
    if let Some(v) = f.solo {
        new.solo = v;
    }
    if let Some(v) = f.sync_lock {
        new.sync_lock = v;
    }
    if let Some(g) = f.group {
        new.group = Some(g.to_owned());
    } else if f.clear_group {
        new.group = None;
    }
    let turning_magnetic_on = f.magnetic == Some(true) && !old.magnetic;
    if let Some(v) = f.magnetic {
        if v && old.kind != TrackKind::Visual {
            return Err(CommandError::invalid("only visual tracks can be magnetic"));
        }
        new.magnetic = v;
    }
    let (index, clips) = {
        let s = ctx.sequence(&seq_id)?;
        (
            s.track_position(track_id).unwrap_or(0),
            s.track_clips(track_id).cloned().collect::<Vec<_>>(),
        )
    };
    ctx.emit(PrimitiveOp::Track {
        sequence: seq_id.clone(),
        id: track_id.clone(),
        old: Some(TrackSlot { index, track: old }),
        new: Some(TrackSlot { index, track: new }),
    })?;
    if turning_magnetic_on {
        let mut cursor = Ticks::ZERO;
        for clip in clips {
            if clip.start != cursor {
                if !f.compact {
                    return Err(CommandError::new(
                        ErrorCode::GapInMagneticTrack,
                        format!("track {track_id} has gaps: pass compact=true to close them"),
                    )
                    .with_entities([EntityRef::new(EntityKind::Track, track_id.as_str())]));
                }
                let mut moved = clip.clone();
                moved.start = cursor;
                ctx.replace_clip_op(&seq_id, clip.clone(), moved)?;
            }
            cursor = cursor.checked_add(clip.duration)?;
        }
    }
    Ok(ctx.take_output(None))
}

pub(crate) fn delete_track(ctx: &mut Ctx, track_id: &TrackId) -> Result<CommandOutput> {
    let (seq_id, track) = ctx.locate_track(track_id)?;
    track_locked(&track)?;
    let (index, clips) = {
        let s = ctx.sequence(&seq_id)?;
        (
            s.track_position(track_id).unwrap_or(0),
            s.track_clips(track_id).cloned().collect::<Vec<_>>(),
        )
    };
    for clip in clips {
        ctx.remove_clip_op(&seq_id, clip)?;
    }
    ctx.emit(PrimitiveOp::Track {
        sequence: seq_id,
        id: track_id.clone(),
        old: Some(TrackSlot { index, track }),
        new: None,
    })?;
    Ok(ctx.take_output(None))
}

fn check_marker_time(time: Ticks) -> Result<()> {
    if !time.within_timeline() {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "marker time must be within [0, 24h]",
        ));
    }
    Ok(())
}

pub(crate) fn add_marker(
    ctx: &mut Ctx,
    sequence: &SequenceId,
    id: Option<&MarkerId>,
    time: Ticks,
    label: &str,
) -> Result<CommandOutput> {
    ctx.sequence(sequence)?;
    check_marker_time(time)?;
    let id = id
        .cloned()
        .unwrap_or_else(|| MarkerId(ctx.derive_id("mrk")));
    if ctx.sequence(sequence)?.marker(&id).is_some() {
        return Err(CommandError::invalid(format!("marker {id} already exists")));
    }
    let marker = Marker {
        id: id.clone(),
        time,
        label: label.to_owned(),
    };
    ctx.emit(PrimitiveOp::Marker {
        sequence: sequence.clone(),
        id: id.clone(),
        old: None,
        new: Some(marker),
    })?;
    ctx.note_created(EntityKind::Marker, id.as_str());
    Ok(ctx.take_output(Some(id.0)))
}

pub(crate) fn move_marker(
    ctx: &mut Ctx,
    sequence: &SequenceId,
    id: &MarkerId,
    time: Ticks,
) -> Result<CommandOutput> {
    check_marker_time(time)?;
    let old = ctx
        .sequence(sequence)?
        .marker(id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("marker", id))?;
    let new = Marker {
        time,
        ..old.clone()
    };
    ctx.emit(PrimitiveOp::Marker {
        sequence: sequence.clone(),
        id: id.clone(),
        old: Some(old),
        new: Some(new),
    })?;
    Ok(ctx.take_output(None))
}

pub(crate) fn delete_marker(
    ctx: &mut Ctx,
    sequence: &SequenceId,
    id: &MarkerId,
) -> Result<CommandOutput> {
    let old = ctx
        .sequence(sequence)?
        .marker(id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("marker", id))?;
    ctx.emit(PrimitiveOp::Marker {
        sequence: sequence.clone(),
        id: id.clone(),
        old: Some(old),
        new: None,
    })?;
    Ok(ctx.take_output(None))
}
