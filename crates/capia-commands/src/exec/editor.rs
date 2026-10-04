//! Comandos do editor manual (Fase 3): formato de sequence, pastas, deliverables, grupos,
//! transições, texto, nome/ativação de clip e `detach_audio`. Todos expandem em ops primitivas
//! comuns (com undo); nenhum caminho de UI altera o documento por fora.

use super::check::{check_clip, check_no_overlap, track_locked};
use crate::ctx::{CommandOutput, Ctx};
use crate::error::{CommandError, Result};
use capia_model::{
    Asset, Clip, ClipContent, ClipId, Deliverable, DeliverableId, EntityKind, EntityRef, ErrorCode,
    Folder, FolderId, PrimitiveOp, SequenceId, TextStyle, TrackId, TrackKind, Transition,
    TransitionKind,
};
use capia_time::Ticks;
use std::collections::BTreeSet;

fn clip_ref(id: &ClipId) -> EntityRef {
    EntityRef::new(EntityKind::Clip, id.as_str())
}

// ------------------------------------------------------------------------------- sequence

pub(crate) fn set_sequence_format(
    ctx: &mut Ctx,
    sequence: &SequenceId,
    width: u32,
    height: u32,
) -> Result<CommandOutput> {
    if !(16..=16_384).contains(&width) || !(16..=16_384).contains(&height) {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "width/height must be within 16..=16384",
        ));
    }
    let old = ctx.sequence(sequence)?.header.clone();
    let mut new = old.clone();
    new.width = width;
    new.height = height;
    if old != new {
        ctx.emit(PrimitiveOp::Sequence {
            id: sequence.clone(),
            old: Some(old),
            new: Some(new),
        })?;
    }
    Ok(ctx.take_output(None))
}

pub(crate) fn set_sequence_folder(
    ctx: &mut Ctx,
    sequence: &SequenceId,
    folder: Option<&FolderId>,
) -> Result<CommandOutput> {
    if let Some(f) = folder {
        if ctx.doc.folder(f).is_none() {
            return Err(CommandError::not_found("folder", f));
        }
    }
    let old = ctx.sequence(sequence)?.header.clone();
    let mut new = old.clone();
    new.folder = folder.cloned();
    if old != new {
        ctx.emit(PrimitiveOp::Sequence {
            id: sequence.clone(),
            old: Some(old),
            new: Some(new),
        })?;
    }
    Ok(ctx.take_output(None))
}

// --------------------------------------------------------------------------------- folders

fn check_folder_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 128 {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "folder name must have 1..=128 characters",
        ));
    }
    Ok(())
}

pub(crate) fn create_folder(
    ctx: &mut Ctx,
    id: Option<&FolderId>,
    name: &str,
    parent: Option<&FolderId>,
) -> Result<CommandOutput> {
    check_folder_name(name)?;
    if let Some(p) = parent {
        if ctx.doc.folder(p).is_none() {
            return Err(CommandError::not_found("folder", p));
        }
    }
    let id = id
        .cloned()
        .unwrap_or_else(|| FolderId(ctx.derive_id("fld")));
    if ctx.doc.folder(&id).is_some() {
        return Err(CommandError::invalid(format!("folder {id} already exists")));
    }
    ctx.emit(PrimitiveOp::Folder {
        id: id.clone(),
        old: None,
        new: Some(Folder {
            id: id.clone(),
            name: name.to_owned(),
            parent: parent.cloned(),
        }),
    })?;
    ctx.note_created(EntityKind::Folder, id.as_str());
    Ok(ctx.take_output(Some(id.0)))
}

pub(crate) fn rename_folder(ctx: &mut Ctx, folder: &FolderId, name: &str) -> Result<CommandOutput> {
    check_folder_name(name)?;
    let old = ctx
        .doc
        .folder(folder)
        .cloned()
        .ok_or_else(|| CommandError::not_found("folder", folder))?;
    let mut new = old.clone();
    new.name = name.to_owned();
    if old != new {
        ctx.emit(PrimitiveOp::Folder {
            id: folder.clone(),
            old: Some(old),
            new: Some(new),
        })?;
    }
    Ok(ctx.take_output(None))
}

pub(crate) fn move_folder(
    ctx: &mut Ctx,
    folder: &FolderId,
    parent: Option<&FolderId>,
) -> Result<CommandOutput> {
    let old = ctx
        .doc
        .folder(folder)
        .cloned()
        .ok_or_else(|| CommandError::not_found("folder", folder))?;
    if let Some(p) = parent {
        // destino existe e não é o próprio nem um descendente (evita ciclo)
        let mut cur = Some(p.clone());
        while let Some(c) = cur {
            if &c == folder {
                return Err(CommandError::new(
                    ErrorCode::InvariantViolation,
                    "a folder cannot be moved into itself or its descendants",
                ));
            }
            let f = ctx
                .doc
                .folder(&c)
                .ok_or_else(|| CommandError::not_found("folder", &c))?;
            cur = f.parent.clone();
        }
    }
    let mut new = old.clone();
    new.parent = parent.cloned();
    if old != new {
        ctx.emit(PrimitiveOp::Folder {
            id: folder.clone(),
            old: Some(old),
            new: Some(new),
        })?;
    }
    Ok(ctx.take_output(None))
}

pub(crate) fn delete_folder(ctx: &mut Ctx, folder: &FolderId) -> Result<CommandOutput> {
    let old = ctx
        .doc
        .folder(folder)
        .cloned()
        .ok_or_else(|| CommandError::not_found("folder", folder))?;
    let children = ctx
        .doc
        .folders()
        .filter(|f| f.parent.as_ref() == Some(folder))
        .count();
    let sequences = ctx
        .doc
        .sequences()
        .filter(|(_, s)| s.header.folder.as_ref() == Some(folder))
        .count();
    if children + sequences > 0 {
        return Err(CommandError::new(
            ErrorCode::InUse,
            format!("folder {folder} is not empty ({children} folder(s), {sequences} sequence(s))"),
        )
        .with_entities([EntityRef::new(EntityKind::Folder, folder.as_str())]));
    }
    ctx.emit(PrimitiveOp::Folder {
        id: folder.clone(),
        old: Some(old),
        new: None,
    })?;
    Ok(ctx.take_output(None))
}

// ----------------------------------------------------------------------------- deliverables

fn check_deliverable(ctx: &Ctx, d: &Deliverable) -> Result<()> {
    if ctx.doc.sequence(&d.sequence).is_none() {
        return Err(CommandError::not_found("sequence", &d.sequence));
    }
    if d.name.trim().is_empty() || d.preset.trim().is_empty() || d.path.trim().is_empty() {
        return Err(CommandError::invalid(
            "deliverable needs name, preset and path",
        ));
    }
    for v in [d.width, d.height].into_iter().flatten() {
        if !(16..=16_384).contains(&v) {
            return Err(CommandError::new(
                ErrorCode::OutOfRange,
                "deliverable width/height must be within 16..=16384",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_deliverable(
    ctx: &mut Ctx,
    id: Option<&DeliverableId>,
    name: &str,
    sequence: &SequenceId,
    preset: &str,
    path: &str,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<CommandOutput> {
    let id = id
        .cloned()
        .unwrap_or_else(|| DeliverableId(ctx.derive_id("dlv")));
    if ctx.doc.deliverable(&id).is_some() {
        return Err(CommandError::invalid(format!(
            "deliverable {id} already exists"
        )));
    }
    let d = Deliverable {
        id: id.clone(),
        name: name.to_owned(),
        sequence: sequence.clone(),
        preset: preset.to_owned(),
        path: path.to_owned(),
        width,
        height,
    };
    check_deliverable(ctx, &d)?;
    ctx.emit(PrimitiveOp::Deliverable {
        id: id.clone(),
        old: None,
        new: Some(d),
    })?;
    ctx.note_created(EntityKind::Deliverable, id.as_str());
    Ok(ctx.take_output(Some(id.0)))
}

pub(crate) fn update_deliverable(ctx: &mut Ctx, d: &Deliverable) -> Result<CommandOutput> {
    let old = ctx
        .doc
        .deliverable(&d.id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("deliverable", &d.id))?;
    check_deliverable(ctx, d)?;
    if &old != d {
        ctx.emit(PrimitiveOp::Deliverable {
            id: d.id.clone(),
            old: Some(old),
            new: Some(d.clone()),
        })?;
    }
    Ok(ctx.take_output(None))
}

pub(crate) fn delete_deliverable(ctx: &mut Ctx, id: &DeliverableId) -> Result<CommandOutput> {
    let old = ctx
        .doc
        .deliverable(id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("deliverable", id))?;
    ctx.emit(PrimitiveOp::Deliverable {
        id: id.clone(),
        old: Some(old),
        new: None,
    })?;
    Ok(ctx.take_output(None))
}

/// Remove os deliverables que apontam para `sequence` (chamado por `delete_sequence`).
pub(crate) fn drop_deliverables_of(ctx: &mut Ctx, sequence: &SequenceId) -> Result<()> {
    let doomed: Vec<Deliverable> = ctx
        .doc
        .deliverables()
        .filter(|d| &d.sequence == sequence)
        .cloned()
        .collect();
    for d in doomed {
        ctx.emit(PrimitiveOp::Deliverable {
            id: d.id.clone(),
            old: Some(d),
            new: None,
        })?;
    }
    Ok(())
}

// ------------------------------------------------------------------------------- clip fields

/// Clip editável: existe, a track não está travada.
fn editable(ctx: &Ctx, clip: &ClipId) -> Result<(SequenceId, Clip)> {
    let (seq, clip, track) = ctx.locate_clip(clip)?;
    track_locked(&track)?;
    Ok((seq, clip))
}

fn commit_clip(ctx: &mut Ctx, seq: &SequenceId, old: Clip, new: Clip) -> Result<()> {
    check_clip(&ctx.doc, ctx.sequence(seq)?, &new)?;
    ctx.replace_clip_op(seq, old, new)
}

pub(crate) fn rename_clip(ctx: &mut Ctx, clip: &ClipId, name: &str) -> Result<CommandOutput> {
    if name.len() > 256 {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "clip name must have at most 256 characters",
        ));
    }
    let (seq, old) = editable(ctx, clip)?;
    let mut new = old.clone();
    new.name = name.to_owned();
    commit_clip(ctx, &seq, old, new)?;
    Ok(ctx.take_output(None))
}

pub(crate) fn set_clip_enabled(
    ctx: &mut Ctx,
    clip: &ClipId,
    enabled: bool,
) -> Result<CommandOutput> {
    let (seq, old) = editable(ctx, clip)?;
    let mut new = old.clone();
    new.enabled = enabled;
    commit_clip(ctx, &seq, old, new)?;
    Ok(ctx.take_output(None))
}

pub(crate) fn set_text(
    ctx: &mut Ctx,
    clip: &ClipId,
    text: Option<&str>,
    style: Option<&TextStyle>,
) -> Result<CommandOutput> {
    let (seq, old) = editable(ctx, clip)?;
    let ClipContent::Text {
        text: old_text,
        style: old_style,
    } = &old.content
    else {
        return Err(CommandError::new(
            ErrorCode::WrongTrackKind,
            format!("clip {clip} is not a text clip"),
        )
        .with_entities([clip_ref(clip)]));
    };
    if text.is_some_and(|t| t.len() > 10_000) {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "text must have at most 10000 bytes",
        ));
    }
    if let Some(s) = style {
        s.validate()
            .map_err(|why| CommandError::new(ErrorCode::OutOfRange, why))?;
    }
    let mut new = old.clone();
    new.content = ClipContent::Text {
        text: text.map_or_else(|| old_text.clone(), str::to_owned),
        style: style.cloned().unwrap_or_else(|| old_style.clone()),
    };
    commit_clip(ctx, &seq, old, new)?;
    Ok(ctx.take_output(None))
}

// ------------------------------------------------------------------------------------ groups

pub(crate) fn group_clips(
    ctx: &mut Ctx,
    clips: &[ClipId],
    group: Option<&str>,
) -> Result<CommandOutput> {
    if clips.len() < 2 {
        return Err(CommandError::invalid("a group needs at least 2 clips"));
    }
    let label = group
        .map(str::to_owned)
        .unwrap_or_else(|| ctx.derive_id("grp"));
    if label.is_empty() || label.len() > 64 {
        return Err(CommandError::new(
            ErrorCode::OutOfRange,
            "group label must have 1..=64 characters",
        ));
    }
    let mut seq_of: Option<SequenceId> = None;
    let mut targets = Vec::new();
    for id in clips.iter().collect::<BTreeSet<_>>() {
        let (seq, clip) = editable(ctx, id)?;
        if seq_of.as_ref().is_some_and(|s| s != &seq) {
            return Err(CommandError::invalid(
                "all clips of a group must belong to the same sequence",
            ));
        }
        seq_of = Some(seq);
        targets.push(clip);
    }
    let seq = seq_of.ok_or_else(|| CommandError::invalid("no clips"))?;
    for old in targets {
        let mut new = old.clone();
        new.group = Some(label.clone());
        commit_clip(ctx, &seq, old, new)?;
    }
    Ok(ctx.take_output(Some(label)))
}

/// Desfaz o(s) grupo(s) dos clips dados (e de todos os membros desses grupos).
pub(crate) fn ungroup(ctx: &mut Ctx, clips: &[ClipId]) -> Result<CommandOutput> {
    let mut labels: BTreeSet<(SequenceId, String)> = BTreeSet::new();
    for id in clips {
        let (seq, clip, _) = ctx.locate_clip(id)?;
        if let Some(g) = clip.group {
            labels.insert((seq, g));
        }
    }
    for (seq_id, label) in labels {
        let members: Vec<Clip> = ctx
            .sequence(&seq_id)?
            .clips()
            .filter(|c| c.group.as_deref() == Some(label.as_str()))
            .cloned()
            .collect();
        for old in members {
            let track = ctx
                .sequence(&seq_id)?
                .track(&old.track)
                .cloned()
                .ok_or_else(|| CommandError::not_found("track", &old.track))?;
            track_locked(&track)?;
            let mut new = old.clone();
            new.group = None;
            commit_clip(ctx, &seq_id, old, new)?;
        }
    }
    Ok(ctx.take_output(None))
}

// ------------------------------------------------------------------------------- transitions

/// Duração de conteúdo (em ticks) disponível na fonte de um clip, se limitada.
fn source_len(ctx: &Ctx, clip: &Clip) -> Option<Ticks> {
    match &clip.content {
        ClipContent::Media { asset, .. } => ctx.doc.asset(asset).and_then(|a: &Asset| a.duration),
        ClipContent::Nested { sequence, .. } => ctx.doc.sequence(sequence).map(|s| s.duration()),
        _ => None,
    }
}

fn transition_hint(max: Ticks) -> serde_json::Value {
    serde_json::json!({ "max_duration_ticks": max.0 })
}

pub(crate) fn set_transition(
    ctx: &mut Ctx,
    clip: &ClipId,
    transition: Option<&Transition>,
) -> Result<CommandOutput> {
    let (seq_id, old) = editable(ctx, clip)?;
    let mut new = old.clone();
    new.transition_in = transition.copied();
    if let Some(tr) = transition {
        let seq = ctx.sequence(&seq_id)?;
        let track = seq
            .track(&old.track)
            .cloned()
            .ok_or_else(|| CommandError::not_found("track", &old.track))?;
        if track.kind != TrackKind::Visual {
            return Err(CommandError::new(
                ErrorCode::WrongTrackKind,
                "transitions only exist on visual tracks",
            ));
        }
        if tr.duration <= Ticks::ZERO || !seq.frame_rate().is_aligned(tr.duration) {
            return Err(CommandError::new(
                ErrorCode::NotFrameAligned,
                "transition duration must be a positive whole number of frames",
            ));
        }
        let prev = seq
            .track_clips(&old.track)
            .filter(|c| c.end() == old.start)
            .cloned()
            .next();
        match tr.kind {
            TransitionKind::SlideIn => {
                if tr.duration > old.duration {
                    return Err(CommandError::new(
                        ErrorCode::OutOfRange,
                        "transition is longer than the clip",
                    )
                    .with_hint(transition_hint(old.duration)));
                }
            }
            TransitionKind::Fade | TransitionKind::Dissolve => {
                let Some(prev) = prev else {
                    return Err(CommandError::new(
                        ErrorCode::NotOnBoundary,
                        "this transition needs an adjacent previous clip on the same track",
                    )
                    .with_entities([clip_ref(clip)]));
                };
                let half = Ticks(tr.duration.0 / 2);
                if tr.duration.0 % 2 != 0 || half <= Ticks::ZERO {
                    return Err(CommandError::new(
                        ErrorCode::OutOfRange,
                        "transition duration must be even (ticks)",
                    ));
                }
                // espaço próprio: metade em cada lado do corte, sem invadir a transição vizinha
                let prev_half = prev
                    .transition_in
                    .map_or(Ticks::ZERO, |t| Ticks(t.duration.0 / 2));
                let own_cap = old.duration.0.min(prev.duration.0 - prev_half.0);
                if half.0 > own_cap {
                    return Err(CommandError::new(
                        ErrorCode::OutOfRange,
                        "transition is longer than the neighbouring clips allow",
                    )
                    .with_hint(transition_hint(Ticks(own_cap.max(0) * 2))));
                }
                if tr.kind == TransitionKind::Dissolve {
                    if old.reversed || prev.reversed {
                        return Err(CommandError::invalid(
                            "dissolve does not support reversed clips",
                        ));
                    }
                    // handles: o anterior precisa de `half` de fonte além do fim; este, antes do início
                    let need_after = i128::from(half.0) * i128::from(prev.speed.num())
                        / i128::from(prev.speed.den());
                    let need_before = i128::from(half.0) * i128::from(old.speed.num())
                        / i128::from(old.speed.den());
                    let prev_end_src = i128::from(prev.source_in.0)
                        + i128::from(prev.duration.0) * i128::from(prev.speed.num())
                            / i128::from(prev.speed.den());
                    let after_ok = source_len(ctx, &prev)
                        .is_none_or(|len| prev_end_src + need_after <= i128::from(len.0));
                    let before_ok = !old.content.has_source_time()
                        || i128::from(old.source_in.0) >= need_before;
                    if !after_ok || !before_ok {
                        return Err(CommandError::new(
                            ErrorCode::InsufficientHandles,
                            "not enough source handles for a dissolve at this cut (use fade or a shorter duration)",
                        )
                        .with_entities([clip_ref(clip), clip_ref(&prev.id)]));
                    }
                }
            }
        }
    }
    commit_clip(ctx, &seq_id, old, new)?;
    Ok(ctx.take_output(None))
}

// ------------------------------------------------------------------------------ detach audio

pub(crate) fn detach_audio(
    ctx: &mut Ctx,
    clip: &ClipId,
    audio_track: Option<&TrackId>,
    audio_clip_id: Option<&ClipId>,
) -> Result<CommandOutput> {
    let (seq_id, old) = editable(ctx, clip)?;
    let ClipContent::Media {
        asset,
        has_video: true,
        has_audio: true,
    } = &old.content
    else {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            format!("clip {clip} has no linked audio to detach"),
        )
        .with_entities([clip_ref(clip)]));
    };
    let asset = asset.clone();
    // destino: a track pedida, ou a primeira track de áudio livre no trecho
    let target = {
        let seq = ctx.sequence(&seq_id)?;
        match audio_track {
            Some(t) => seq
                .track(t)
                .cloned()
                .ok_or_else(|| CommandError::not_found("track", t))?,
            None => seq
                .tracks()
                .iter()
                .find(|t| {
                    t.kind == TrackKind::Audio
                        && !t.locked
                        && check_no_overlap(seq, &t.id, old.start, old.end(), &BTreeSet::new())
                            .is_ok()
                })
                .cloned()
                .ok_or_else(|| {
                    CommandError::new(
                        ErrorCode::NotFound,
                        "no free audio track for the detached audio: add an audio track first",
                    )
                })?,
        }
    };
    if target.kind != TrackKind::Audio {
        return Err(CommandError::new(
            ErrorCode::WrongTrackKind,
            format!("track {} is not an audio track", target.id),
        ));
    }
    track_locked(&target)?;
    let new_id = audio_clip_id
        .cloned()
        .unwrap_or_else(|| ClipId(ctx.derive_id("clip")));
    if ctx.doc.find_clip(&new_id).is_some() {
        return Err(CommandError::invalid(format!(
            "clip id {new_id} already exists"
        )));
    }
    check_no_overlap(
        ctx.sequence(&seq_id)?,
        &target.id,
        old.start,
        old.end(),
        &BTreeSet::new(),
    )?;
    let mut audio = old.clone();
    audio.id = new_id.clone();
    audio.track = target.id.clone();
    audio.group = None;
    audio.transition_in = None;
    audio.content = ClipContent::Media {
        asset: asset.clone(),
        has_video: false,
        has_audio: true,
    };
    // só o áudio herda o volume; o vídeo mantém as propriedades visuais
    let mut video = old.clone();
    video.content = ClipContent::Media {
        asset,
        has_video: true,
        has_audio: false,
    };
    video.properties.remove("volume_db");
    audio.properties.retain(|k, _| k == "volume_db");
    commit_clip(ctx, &seq_id, old, video)?;
    check_clip(&ctx.doc, ctx.sequence(&seq_id)?, &audio)?;
    ctx.insert_clip_op(&seq_id, audio)?;
    ctx.note_created(EntityKind::Clip, new_id.as_str());
    Ok(ctx.take_output(Some(new_id.0)))
}
