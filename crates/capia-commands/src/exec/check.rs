//! Verificações de precondição compartilhadas pelos comandos de clip.

use crate::error::{CommandError, Result};
use capia_model::{
    Clip, ClipId, Document, EntityKind, EntityRef, ErrorCode, Sequence, Track, TrackId,
    validate_clip,
};
use capia_time::Ticks;
use serde_json::json;
use std::collections::BTreeSet;

pub(crate) fn track_locked(track: &Track) -> Result<()> {
    if track.locked {
        return Err(CommandError::new(
            ErrorCode::TrackLocked,
            format!("track {} is locked", track.id),
        )
        .with_entities([EntityRef::new(EntityKind::Track, track.id.as_str())]));
    }
    Ok(())
}

/// Invariantes do clip resultante; o primeiro `Violation` vira o erro do comando.
pub(crate) fn check_clip(doc: &Document, seq: &Sequence, clip: &Clip) -> Result<()> {
    match validate_clip(doc, seq, clip).first() {
        Some(v) => Err(CommandError::from_violation(v)),
        None => Ok(()),
    }
}

/// Faixas livres da track (até 8), em ticks, para a dica de `OVERLAP`.
fn free_ranges(seq: &Sequence, track: &TrackId, ignore: &BTreeSet<ClipId>) -> Vec<[i64; 2]> {
    let mut out = Vec::new();
    let mut cursor = 0;
    for c in seq.track_clips(track).filter(|c| !ignore.contains(&c.id)) {
        if c.start.0 > cursor {
            out.push([cursor, c.start.0]);
        }
        cursor = cursor.max(c.end().0);
        if out.len() >= 8 {
            return out;
        }
    }
    out.push([cursor, capia_time::MAX_TIMELINE_TICKS]);
    out
}

/// `[start, end)` não pode intersectar clips da track (exceto `ignore`).
pub(crate) fn check_no_overlap(
    seq: &Sequence,
    track: &TrackId,
    start: Ticks,
    end: Ticks,
    ignore: &BTreeSet<ClipId>,
) -> Result<()> {
    let hits: Vec<&Clip> = seq
        .clips_in(track, start, end)
        .into_iter()
        .filter(|c| !ignore.contains(&c.id))
        .collect();
    if hits.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = hits.iter().map(|c| c.id.as_str()).collect();
    Err(CommandError::new(
        ErrorCode::Overlap,
        format!("range would overlap clip {} on track {track}", ids[0]),
    )
    .with_entities(
        hits.iter()
            .map(|c| EntityRef::new(EntityKind::Clip, c.id.as_str())),
    )
    .with_hint(json!({
        "range_ticks": [start.0, end.0],
        "free_ranges": free_ranges(seq, track, ignore),
    })))
}
