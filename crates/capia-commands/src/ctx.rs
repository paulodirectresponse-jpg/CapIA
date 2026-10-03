//! Contexto de execução de uma transação: working copy do documento + ops primitivas emitidas.

use crate::error::{CommandError, Result};
use crate::hash::sha256_hex;
use capia_model::{
    Clip, ClipId, Document, EntityRef, PrimitiveOp, Sequence, SequenceId, Track, TrackId,
};
use std::collections::BTreeSet;

/// Resultado de um comando.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandOutput {
    /// Id principal criado (alvo da `ref` simbólica), se houver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<EntityRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub(crate) struct Ctx {
    pub doc: Document,
    pub ops: Vec<PrimitiveOp>,
    max_ops: usize,
    operation_id: String,
    slot: u32,
    pub warnings: Vec<String>,
    pub created: Vec<EntityRef>,
}

impl Ctx {
    pub(crate) fn new(doc: Document, max_ops: usize) -> Self {
        Self {
            doc,
            ops: Vec::new(),
            max_ops,
            operation_id: String::new(),
            slot: 0,
            warnings: Vec::new(),
            created: Vec::new(),
        }
    }

    /// Prepara o contexto para o próximo comando.
    pub(crate) fn begin_command(&mut self, operation_id: &str) {
        self.operation_id = operation_id.to_owned();
        self.slot = 0;
        self.warnings.clear();
        self.created.clear();
    }

    /// Aplica a op na working copy e a registra. Falha de aplicação indica estado inconsistente
    /// com o que o comando calculou (bug) — vira erro, a transação é descartada.
    pub(crate) fn emit(&mut self, op: PrimitiveOp) -> Result<()> {
        if self.ops.len() >= self.max_ops {
            return Err(CommandError::new(
                capia_model::ErrorCode::LimitExceeded,
                format!("transaction exceeds {} primitive ops", self.max_ops),
            ));
        }
        self.doc.apply_op(&op)?;
        self.ops.push(op);
        Ok(())
    }

    /// Id derivado deterministicamente de `operation_id` + posição: replays e re-previews geram os
    /// mesmos ids (docs/COMMAND_SYSTEM.md §4.1).
    pub(crate) fn derive_id(&mut self, kind: &str) -> String {
        let slot = self.slot;
        self.slot += 1;
        let digest = sha256_hex(format!("{}|{kind}|{slot}", self.operation_id).as_bytes());
        format!("{kind}_{}", &digest[..16])
    }

    pub(crate) fn note_created(&mut self, kind: capia_model::EntityKind, id: &str) {
        self.created.push(EntityRef::new(kind, id));
    }

    pub(crate) fn take_output(&mut self, id: Option<String>) -> CommandOutput {
        CommandOutput {
            id,
            created: core::mem::take(&mut self.created),
            warnings: core::mem::take(&mut self.warnings),
        }
    }

    // ---- consultas (clonam: o contexto precisa continuar mutável) -----------------------------

    pub(crate) fn sequence(&self, id: &SequenceId) -> Result<&Sequence> {
        self.doc
            .sequence(id)
            .ok_or_else(|| CommandError::not_found("sequence", id))
    }

    /// Clip + sequence + track que o contém.
    pub(crate) fn locate_clip(&self, id: &ClipId) -> Result<(SequenceId, Clip, Track)> {
        let (sid, seq, clip) = self
            .doc
            .find_clip(id)
            .ok_or_else(|| CommandError::not_found("clip", id))?;
        let track = seq
            .track(&clip.track)
            .ok_or_else(|| CommandError::not_found("track", &clip.track))?
            .clone();
        Ok((sid.clone(), clip.clone(), track))
    }

    pub(crate) fn locate_track(&self, id: &TrackId) -> Result<(SequenceId, Track)> {
        let (sid, _, track) = self
            .doc
            .find_track(id)
            .ok_or_else(|| CommandError::not_found("track", id))?;
        Ok((sid.clone(), track.clone()))
    }

    // ---- escrita de clips ---------------------------------------------------------------------

    pub(crate) fn insert_clip_op(&mut self, seq: &SequenceId, clip: Clip) -> Result<()> {
        self.emit(PrimitiveOp::Clip {
            sequence: seq.clone(),
            id: clip.id.clone(),
            old: None,
            new: Some(clip),
        })
    }

    pub(crate) fn remove_clip_op(&mut self, seq: &SequenceId, old: Clip) -> Result<()> {
        self.emit(PrimitiveOp::Clip {
            sequence: seq.clone(),
            id: old.id.clone(),
            old: Some(old),
            new: None,
        })
    }

    pub(crate) fn replace_clip_op(&mut self, seq: &SequenceId, old: Clip, new: Clip) -> Result<()> {
        if old == new {
            return Ok(());
        }
        self.emit(PrimitiveOp::Clip {
            sequence: seq.clone(),
            id: old.id.clone(),
            old: Some(old),
            new: Some(new),
        })
    }
}

/// Sequences tocadas pelas ops (escopo da validação de invariantes).
pub(crate) fn touched_sequences(ops: &[PrimitiveOp]) -> BTreeSet<SequenceId> {
    ops.iter()
        .filter_map(|o| o.sequence_id().cloned())
        .collect()
}

/// Alguma op mexe no grafo de nested (clip `Nested` ou sequence criada/removida)?
pub(crate) fn touches_nested_graph(ops: &[PrimitiveOp]) -> bool {
    use capia_model::ClipContent;
    ops.iter().any(|op| match op {
        PrimitiveOp::Sequence { .. } => true,
        PrimitiveOp::Clip { old, new, .. } => old
            .iter()
            .chain(new.iter())
            .any(|c| matches!(c.content, ClipContent::Nested { .. })),
        _ => false,
    })
}
