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
/// Teto da duração de uma transição (30 s em ticks).
pub const MAX_TRANSITION_TICKS: Ticks = Ticks(30 * capia_time::TICKS_PER_SECOND);

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
    if let ClipContent::Text { style, .. } = &clip.content {
        if let Err(why) = style.validate() {
            out.push(Violation::new(
                ErrorCode::OutOfRange,
                format!("clip {} text style: {why}", clip.id),
                me.clone(),
            ));
        }
    }
    if let Some(tr) = &clip.transition_in {
        if track.kind != TrackKind::Visual {
            out.push(Violation::new(
                ErrorCode::WrongTrackKind,
                format!("clip {} has a transition on a non-visual track", clip.id),
                me.clone(),
            ));
        }
        if tr.duration <= Ticks::ZERO || tr.duration > MAX_TRANSITION_TICKS {
            out.push(Violation::new(
                ErrorCode::OutOfRange,
                format!("clip {} transition duration outside (0, 30s]", clip.id),
                me.clone(),
            ));
        }
    }
    if clip
        .group
        .as_ref()
        .is_some_and(|g| g.is_empty() || g.len() > 64)
    {
        out.push(Violation::new(
            ErrorCode::OutOfRange,
            format!("clip {} group label must have 1..=64 characters", clip.id),
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
    if let ClipContent::Nested { sequence, .. } = &clip.content
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

/// Arestas `pai → filho` do grafo de nested, a partir do índice de cada sequence (O(#nested)).
fn nested_edges(doc: &Document) -> BTreeMap<&SequenceId, BTreeSet<&SequenceId>> {
    let mut edges: BTreeMap<&SequenceId, BTreeSet<&SequenceId>> = BTreeMap::new();
    for (id, seq) in doc.sequences() {
        let e = edges.entry(id).or_default();
        for (_, n) in seq.nested_refs() {
            if let Some((target, _)) = doc.sequences().find(|(sid, _)| *sid == &n.target) {
                e.insert(target);
            }
        }
    }
    edges
}

/// Caminho `from → … → to` seguindo nested (inclui as pontas), se existir. `from == to` ⇒ `[from]`.
pub fn nested_path(doc: &Document, from: &SequenceId, to: &SequenceId) -> Option<Vec<SequenceId>> {
    let edges = nested_edges(doc);
    fn dfs<'a>(
        n: &'a SequenceId,
        to: &SequenceId,
        edges: &BTreeMap<&'a SequenceId, BTreeSet<&'a SequenceId>>,
        seen: &mut BTreeSet<&'a SequenceId>,
        path: &mut Vec<SequenceId>,
    ) -> bool {
        path.push(n.clone());
        if n == to {
            return true;
        }
        if seen.insert(n) {
            for next in edges.get(n).into_iter().flatten() {
                if dfs(next, to, edges, seen, path) {
                    return true;
                }
            }
        }
        path.pop();
        false
    }
    let start = doc
        .sequences()
        .find(|(id, _)| *id == from)
        .map(|(id, _)| id)?;
    let mut path = Vec::new();
    dfs(start, to, &edges, &mut BTreeSet::new(), &mut path).then_some(path)
}

/// Maior número de saltos descendo a partir de `seq` (0 = sem filhos). Protegido contra ciclos.
pub fn nested_depth_below(doc: &Document, seq: &SequenceId) -> usize {
    let edges = nested_edges(doc);
    fn go<'a>(
        n: &'a SequenceId,
        edges: &BTreeMap<&'a SequenceId, BTreeSet<&'a SequenceId>>,
        memo: &mut BTreeMap<&'a SequenceId, usize>,
        visiting: &mut BTreeSet<&'a SequenceId>,
    ) -> usize {
        if let Some(d) = memo.get(n) {
            return *d;
        }
        if !visiting.insert(n) {
            return 0;
        }
        let d = edges
            .get(n)
            .into_iter()
            .flatten()
            .map(|c| 1 + go(c, edges, memo, visiting))
            .max()
            .unwrap_or(0);
        visiting.remove(n);
        memo.insert(n, d);
        d
    }
    let Some((start, _)) = doc.sequences().find(|(id, _)| *id == seq) else {
        return 0;
    };
    go(start, &edges, &mut BTreeMap::new(), &mut BTreeSet::new())
}

/// Maior número de saltos subindo até `seq` (0 = ninguém a referencia).
pub fn nested_depth_above(doc: &Document, seq: &SequenceId) -> usize {
    let edges = nested_edges(doc);
    let mut parents: BTreeMap<&SequenceId, BTreeSet<&SequenceId>> = BTreeMap::new();
    for (p, children) in &edges {
        for c in children {
            parents.entry(c).or_default().insert(p);
        }
    }
    fn go<'a>(
        n: &'a SequenceId,
        parents: &BTreeMap<&'a SequenceId, BTreeSet<&'a SequenceId>>,
        memo: &mut BTreeMap<&'a SequenceId, usize>,
        visiting: &mut BTreeSet<&'a SequenceId>,
    ) -> usize {
        if let Some(d) = memo.get(n) {
            return *d;
        }
        if !visiting.insert(n) {
            return 0;
        }
        let d = parents
            .get(n)
            .into_iter()
            .flatten()
            .map(|p| 1 + go(p, parents, memo, visiting))
            .max()
            .unwrap_or(0);
        visiting.remove(n);
        memo.insert(n, d);
        d
    }
    let Some((start, _)) = doc.sequences().find(|(id, _)| *id == seq) else {
        return 0;
    };
    go(start, &parents, &mut BTreeMap::new(), &mut BTreeSet::new())
}

/// Adicionar a aresta `parent → target` quebraria o DAG ou o limite de profundidade? Devolve a
/// violação (ciclo com o caminho em `message`, ou profundidade) **antes** de qualquer escrita.
pub fn check_nested_edge(
    doc: &Document,
    parent: &SequenceId,
    target: &SequenceId,
) -> Option<Violation> {
    let refs = vec![
        EntityRef::new(EntityKind::Sequence, parent.as_str()),
        EntityRef::new(EntityKind::Sequence, target.as_str()),
    ];
    if let Some(path) = nested_path(doc, target, parent) {
        let chain: Vec<&str> = path.iter().map(SequenceId::as_str).collect();
        return Some(Violation::new(
            ErrorCode::NestedCycle,
            format!(
                "nesting {parent} -> {target} would create a cycle: {parent} -> {}",
                chain.join(" -> ")
            ),
            refs,
        ));
    }
    let depth = nested_depth_above(doc, parent) + 1 + nested_depth_below(doc, target);
    (depth > MAX_NESTING_DEPTH).then(|| {
        Violation::new(
            ErrorCode::NestedDepth,
            format!("nesting {parent} -> {target} reaches depth {depth} > {MAX_NESTING_DEPTH}"),
            refs,
        )
    })
}

/// Invariante 6: grafo de `Nested` acíclico e com profundidade ≤ 16.
pub fn validate_nested_graph(doc: &Document) -> Vec<Violation> {
    let edges = nested_edges(doc);
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

/// Valida pastas, deliverables e a pasta de cada sequence.
fn validate_organization(doc: &Document) -> Vec<Violation> {
    let mut out = Vec::new();
    for f in doc.folders() {
        let me = vec![EntityRef::new(EntityKind::Folder, f.id.as_str())];
        if f.name.trim().is_empty() || f.name.len() > 128 {
            out.push(Violation::new(
                ErrorCode::OutOfRange,
                format!("folder {} name must have 1..=128 characters", f.id),
                me.clone(),
            ));
        }
        // pai existente e sem ciclo (a cadeia termina na raiz em ≤ nº de pastas passos)
        let mut cur = f.parent.clone();
        let mut steps = 0usize;
        while let Some(p) = cur {
            steps += 1;
            match doc.folder(&p) {
                None => {
                    out.push(Violation::new(
                        ErrorCode::DanglingReference,
                        format!("folder {} references missing parent {p}", f.id),
                        me.clone(),
                    ));
                    break;
                }
                Some(pf) => cur = pf.parent.clone(),
            }
            if steps > doc.folders.len() {
                out.push(Violation::new(
                    ErrorCode::InvariantViolation,
                    format!("folder {} is part of a parent cycle", f.id),
                    me.clone(),
                ));
                break;
            }
        }
    }
    for (id, seq) in doc.sequences() {
        let h = &seq.header;
        let me = vec![EntityRef::new(EntityKind::Sequence, id.as_str())];
        if let Some(fid) = &h.folder {
            if doc.folder(fid).is_none() {
                out.push(Violation::new(
                    ErrorCode::DanglingReference,
                    format!("sequence {id} references missing folder {fid}"),
                    me.clone(),
                ));
            }
        }
        if !(16..=16_384).contains(&h.width) || !(16..=16_384).contains(&h.height) {
            out.push(Violation::new(
                ErrorCode::OutOfRange,
                format!("sequence {id} frame size must be within 16..=16384"),
                me,
            ));
        }
    }
    for d in doc.deliverables() {
        let me = vec![EntityRef::new(EntityKind::Deliverable, d.id.as_str())];
        if doc.sequence(&d.sequence).is_none() {
            out.push(Violation::new(
                ErrorCode::DanglingReference,
                format!(
                    "deliverable {} references missing sequence {}",
                    d.id, d.sequence
                ),
                me.clone(),
            ));
        }
        if d.path.trim().is_empty() || d.name.trim().is_empty() || d.preset.trim().is_empty() {
            out.push(Violation::new(
                ErrorCode::InvalidArgument,
                format!("deliverable {} needs name, preset and path", d.id),
                me,
            ));
        }
    }
    out
}

/// Valida o documento inteiro (todas as sequences + grafo de nested + organização).
pub fn validate_document(doc: &Document) -> Vec<Violation> {
    let mut out: Vec<Violation> = doc
        .sequences()
        .flat_map(|(id, seq)| validate_sequence(doc, id, seq))
        .collect();
    out.extend(validate_nested_graph(doc));
    out.extend(validate_organization(doc));
    out
}
