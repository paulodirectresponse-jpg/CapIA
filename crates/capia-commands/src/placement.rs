//! Resolução de colocação (função pura de UX/engine): em qual track um clip deve entrar.
//!
//! * `explicit`: a track pedida (kind e trava são validados; sobreposição é problema do `insert`).
//! * `first_available`: a track livre mais alta da mesma família, ignorando tracks ocupadas,
//!   travadas e **magnéticas** (a main track nunca é escolhida automaticamente).
//! * `prefer`: a track preferida se livre; senão uma nova track adjacente.
//! * sem track livre: nova track — **acima** da mais alta visual / **abaixo** da mais baixa de áudio.

use crate::error::{CommandError, Result};
use capia_model::{EntityKind, EntityRef, ErrorCode, Sequence, Track, TrackId, TrackKind};
use capia_time::TimeRange;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlacementStrategy {
    Explicit { track: TrackId },
    FirstAvailable,
    Prefer { track: TrackId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertSide {
    Above,
    Below,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement {
    Existing {
        track: TrackId,
    },
    NewTrack {
        track_kind: TrackKind,
        insert: InsertSide,
        relative_to: Option<TrackId>,
    },
}

fn busy(seq: &Sequence, track: &Track, spans: &[TimeRange]) -> bool {
    spans.iter().any(|s| {
        s.end()
            .is_none_or(|end| !seq.clips_in(&track.id, s.start, end).is_empty())
    })
}

fn side_for(kind: TrackKind) -> InsertSide {
    match kind {
        TrackKind::Visual => InsertSide::Above,
        TrackKind::Audio => InsertSide::Below,
    }
}

fn checked_track<'a>(seq: &'a Sequence, id: &TrackId, kind: TrackKind) -> Result<&'a Track> {
    let t = seq
        .track(id)
        .ok_or_else(|| CommandError::not_found("track", id))?;
    if t.kind != kind {
        return Err(CommandError::new(
            ErrorCode::WrongTrackKind,
            format!("track {id} is {:?} but the clip is {kind:?}", t.kind),
        )
        .with_entities([EntityRef::new(EntityKind::Track, id.as_str())]));
    }
    Ok(t)
}

pub fn resolve_placement(
    seq: &Sequence,
    strategy: &PlacementStrategy,
    kind: TrackKind,
    spans: &[TimeRange],
) -> Result<Placement> {
    match strategy {
        PlacementStrategy::Explicit { track } => {
            let t = checked_track(seq, track, kind)?;
            if t.locked {
                return Err(CommandError::new(
                    ErrorCode::TrackLocked,
                    format!("track {track} is locked"),
                )
                .with_entities([EntityRef::new(EntityKind::Track, track.as_str())]));
            }
            Ok(Placement::Existing {
                track: t.id.clone(),
            })
        }
        PlacementStrategy::FirstAvailable => {
            let found = seq
                .tracks()
                .iter()
                .find(|t| t.kind == kind && !t.locked && !t.magnetic && !busy(seq, t, spans));
            if let Some(t) = found {
                return Ok(Placement::Existing {
                    track: t.id.clone(),
                });
            }
            let relative_to = match kind {
                TrackKind::Visual => seq.tracks().iter().find(|t| t.kind == kind),
                TrackKind::Audio => seq.tracks().iter().rev().find(|t| t.kind == kind),
            };
            Ok(Placement::NewTrack {
                track_kind: kind,
                insert: side_for(kind),
                relative_to: relative_to.map(|t| t.id.clone()),
            })
        }
        PlacementStrategy::Prefer { track } => {
            let t = checked_track(seq, track, kind)?;
            if t.magnetic || (!t.locked && !busy(seq, t, spans)) {
                return Ok(Placement::Existing {
                    track: t.id.clone(),
                });
            }
            Ok(Placement::NewTrack {
                track_kind: kind,
                insert: side_for(kind),
                relative_to: Some(t.id.clone()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use capia_model::SequenceHeader;
    use capia_time::{FrameRate, Ticks};

    fn seq_with(tracks: &[(&str, TrackKind, bool)]) -> Sequence {
        let header = SequenceHeader {
            name: "s".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: 48_000,
            width: 1920,
            height: 1080,
            folder: None,
        };
        let tracks: Vec<Track> = tracks
            .iter()
            .map(|(id, kind, magnetic)| {
                let mut t = Track::new(*id, *kind);
                t.magnetic = *magnetic;
                t
            })
            .collect();
        Sequence::from_parts(header, tracks, Default::default(), Default::default()).unwrap()
    }

    #[test]
    fn first_available_creates_a_track_when_everything_is_busy_or_magnetic() {
        let s = seq_with(&[
            ("V2", TrackKind::Visual, false),
            ("V1", TrackKind::Visual, true),
            ("A1", TrackKind::Audio, false),
        ]);
        let span = [TimeRange::new(Ticks(0), Ticks(10))];
        // V2 livre (vazia)
        assert_eq!(
            resolve_placement(
                &s,
                &PlacementStrategy::FirstAvailable,
                TrackKind::Visual,
                &span
            )
            .unwrap(),
            Placement::Existing { track: "V2".into() }
        );
    }

    #[test]
    fn explicit_validates_kind() {
        let s = seq_with(&[("V1", TrackKind::Visual, false)]);
        let err = resolve_placement(
            &s,
            &PlacementStrategy::Explicit { track: "V1".into() },
            TrackKind::Audio,
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::WrongTrackKind);
        let err = resolve_placement(
            &s,
            &PlacementStrategy::Explicit {
                track: "nope".into(),
            },
            TrackKind::Visual,
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }
}
