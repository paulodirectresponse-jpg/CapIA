//! Composição de sequences (ADR-050): `duplicate_sequence`, `make_unique`, `flatten_nested`,
//! `create_nested_from_selection` e `generate_variants`. Tudo é feito com ops primitivas na working
//! copy da transação ⇒ atômico, com undo/redo/persistência herdados do engine.
//!
//! **Ids deterministicos** (nada aleatório): o elemento `X` copiado para a sequence `S'` vira
//! `S'.X`; uma sequence descendente `D` copiada em profundidade para a raiz `R'` vira `R'~D`.

use super::check::{check_clip, track_locked};
use super::nested;
use crate::command::{VariantSpec, VariantSwap};
use crate::ctx::{CommandOutput, Ctx};
use crate::error::{CommandError, Result};
use capia_model::{
    AssetId, Clip, ClipContent, ClipId, EntityKind, EntityRef, ErrorCode, MAX_TRACKS_PER_SEQUENCE,
    Marker, MarkerId, PrimitiveOp, Sequence, SequenceHeader, SequenceId, Track, TrackId, TrackKind,
    TrackSlot,
};
use capia_time::Ticks;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// Teto do comprimento de qualquer id derivado.
const MAX_ID_LEN: usize = 256;
/// Sequences descendentes copiadas por um único comando em modo `deep`.
const MAX_DEEP_COPIES: usize = 64;
/// Variantes por comando e trocas por variante.
const MAX_VARIANTS: usize = 100;
const MAX_SWAPS: usize = 256;
/// Clips por `create_nested_from_selection`.
const MAX_SELECTION: usize = 5_000;

fn derived(prefix: &str, sep: char, old: &str) -> Result<String> {
    let id = format!("{prefix}{sep}{old}");
    if id.len() > MAX_ID_LEN {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            format!("derived id exceeds {MAX_ID_LEN} characters"),
        ));
    }
    Ok(id)
}

fn seq_entity(id: &SequenceId) -> EntityRef {
    EntityRef::new(EntityKind::Sequence, id.as_str())
}

fn clip_entity(id: &ClipId) -> EntityRef {
    EntityRef::new(EntityKind::Clip, id.as_str())
}

fn fresh_sequence_id(ctx: &mut Ctx, id: Option<&SequenceId>) -> Result<SequenceId> {
    let id = id
        .cloned()
        .unwrap_or_else(|| SequenceId(ctx.derive_id("seq")));
    if ctx.doc.sequence(&id).is_some() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            format!("sequence {id} already exists"),
        )
        .with_entities([seq_entity(&id)]));
    }
    Ok(id)
}

// ---------------------------------------------------------------------------------------------
// cópia de sequences
// ---------------------------------------------------------------------------------------------

/// Copia UMA sequence para `new_id`; clips `Nested` são apontados para `remap[alvo]` quando existir
/// (senão continuam compartilhando a filha original).
fn copy_one(
    ctx: &mut Ctx,
    source: &SequenceId,
    new_id: &SequenceId,
    name: &str,
    remap: &BTreeMap<SequenceId, SequenceId>,
) -> Result<()> {
    let src: Sequence = ctx.sequence(source)?.clone();
    ctx.emit(PrimitiveOp::Sequence {
        id: new_id.clone(),
        old: None,
        new: Some(SequenceHeader {
            name: name.to_owned(),
            ..src.header.clone()
        }),
    })?;
    ctx.note_created(EntityKind::Sequence, new_id.as_str());
    let mut track_map: BTreeMap<TrackId, TrackId> = BTreeMap::new();
    for (index, t) in src.tracks().iter().enumerate() {
        let tid = TrackId(derived(new_id.as_str(), '.', t.id.as_str())?);
        if ctx.doc.find_track(&tid).is_some() {
            return Err(CommandError::invalid(format!("track {tid} already exists")));
        }
        let mut track = t.clone();
        track.id = tid.clone();
        ctx.emit(PrimitiveOp::Track {
            sequence: new_id.clone(),
            id: tid.clone(),
            old: None,
            new: Some(TrackSlot { index, track }),
        })?;
        track_map.insert(t.id.clone(), tid);
    }
    let mut clips: Vec<Clip> = src.clips().cloned().collect();
    clips.sort_by(|a, b| (&a.track, a.start, &a.id).cmp(&(&b.track, b.start, &b.id)));
    for c in clips {
        let cid = ClipId(derived(new_id.as_str(), '.', c.id.as_str())?);
        if ctx.doc.find_clip(&cid).is_some() {
            return Err(CommandError::invalid(format!("clip {cid} already exists"))
                .with_entities([clip_entity(&cid)]));
        }
        let mut copy = c.clone();
        copy.id = cid;
        copy.track = track_map.get(&c.track).cloned().ok_or_else(|| {
            CommandError::new(ErrorCode::InvariantViolation, "clip on unknown track")
        })?;
        if let ClipContent::Nested { sequence, .. } = &mut copy.content
            && let Some(new_target) = remap.get(sequence)
        {
            *sequence = new_target.clone();
        }
        nested_edge_ok(ctx, new_id, &copy)?;
        ctx.insert_clip_op(new_id, copy)?;
    }
    let markers: Vec<Marker> = src.markers().cloned().collect();
    for m in markers {
        let mid = MarkerId(derived(new_id.as_str(), '.', m.id.as_str())?);
        ctx.emit(PrimitiveOp::Marker {
            sequence: new_id.clone(),
            id: mid.clone(),
            old: None,
            new: Some(Marker { id: mid, ..m }),
        })?;
    }
    Ok(())
}

/// Aresta `pai → alvo` de um clip nested precisa manter DAG e profundidade (antes de escrever).
fn nested_edge_ok(ctx: &Ctx, parent: &SequenceId, clip: &Clip) -> Result<()> {
    if let ClipContent::Nested { sequence, .. } = &clip.content {
        nested::guard_edge(ctx, parent, sequence)?;
    }
    Ok(())
}

/// Descendentes (transitivos) de `root`, em pós-ordem (filhas antes das mães), sem repetição.
fn descendants_post_order(ctx: &Ctx, root: &SequenceId) -> Result<Vec<SequenceId>> {
    fn visit(
        ctx: &Ctx,
        id: &SequenceId,
        seen: &mut BTreeSet<SequenceId>,
        out: &mut Vec<SequenceId>,
    ) -> Result<()> {
        let targets: BTreeSet<SequenceId> = ctx
            .sequence(id)?
            .nested_refs()
            .map(|(_, n)| n.target.clone())
            .collect();
        for t in targets {
            if seen.insert(t.clone()) {
                visit(ctx, &t, seen, out)?;
                out.push(t);
            }
        }
        Ok(())
    }
    let mut seen = BTreeSet::from([root.clone()]);
    let mut out = Vec::new();
    visit(ctx, root, &mut seen, &mut out)?;
    Ok(out)
}

/// Copia `source` (e, com `deep`, toda a subárvore de nested) e devolve o mapa antigo → novo.
fn copy_tree(
    ctx: &mut Ctx,
    source: &SequenceId,
    new_root: &SequenceId,
    name: &str,
    deep: bool,
) -> Result<BTreeMap<SequenceId, SequenceId>> {
    let mut remap: BTreeMap<SequenceId, SequenceId> = BTreeMap::new();
    if deep {
        let desc = descendants_post_order(ctx, source)?;
        if desc.len() > MAX_DEEP_COPIES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "deep copy would duplicate {} sequences (max {MAX_DEEP_COPIES})",
                    desc.len()
                ),
            ));
        }
        for d in desc {
            let nid = SequenceId(derived(new_root.as_str(), '~', d.as_str())?);
            if ctx.doc.sequence(&nid).is_some() {
                return Err(
                    CommandError::invalid(format!("sequence {nid} already exists"))
                        .with_entities([seq_entity(&nid)]),
                );
            }
            let dname = ctx.sequence(&d)?.header.name.clone();
            copy_one(ctx, &d, &nid, &dname, &remap)?;
            remap.insert(d, nid);
        }
    }
    copy_one(ctx, source, new_root, name, &remap)?;
    remap.insert(source.clone(), new_root.clone());
    Ok(remap)
}

fn copy_name(ctx: &Ctx, source: &SequenceId, name: Option<&str>) -> Result<String> {
    Ok(match name {
        Some(n) => n.to_owned(),
        None => format!("{} copy", ctx.sequence(source)?.header.name),
    })
}

pub(crate) fn duplicate_sequence(
    ctx: &mut Ctx,
    source: &SequenceId,
    new_sequence: Option<&SequenceId>,
    name: Option<&str>,
    deep: bool,
) -> Result<CommandOutput> {
    ctx.sequence(source)?;
    let new_id = fresh_sequence_id(ctx, new_sequence)?;
    let name = copy_name(ctx, source, name)?;
    copy_tree(ctx, source, &new_id, &name, deep)?;
    Ok(ctx.take_output(Some(new_id.0)))
}

pub(crate) fn make_unique(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    new_sequence: Option<&SequenceId>,
    name: Option<&str>,
    deep: bool,
) -> Result<CommandOutput> {
    let (_, _, _, target, _) = nested::nested_clip(ctx, clip_id)?;
    let new_id = fresh_sequence_id(ctx, new_sequence)?;
    let name = copy_name(ctx, &target, name)?;
    copy_tree(ctx, &target, &new_id, &name, deep)?;
    // o clip passa a apontar para a cópia (aresta nova validada e sem a antiga)
    nested::set_nested_target(ctx, clip_id, &new_id)?;
    Ok(ctx.take_output(Some(new_id.0)))
}

// ---------------------------------------------------------------------------------------------
// flatten_nested
// ---------------------------------------------------------------------------------------------

pub(crate) fn flatten_nested(
    ctx: &mut Ctx,
    clip_id: &ClipId,
    prefix: Option<&str>,
) -> Result<CommandOutput> {
    let (parent, clip, track, child_id, _) = nested::nested_clip(ctx, clip_id)?;
    let unsupported = |why: &str| {
        CommandError::new(
            ErrorCode::UnsupportedCommand,
            format!("cannot flatten {clip_id}: {why}"),
        )
        .with_entities([clip_entity(clip_id)])
    };
    if track.magnetic {
        return Err(unsupported(
            "its track is magnetic (ripple semantics are undefined)",
        ));
    }
    if clip.speed != capia_time::Rational::ONE || clip.reversed {
        return Err(unsupported(
            "retimed or reversed nested clips cannot be baked",
        ));
    }
    if !clip.properties.is_empty() {
        return Err(unsupported(
            "the nested clip has its own properties/keyframes, which cannot be baked into the children",
        ));
    }
    let child = ctx.sequence(&child_id)?.clone();
    let parent_fr = ctx.sequence(&parent)?.frame_rate();
    if child.frame_rate() != parent_fr {
        return Err(unsupported("the child sequence has a different frame rate"));
    }
    if !parent_fr.is_aligned(clip.source_in) {
        return Err(unsupported("source_in is not frame-aligned"));
    }
    let prefix = prefix.unwrap_or(clip_id.as_str()).to_owned();
    // janela do filho visível através do clip: [w0, w1)
    let w0 = clip.source_in;
    let w1 = clip.source_in.checked_add(clip.duration)?;

    // novas tracks (livres) tomam o lugar do track do clip na ordem de empilhamento
    let base_index = ctx
        .sequence(&parent)?
        .track_position(&track.id)
        .ok_or_else(|| CommandError::not_found("track", &track.id))?;
    if ctx.sequence(&parent)?.tracks().len() + child.tracks().len() > MAX_TRACKS_PER_SEQUENCE {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "too many tracks",
        ));
    }
    let mut track_map: BTreeMap<TrackId, TrackId> = BTreeMap::new();
    for (i, t) in child.tracks().iter().enumerate() {
        let tid = TrackId(derived(&prefix, '.', t.id.as_str())?);
        if ctx.doc.find_track(&tid).is_some() {
            return Err(CommandError::invalid(format!("track {tid} already exists")));
        }
        let mut nt = t.clone();
        nt.id = tid.clone();
        nt.magnetic = false;
        nt.name = if t.name.is_empty() {
            clip.name.clone()
        } else {
            format!("{}/{}", clip.name, t.name)
        };
        ctx.emit(PrimitiveOp::Track {
            sequence: parent.clone(),
            id: tid.clone(),
            old: None,
            new: Some(TrackSlot {
                index: base_index + i,
                track: nt,
            }),
        })?;
        ctx.note_created(EntityKind::Track, tid.as_str());
        track_map.insert(t.id.clone(), tid);
    }

    let mut kids: Vec<Clip> = child.clips().cloned().collect();
    kids.sort_by(|a, b| (&a.track, a.start, &a.id).cmp(&(&b.track, b.start, &b.id)));
    let mut first_id: Option<String> = None;
    let mut cut = 0usize;
    for k in kids {
        let lo = k.start.max(w0);
        let hi = k.end().min(w1);
        if lo >= hi {
            continue;
        }
        let (a, b) = (lo.checked_sub(k.start)?, k.end().checked_sub(hi)?);
        let mut copy = k.clone();
        copy.id = ClipId(derived(&prefix, '.', k.id.as_str())?);
        if ctx.doc.find_clip(&copy.id).is_some() {
            return Err(CommandError::invalid(format!(
                "clip {} already exists",
                copy.id
            )));
        }
        copy.track = track_map.get(&k.track).cloned().ok_or_else(|| {
            CommandError::new(ErrorCode::InvariantViolation, "clip on unknown track")
        })?;
        copy.start = clip.start.checked_add(lo.checked_sub(w0)?)?;
        copy.duration = hi.checked_sub(lo)?;
        copy.enabled = k.enabled && clip.enabled;
        if a.0 > 0 || b.0 > 0 {
            cut += 1;
            trim_content(&mut copy, &k, a, b)?;
        }
        nested_edge_ok(ctx, &parent, &copy)?;
        let id = copy.id.clone();
        ctx.insert_clip_op(&parent, copy)?;
        ctx.note_created(EntityKind::Clip, id.as_str());
        first_id.get_or_insert(id.0);
    }
    // o clip nested sai (sem ripple: a track é livre e o tempo dos demais não muda)
    ctx.remove_clip_op(&parent, clip.clone())?;
    if child.marker_count() > 0 {
        ctx.warnings.push(format!(
            "markers of sequence {child_id} were not copied into the parent"
        ));
    }
    if cut > 0 {
        ctx.warnings.push(format!(
            "{cut} clip(s) were trimmed to the window visible through the nested clip"
        ));
    }
    // valida os clips criados com as regras do pai (alinhamento, handles, sobreposição por track)
    let seq = ctx.sequence(&parent)?.clone();
    for t in track_map.values() {
        for c in seq.track_clips(t) {
            check_clip(&ctx.doc, &seq, c)?;
        }
    }
    Ok(ctx.take_output(first_id))
}

/// Recorta `a` à esquerda e `b` à direita de um clip já reposicionado, mantendo o mapeamento
/// conteúdo↔tempo exato (mesma regra de `split_clip`): clips com fonte externa preservam os
/// keyframes (tempo de conteúdo); os demais rebaseiam o tempo local.
fn trim_content(copy: &mut Clip, original: &Clip, a: Ticks, b: Ticks) -> Result<()> {
    let (n, d) = (original.speed.num(), original.speed.den());
    if original.reversed {
        copy.source_in = original.source_in.checked_add(b.mul_div_round(n, d)?)?;
    } else {
        copy.source_in = original.source_in.checked_add(a.mul_div_round(n, d)?)?;
    }
    if !original.content.has_source_time() {
        copy.source_in = Ticks::ZERO;
        let mut props = original.properties.clone();
        for (name, anim) in &original.properties {
            let mut cur = anim.clone();
            if a.0 > 0 {
                cur = cur.split_at(a, true).right;
            }
            if b.0 > 0 {
                let keep = original.duration.checked_sub(a)?.checked_sub(b)?;
                cur = cur.split_at(keep, false).left;
            }
            props.insert(name.clone(), cur);
        }
        copy.properties = props;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// create_nested_from_selection
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_nested_from_selection(
    ctx: &mut Ctx,
    clips: &[ClipId],
    new_sequence: Option<&SequenceId>,
    name: Option<&str>,
    clip_id: Option<&ClipId>,
    target_track: Option<&TrackId>,
    follow_length: bool,
) -> Result<CommandOutput> {
    if clips.is_empty() {
        return Err(CommandError::invalid("the selection is empty"));
    }
    if clips.len() > MAX_SELECTION {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            format!("selection exceeds {MAX_SELECTION} clips"),
        ));
    }
    let unique: BTreeSet<&ClipId> = clips.iter().collect();
    if unique.len() != clips.len() {
        return Err(CommandError::invalid(
            "the selection contains the same clip twice",
        ));
    }
    let mut parent: Option<SequenceId> = None;
    let mut selected: Vec<Clip> = Vec::with_capacity(clips.len());
    for id in clips {
        let (sid, clip, track) = ctx.locate_clip(id)?;
        track_locked(&track)?;
        if track.magnetic {
            return Err(CommandError::new(
                ErrorCode::UnsupportedCommand,
                format!("clip {id} is on a magnetic track (ripple semantics are undefined)"),
            )
            .with_entities([clip_entity(id)]));
        }
        match &parent {
            None => parent = Some(sid),
            Some(p) if *p != sid => {
                return Err(CommandError::invalid(
                    "all selected clips must belong to the same sequence",
                )
                .with_entities([clip_entity(id)]));
            }
            _ => {}
        }
        selected.push(clip);
    }
    let parent = parent.ok_or_else(|| CommandError::invalid("the selection is empty"))?;
    let pseq = ctx.sequence(&parent)?.clone();
    let fr = pseq.frame_rate();
    let t0 = selected
        .iter()
        .map(|c| c.start)
        .min()
        .unwrap_or(Ticks::ZERO);
    let t1 = selected.iter().map(Clip::end).max().unwrap_or(Ticks::ZERO);
    // base alinhada ao frame (o filho nasce em 0): preserva o alinhamento dos clips visuais
    let base = fr.align_floor(t0);

    let new_id = fresh_sequence_id(ctx, new_sequence)?;
    let nested_id = match clip_id {
        Some(c) => c.clone(),
        None => ClipId(ctx.derive_id("clip")),
    };
    if ctx.doc.find_clip(&nested_id).is_some() {
        return Err(CommandError::invalid(format!(
            "clip {nested_id} already exists"
        )));
    }
    // track de destino do nested: a de cima (primeira na ordem) entre as dos clips, ou a pedida
    let used_tracks: BTreeSet<TrackId> = selected.iter().map(|c| c.track.clone()).collect();
    let ordered: Vec<&Track> = pseq
        .tracks()
        .iter()
        .filter(|t| used_tracks.contains(&t.id))
        .collect();
    let host = match target_track {
        Some(t) => t.clone(),
        None => ordered
            .iter()
            .find(|t| t.kind == TrackKind::Visual)
            .or_else(|| ordered.first())
            .map(|t| t.id.clone())
            .ok_or_else(|| CommandError::invalid("no track for the nested clip"))?,
    };
    // o nested tem de caber depois da remoção dos clips selecionados
    let host_track = pseq
        .track(&host)
        .ok_or_else(|| CommandError::not_found("track", &host))?
        .clone();
    track_locked(&host_track)?;
    if host_track.magnetic {
        return Err(CommandError::new(
            ErrorCode::UnsupportedCommand,
            "the target track is magnetic",
        ));
    }
    let header = SequenceHeader {
        name: name.map_or_else(|| format!("Nested {}", new_id.as_str()), str::to_owned),
        ..pseq.header.clone()
    };
    ctx.emit(PrimitiveOp::Sequence {
        id: new_id.clone(),
        old: None,
        new: Some(header.clone()),
    })?;
    ctx.note_created(EntityKind::Sequence, new_id.as_str());
    let mut track_map: BTreeMap<TrackId, TrackId> = BTreeMap::new();
    for (i, t) in ordered.iter().enumerate() {
        let tid = TrackId(derived(new_id.as_str(), '.', t.id.as_str())?);
        if ctx.doc.find_track(&tid).is_some() {
            return Err(CommandError::invalid(format!("track {tid} already exists")));
        }
        let mut nt = (*t).clone();
        nt.id = tid.clone();
        nt.magnetic = false;
        nt.locked = false;
        ctx.emit(PrimitiveOp::Track {
            sequence: new_id.clone(),
            id: tid.clone(),
            old: None,
            new: Some(TrackSlot {
                index: i,
                track: nt,
            }),
        })?;
        track_map.insert(t.id.clone(), tid);
    }
    // move os clips (mesmo ClipId): remove do pai e insere no filho, com o tempo relativo preservado
    selected.sort_by(|a, b| (&a.track, a.start, &a.id).cmp(&(&b.track, b.start, &b.id)));
    for c in &selected {
        ctx.remove_clip_op(&parent, c.clone())?;
    }
    for c in selected {
        let mut moved = c.clone();
        moved.track = track_map
            .get(&c.track)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::InvariantViolation, "unknown track"))?;
        moved.start = c.start.checked_sub(base)?;
        nested_edge_ok(ctx, &new_id, &moved)?;
        ctx.insert_clip_op(&new_id, moved)?;
    }
    // o nested ocupa [base, t1) na track escolhida
    let duration = {
        let raw = t1.checked_sub(base)?;
        match host_track.kind {
            TrackKind::Visual => fr.align_duration_half_up(raw)?,
            TrackKind::Audio => raw.max(Ticks(1)),
        }
    };
    let child_name = header.name;
    nested::insert_nested(
        ctx,
        &host,
        base,
        &new_id,
        Some(&nested_id),
        &child_name,
        Some(duration),
        Ticks::ZERO,
        follow_length,
        false,
        None,
    )?;
    ctx.note_created(EntityKind::Clip, nested_id.as_str());
    Ok(ctx.take_output(Some(new_id.0)))
}

// ---------------------------------------------------------------------------------------------
// generate_variants (estrutura de timeline; NÃO é geração por IA)
// ---------------------------------------------------------------------------------------------

fn replace_media(ctx: &mut Ctx, clip_id: &ClipId, asset_id: &AssetId) -> Result<()> {
    let (seq, clip, track) = ctx.locate_clip(clip_id)?;
    track_locked(&track)?;
    let asset = ctx
        .doc
        .asset(asset_id)
        .ok_or_else(|| CommandError::not_found("asset", asset_id))?
        .clone();
    let mut new = clip.clone();
    match &mut new.content {
        ClipContent::Media {
            asset,
            has_video,
            has_audio,
        } => {
            let doc_asset = ctx
                .doc
                .asset(asset_id)
                .ok_or_else(|| CommandError::not_found("asset", asset_id))?;
            if (*has_video && !doc_asset.has_video) || (*has_audio && !doc_asset.has_audio) {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    format!("asset {asset_id} does not have the streams clip {clip_id} uses"),
                )
                .with_entities([clip_entity(clip_id)]));
            }
            *asset = asset_id.clone();
        }
        ClipContent::Image { asset } => {
            if !asset_has_picture(&ctx.doc, asset_id) {
                return Err(CommandError::invalid(format!(
                    "asset {asset_id} has no picture for image clip {clip_id}"
                )));
            }
            *asset = asset_id.clone();
        }
        _ => {
            return Err(
                CommandError::invalid(format!("clip {clip_id} has no media to replace"))
                    .with_entities([clip_entity(clip_id)]),
            );
        }
    }
    let _ = asset;
    check_clip(&ctx.doc, ctx.sequence(&seq)?, &new)?;
    ctx.replace_clip_op(&seq, clip, new)
}

fn asset_has_picture(doc: &capia_model::Document, id: &AssetId) -> bool {
    doc.asset(id).is_some_and(|a| a.has_video)
}

pub(crate) fn generate_variants(
    ctx: &mut Ctx,
    template: &SequenceId,
    variants: &[VariantSpec],
) -> Result<CommandOutput> {
    ctx.sequence(template)?;
    if variants.is_empty() {
        return Err(CommandError::invalid("no variants requested"));
    }
    if variants.len() > MAX_VARIANTS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            format!("more than {MAX_VARIANTS} variants in one command"),
        ));
    }
    let mut first: Option<String> = None;
    for (i, v) in variants.iter().enumerate() {
        if v.swaps.len() > MAX_SWAPS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                format!("variant {i} has more than {MAX_SWAPS} swaps"),
            ));
        }
        let new_id = fresh_sequence_id(ctx, v.sequence.as_ref())
            .map_err(|e| e.with_hint(json!({ "variant_index": i })))?;
        let name = match &v.name {
            Some(n) => n.clone(),
            None => format!("{} v{}", ctx.sequence(template)?.header.name, i + 1),
        };
        copy_tree(ctx, template, &new_id, &name, v.deep)?;
        for swap in &v.swaps {
            match swap {
                VariantSwap::SetNested { clip, sequence } => {
                    let copy = ClipId(derived(new_id.as_str(), '.', clip.as_str())?);
                    ctx.sequence(&new_id)?
                        .clip(&copy)
                        .ok_or_else(|| {
                            CommandError::not_found("clip", clip)
                                .with_hint(json!({ "variant_index": i, "note": "swaps name clips of the TEMPLATE" }))
                        })?;
                    nested::set_nested_target(ctx, &copy, sequence)?;
                }
                VariantSwap::ReplaceMedia { clip, asset } => {
                    let copy = ClipId(derived(new_id.as_str(), '.', clip.as_str())?);
                    ctx.sequence(&new_id)?
                        .clip(&copy)
                        .ok_or_else(|| {
                            CommandError::not_found("clip", clip)
                                .with_hint(json!({ "variant_index": i, "note": "swaps name clips of the TEMPLATE" }))
                        })?;
                    replace_media(ctx, &copy, asset)?;
                }
            }
        }
        first.get_or_insert(new_id.0);
    }
    Ok(ctx.take_output(first))
}
