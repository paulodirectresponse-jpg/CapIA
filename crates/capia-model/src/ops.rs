//! Operações primitivas — o mecanismo de escrita, undo/redo e replay (docs/COMMAND_SYSTEM.md §1).
//!
//! Cada op carrega **o valor antigo e o novo** da entidade (`None` = não existia / deixa de
//! existir). Isso torna a inversa trivial (trocar os dois), permite verificar em `apply` que o
//! estado bate com o esperado (determinismo de replay/undo) e dá o conjunto `affected` em nível de
//! entidade para detecção de conflitos.

use crate::clip::Clip;
use crate::document::{Asset, Deliverable, Document, Folder, OpError};
use crate::error::ErrorCode;
use crate::ids::{
    AssetId, ClipId, DeliverableId, EntityKind, EntityRef, FolderId, MarkerId, SequenceId, TrackId,
};
use crate::sequence::{Marker, SequenceHeader, Track};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Uma track com a posição que ocupa na sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSlot {
    pub index: usize,
    pub track: Track,
}

// `Clip` carrega o clip inteiro (old/new): a diferença de tamanho é intencional (ops são raras).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PrimitiveOp {
    Clip {
        sequence: SequenceId,
        id: ClipId,
        old: Option<Clip>,
        new: Option<Clip>,
    },
    Track {
        sequence: SequenceId,
        id: TrackId,
        old: Option<TrackSlot>,
        new: Option<TrackSlot>,
    },
    Marker {
        sequence: SequenceId,
        id: MarkerId,
        old: Option<Marker>,
        new: Option<Marker>,
    },
    Sequence {
        id: SequenceId,
        old: Option<SequenceHeader>,
        new: Option<SequenceHeader>,
    },
    Asset {
        id: AssetId,
        old: Option<Asset>,
        new: Option<Asset>,
    },
    Folder {
        id: FolderId,
        old: Option<Folder>,
        new: Option<Folder>,
    },
    Deliverable {
        id: DeliverableId,
        old: Option<Deliverable>,
        new: Option<Deliverable>,
    },
}

fn mismatch(what: &str, id: &str) -> OpError {
    OpError {
        code: ErrorCode::OpMismatch,
        message: format!("{what} {id}: current state differs from the op's `old`"),
    }
}

fn conv(e: (ErrorCode, String)) -> OpError {
    OpError {
        code: e.0,
        message: e.1,
    }
}

impl PrimitiveOp {
    /// Operação inversa: aplicar `op` e depois `op.inverse()` restaura o estado exatamente.
    pub fn inverse(&self) -> Self {
        match self.clone() {
            Self::Clip {
                sequence,
                id,
                old,
                new,
            } => Self::Clip {
                sequence,
                id,
                old: new,
                new: old,
            },
            Self::Track {
                sequence,
                id,
                old,
                new,
            } => Self::Track {
                sequence,
                id,
                old: new,
                new: old,
            },
            Self::Marker {
                sequence,
                id,
                old,
                new,
            } => Self::Marker {
                sequence,
                id,
                old: new,
                new: old,
            },
            Self::Sequence { id, old, new } => Self::Sequence {
                id,
                old: new,
                new: old,
            },
            Self::Asset { id, old, new } => Self::Asset {
                id,
                old: new,
                new: old,
            },
            Self::Folder { id, old, new } => Self::Folder {
                id,
                old: new,
                new: old,
            },
            Self::Deliverable { id, old, new } => Self::Deliverable {
                id,
                old: new,
                new: old,
            },
        }
    }

    /// Sequence tocada (ops de asset não pertencem a uma sequence).
    pub fn sequence_id(&self) -> Option<&SequenceId> {
        match self {
            Self::Clip { sequence, .. }
            | Self::Track { sequence, .. }
            | Self::Marker { sequence, .. } => Some(sequence),
            Self::Sequence { id, .. } => Some(id),
            Self::Asset { .. } | Self::Folder { .. } | Self::Deliverable { .. } => None,
        }
    }

    /// Entidades tocadas por esta op (nível de entidade — mais conservador que entidade+campo).
    pub fn affected(&self) -> BTreeSet<EntityRef> {
        let mut set = BTreeSet::new();
        match self {
            Self::Clip { id, old, new, .. } => {
                set.insert(EntityRef::new(EntityKind::Clip, id.as_str()));
                // mover entre tracks toca as duas
                for c in old.iter().chain(new.iter()) {
                    set.insert(EntityRef::new(EntityKind::Track, c.track.as_str()));
                }
            }
            Self::Track { id, .. } => {
                set.insert(EntityRef::new(EntityKind::Track, id.as_str()));
            }
            Self::Marker { id, .. } => {
                set.insert(EntityRef::new(EntityKind::Marker, id.as_str()));
            }
            Self::Sequence { id, .. } => {
                set.insert(EntityRef::new(EntityKind::Sequence, id.as_str()));
            }
            Self::Asset { id, .. } => {
                set.insert(EntityRef::new(EntityKind::Asset, id.as_str()));
            }
            Self::Folder { id, .. } => {
                set.insert(EntityRef::new(EntityKind::Folder, id.as_str()));
            }
            Self::Deliverable { id, .. } => {
                set.insert(EntityRef::new(EntityKind::Deliverable, id.as_str()));
            }
        }
        set
    }
}

impl Document {
    /// Aplica uma op. Verifica que o estado atual bate com `old`; em erro o documento **não** é
    /// alterado por esta op (a transação descarta a working copy inteira).
    pub fn apply_op(&mut self, op: &PrimitiveOp) -> Result<(), OpError> {
        match op {
            PrimitiveOp::Sequence { id, old, new } => match (old, new) {
                (None, Some(h)) => self.insert_sequence_raw(id.clone(), h.clone()),
                (Some(old_h), None) => {
                    let removed = self.remove_sequence_raw(id)?;
                    if &removed != old_h {
                        return Err(mismatch("sequence", id.as_str()));
                    }
                    Ok(())
                }
                (Some(old_h), Some(new_h)) => {
                    let seq = self.sequence_mut(id).ok_or_else(|| {
                        OpError::new(ErrorCode::OpMismatch, format!("sequence {id} missing"))
                    })?;
                    if &seq.header != old_h {
                        return Err(mismatch("sequence", id.as_str()));
                    }
                    seq.header = new_h.clone();
                    Ok(())
                }
                (None, None) => Ok(()),
            },
            PrimitiveOp::Asset { id, old, new } => {
                if self.assets.get(id) != old.as_ref() {
                    return Err(mismatch("asset", id.as_str()));
                }
                self.put_asset_raw(id, new.clone());
                Ok(())
            }
            PrimitiveOp::Folder { id, old, new } => {
                if self.folders.get(id) != old.as_ref() {
                    return Err(mismatch("folder", id.as_str()));
                }
                self.put_folder_raw(id, new.clone());
                Ok(())
            }
            PrimitiveOp::Deliverable { id, old, new } => {
                if self.deliverables.get(id) != old.as_ref() {
                    return Err(mismatch("deliverable", id.as_str()));
                }
                self.put_deliverable_raw(id, new.clone());
                Ok(())
            }
            PrimitiveOp::Clip {
                sequence,
                id,
                old,
                new,
            } => {
                if old.is_none() && new.is_some() && self.find_clip(id).is_some() {
                    return Err(OpError::new(
                        ErrorCode::OpMismatch,
                        format!("clip id {id} already used in the project"),
                    ));
                }
                let seq = self.sequence_mut(sequence).ok_or_else(|| {
                    OpError::new(
                        ErrorCode::NotFound,
                        format!("sequence {sequence} does not exist"),
                    )
                })?;
                if seq.clip(id) != old.as_ref() {
                    return Err(mismatch("clip", id.as_str()));
                }
                match (old, new) {
                    (None, Some(n)) => seq.raw_insert_clip(n.clone()).map_err(conv),
                    (Some(_), None) => seq.raw_remove_clip(id).map(|_| ()).map_err(conv),
                    (Some(_), Some(n)) => seq.raw_replace_clip(n.clone()).map(|_| ()).map_err(conv),
                    (None, None) => Ok(()),
                }
            }
            PrimitiveOp::Track {
                sequence,
                id,
                old,
                new,
            } => {
                let seq = self.sequence_mut(sequence).ok_or_else(|| {
                    OpError::new(
                        ErrorCode::NotFound,
                        format!("sequence {sequence} does not exist"),
                    )
                })?;
                let current = seq.track_position(id).and_then(|i| {
                    seq.tracks().get(i).map(|t| TrackSlot {
                        index: i,
                        track: t.clone(),
                    })
                });
                if current.as_ref() != old.as_ref() {
                    return Err(mismatch("track", id.as_str()));
                }
                match (old, new) {
                    (None, Some(slot)) => seq
                        .raw_insert_track(slot.index, slot.track.clone())
                        .map_err(conv),
                    (Some(_), None) => seq.raw_remove_track(id).map(|_| ()).map_err(conv),
                    (Some(o), Some(n)) if o.index == n.index => seq
                        .raw_replace_track(id, n.track.clone())
                        .map(|_| ())
                        .map_err(conv),
                    (Some(_), Some(n)) => {
                        // reordenar: a track não pode ter clips removidos — mover preserva o índice
                        // por track; reimplementamos como replace + reposicionar no vetor.
                        seq.raw_reposition_track(id, n.index, n.track.clone())
                            .map_err(conv)
                    }
                    (None, None) => Ok(()),
                }
            }
            PrimitiveOp::Marker {
                sequence,
                id,
                old,
                new,
            } => {
                let seq = self.sequence_mut(sequence).ok_or_else(|| {
                    OpError::new(
                        ErrorCode::NotFound,
                        format!("sequence {sequence} does not exist"),
                    )
                })?;
                if seq.marker(id) != old.as_ref() {
                    return Err(mismatch("marker", id.as_str()));
                }
                seq.raw_put_marker(new.clone(), id);
                Ok(())
            }
        }
    }

    /// Aplica várias ops em ordem. Em erro, o documento fica parcialmente alterado — chame sobre
    /// uma working copy (é o que o Command Engine faz).
    pub fn apply_ops(&mut self, ops: &[PrimitiveOp]) -> Result<(), OpError> {
        ops.iter().try_for_each(|op| self.apply_op(op))
    }
}
