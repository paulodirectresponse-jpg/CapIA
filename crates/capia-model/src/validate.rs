//! Invariantes do documento (docs/TIMELINE_ENGINE.md §6), validadas após toda transação.
//!
//! A validação **nunca** corrige nada: devolve violações estruturadas (código, mensagem, entidades).

use crate::clip::{Clip, ClipContent, speed_in_range};
use crate::document::Document;
use crate::error::ErrorCode;
use crate::ids::{EntityKind, EntityRef, SequenceId};
use crate::property::property_spec;
use crate::sequence::{Sequence, TrackKind};
use capia_time::{MAX_TIMELINE_TICKS, Rational, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_TRACKS_PER_SEQUENCE: usize = 1_000;
pub const MAX_CLIPS_PER_SEQUENCE: usize = 100_000;
pub const MAX_NESTING_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub code: ErrorCode,
    pub message: String,
    pub entities: Vec<EntityRef>,
}

impl Violation {
    fn new(code: ErrorCode, message: impl Into<String>, entities: Vec<EntityRef>) -> Self {
        Self {
            code,
            message: message.into(),
            entities,
        }
    }
}

fn clip_ref(c: &Clip) -> EntityRef {
    EntityRef::new(EntityKind::Clip, c.id.as_str())
}

/// O conteúdo é compatível com a família da track (invariante 5).
pub fn content_fits_track(content: &ClipContent, kind: TrackKind) -> bool {
    match (content, kind) {
        (
            ClipContent::Media {
                has_video,
                has_audio,
                ..
            },
            TrackKind::Visual,
        ) => *has_video || !*has_audio,
        (
            ClipContent::Media {
                has_video,
                has_audio,
                ..
            },
            TrackKind::Audio,
        ) => !*has_video && *has_audio,
        (ClipContent::Nested { .. }, _) => true,
        (_, TrackKind::Visual) => true,
        (_, TrackKind::Audio) => false,
    }
}

/// Valida uma sequence (invariantes 1–5, 8 e 10). O grafo de nested é validado por
/// [`validate_nested_graph`].
pub fn validate_sequence(doc: &Document, seq_id: &SequenceId, seq: &Sequence) -> Vec<Violation> {
    let mut out = Vec::new();
    let seq_ref = EntityRef::new(EntityKind::Sequence, seq_id.as_str());
    if seq.tracks().len() > MAX_TRACKS_PER_SEQUENCE {
        out.push(Violation::new(
            ErrorCode::LimitExceeded,
            "too many tracks",
            vec![seq_ref.clone()],
        ));
    }
    if seq.clip_count() > MAX_CLIPS_PER_SEQUENCE {
        out.push(Violation::new(
            ErrorCode::LimitExceeded,
            "too many clips",
            vec![seq_ref],
        ));
    }

    for m in seq.markers() {
        if !m.time.within_timeline() {
            out.push(Violation::new(
                ErrorCode::OutOfRange,
                format!("marker {} outside the timeline", m.id),
                vec![EntityRef::new(EntityKind::Marker, m.id.as_str())],
            ));
        }
    }

    for track in seq.tracks() {
        let mut prev: Option<&Clip> = None;
        for clip in seq.track_clips(&track.id) {
            if let Some(p) = prev {
                if p.end() > clip.start {
                    out.push(Violation::new(
                        ErrorCode::Overlap,
                        format!(
                            "clip {} overlaps clip {} on track {}",
                            clip.id, p.id, track.id
                        ),
                        vec![clip_ref(p), clip_ref(clip)],
                    ));
                } else if track.magnetic && p.end() != clip.start {
                    out.push(Violation::new(
                        ErrorCode::GapInMagneticTrack,
                        format!("gap before clip {} on magnetic track {}", clip.id, track.id),
                        vec![clip_ref(clip)],
                    ));
                }
            } else if track.magnetic && clip.start != Ticks::ZERO {
                out.push(Violation::new(
                    ErrorCode::GapInMagneticTrack,
                    format!("magnetic track {} does not start at 0", track.id),
                    vec![clip_ref(clip)],
                ));
            }
            prev = Some(clip);
        }
    }

    for clip in seq.clips() {
        out.extend(validate_clip(doc, seq, clip));
    }
    out
}

/// Invariantes de **um** clip (3, 4, 5, 8 e 10): duração, limites, alinhamento (visual), família da
/// track, velocidade, fonte, referências e propriedades. Usada pelos comandos para validar o clip
/// resultante antes de commitar e por [`validate_sequence`].
pub fn validate_clip(doc: &Document, seq: &Sequence, clip: &Clip) -> Vec<Violation> {
    let mut out = Vec::new();
    let fr = seq.frame_rate();
    let me = vec![clip_ref(clip)];
    let Some(track) = seq.track(&clip.track) else {
        out.push(Violation::new(
            ErrorCode::DanglingReference,
            format!("clip {} references missing track {}", clip.id, clip.track),
            me,
        ));
        return out;
    };
    if clip.duration <= Ticks::ZERO {
        out.push(Violation::new(
            ErrorCode::OutOfRange,
            format!("clip {} has non-positive duration", clip.id),
            me.clone(),
        ));
    }
    if clip.start < Ticks::ZERO || clip.start.0.saturating_add(clip.duration.0) > MAX_TIMELINE_TICKS
    {
        out.push(Violation::new(
            ErrorCode::OutOfRange,
            format!("clip {} is outside [0, 24h]", clip.id),
            me.clone(),
        ));
    }
    if track.kind == TrackKind::Visual
        && !(fr.is_aligned(clip.start) && fr.is_aligned(clip.duration))
    {
        out.push(Violation::new(
            ErrorCode::NotFrameAligned,
            format!(
                "clip {} on visual track {} is not frame-aligned",
                clip.id, track.id
            ),
            me.clone(),
        ));
    }
    if !content_fits_track(&clip.content, track.kind) {
        out.push(Violation::new(
            ErrorCode::WrongTrackKind,
            format!(
                "clip {} content does not fit {:?} track {}",
                clip.id, track.kind, track.id
            ),
            me.clone(),
        ));
    }
    if !clip.speed.is_positive() || !speed_in_range(clip.speed) {
        out.push(Violation::new(
            ErrorCode::OutOfRange,
            format!("clip {} speed {} outside [1/100, 100]", clip.id, clip.speed),
            me.clone(),
        ));
    }
    if (clip.speed != Rational::ONE || clip.reversed) && !clip.content.supports_retime() {
        out.push(Violation::new(
            ErrorCode::InvalidArgument,
            format!("clip {} content does not support speed/reverse", clip.id),
            me.clone(),
        ));
    }
    if clip.source_in < Ticks::ZERO {
        out.push(Violation::new(
            ErrorCode::InsufficientHandles,
            format!("clip {} source_in is negative", clip.id),
            me.clone(),
        ));
    }
    if let Some(asset_id) = clip.content.asset() {
        match doc.asset(asset_id) {
            None => out.push(Violation::new(
                ErrorCode::DanglingReference,
                format!("clip {} references missing asset {}", clip.id, asset_id),
                me.clone(),
            )),
            Some(asset) => {
                if let (Some(len), ClipContent::Media { .. }) = (asset.duration, &clip.content)
                    && !clip.fits_source(len)
                {
                    out.push(Violation::new(
                        ErrorCode::InsufficientHandles,
                        format!(
                            "clip {} consumes more source than asset {} has",
                            clip.id, asset_id
                        ),
                        me.clone(),
                    ));
                }
            }
        }
    }
    if let ClipContent::Nested { sequence } = &clip.content
        && doc.sequence(sequence).is_none()
    {
        out.push(Violation::new(
            ErrorCode::DanglingReference,
            format!("clip {} references missing sequence {}", clip.id, sequence),
            me.clone(),
        ));
    }
    for (name, value) in &clip.properties {
        let Some(spec) = property_spec(name) else {
            out.push(Violation::new(
                ErrorCode::InvalidArgument,
                format!("clip {} has unknown property {name}", clip.id),
                me.clone(),
            ));
            return out;
        };
        if let Err(why) = value.check_structure() {
            out.push(Violation::new(
                ErrorCode::InvalidArgument,
                format!("clip {} property {name}: {why}", clip.id),
                me.clone(),
            ));
            return out;
        }
        let values: Vec<f64> = match value {
            crate::property::Animatable::Static(v) => vec![*v],
            crate::property::Animatable::Animated(k) => k.iter().map(|k| k.value).collect(),
        };
        if values.iter().any(|v| !spec.contains(*v)) {
            out.push(Violation::new(
                ErrorCode::OutOfRange,
                format!(
                    "clip {} property {name} outside [{}, {}]",
                    clip.id, spec.min, spec.max
                ),
                me.clone(),
            ));
        }
    }
    out
}

/// Invariante 6: grafo de `Nested` acíclico e com profundidade ≤ 16.
pub fn validate_nested_graph(doc: &Document) -> Vec<Violation> {
    let mut edges: BTreeMap<&SequenceId, BTreeSet<&SequenceId>> = BTreeMap::new();
    for (id, seq) in doc.sequences() {
        let e = edges.entry(id).or_default();
        for c in seq.clips() {
            if let ClipContent::Nested { sequence } = &c.content
                && let Some((target, _)) = doc.sequences().find(|(sid, _)| *sid == sequence)
            {
                e.insert(target);
            }
        }
    }
    let mut out = Vec::new();
    // DFS com cores; `depth[n]` = comprimento do maior caminho a partir de n.
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        Gray,
        Black,
    }
    fn visit<'a>(
        n: &'a SequenceId,
        edges: &BTreeMap<&'a SequenceId, BTreeSet<&'a SequenceId>>,
        color: &mut BTreeMap<&'a SequenceId, Color>,
        depth: &mut BTreeMap<&'a SequenceId, usize>,
        out: &mut Vec<Violation>,
    ) -> usize {
        match color.get(n) {
            Some(Color::Black) => return depth[n],
            Some(Color::Gray) => {
                out.push(Violation::new(
                    ErrorCode::NestedCycle,
                    format!("nested sequences form a cycle through {n}"),
                    vec![EntityRef::new(EntityKind::Sequence, n.as_str())],
                ));
                return 0;
            }
            None => {}
        }
        color.insert(n, Color::Gray);
        let mut best = 0;
        for next in edges.get(n).into_iter().flatten() {
            best = best.max(1 + visit(next, edges, color, depth, out));
        }
        color.insert(n, Color::Black);
        depth.insert(n, best);
        best
    }
    let (mut color, mut depth) = (BTreeMap::new(), BTreeMap::new());
    let ids: Vec<&SequenceId> = edges.keys().copied().collect();
    for id in ids {
        let d = visit(id, &edges, &mut color, &mut depth, &mut out);
        if d > MAX_NESTING_DEPTH {
            out.push(Violation::new(
                ErrorCode::NestedDepth,
                format!("nesting depth {d} from {id} exceeds {MAX_NESTING_DEPTH}"),
                vec![EntityRef::new(EntityKind::Sequence, id.as_str())],
            ));
        }
    }
    out
}

/// Valida o documento inteiro (todas as sequences + grafo de nested).
pub fn validate_document(doc: &Document) -> Vec<Violation> {
    let mut out: Vec<Violation> = doc
        .sequences()
        .flat_map(|(id, seq)| validate_sequence(doc, id, seq))
        .collect();
    out.extend(validate_nested_graph(doc));
    out
}
