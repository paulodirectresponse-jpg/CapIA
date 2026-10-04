use crate::clip::Clip;
use crate::error::ErrorCode;
use crate::ids::{AssetId, ClipId, DeliverableId, FolderId, SequenceId};
use crate::sequence::{Sequence, SequenceHeader};
use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Versão do schema do documento persistido. Migrations só para frente (DATA_MODEL.md §6).
pub const DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// Mídia referenciada por clips. `duration = None` ⇒ ilimitada (imagem, gerador). O asset pode
/// estar offline, mas deve existir (invariante 8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub duration: Option<Ticks>,
    #[serde(default)]
    pub has_video: bool,
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default)]
    pub offline: bool,
}

/// Pasta do painel Project (organização; as abas de sequences só navegam, não organizam).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
    #[serde(default)]
    pub parent: Option<FolderId>,
}

/// Entregável: referencia uma sequence, um preset de export e o destino. A execução (export) é
/// do `capia-project`; o documento só guarda a **definição**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: DeliverableId,
    pub name: String,
    pub sequence: SequenceId,
    /// Nome do preset de export (`h264-mp4`, `intermediate`, ...).
    pub preset: String,
    /// Caminho de saída (relativo à pasta do projeto ou absoluto).
    pub path: String,
    /// Largura/altura de saída; `None` ⇒ as da sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Erro de aplicação de uma operação primitiva.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpError {
    pub code: ErrorCode,
    pub message: String,
}

impl OpError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for OpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for OpError {}

/// O documento do projeto: estado imutável por **compartilhamento estrutural** — `clone()` é O(nº de
/// sequences) e uma transação só copia (copy-on-write) as sequences que realmente altera.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub schema_version: u32,
    /// Incrementa a cada commit, undo e redo.
    pub revision: u64,
    pub(crate) sequences: BTreeMap<SequenceId, Arc<Sequence>>,
    pub(crate) assets: BTreeMap<AssetId, Asset>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) folders: BTreeMap<FolderId, Folder>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) deliverables: BTreeMap<DeliverableId, Deliverable>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            schema_version: DOCUMENT_SCHEMA_VERSION,
            revision: 0,
            sequences: BTreeMap::new(),
            assets: BTreeMap::new(),
            folders: BTreeMap::new(),
            deliverables: BTreeMap::new(),
        }
    }

    pub fn folder(&self, id: &FolderId) -> Option<&Folder> {
        self.folders.get(id)
    }

    pub fn folders(&self) -> impl Iterator<Item = &Folder> {
        self.folders.values()
    }

    pub fn deliverable(&self, id: &DeliverableId) -> Option<&Deliverable> {
        self.deliverables.get(id)
    }

    pub fn deliverables(&self) -> impl Iterator<Item = &Deliverable> {
        self.deliverables.values()
    }

    pub fn sequence(&self, id: &SequenceId) -> Option<&Sequence> {
        self.sequences.get(id).map(Arc::as_ref)
    }

    pub fn sequences(&self) -> impl Iterator<Item = (&SequenceId, &Sequence)> {
        self.sequences.iter().map(|(k, v)| (k, v.as_ref()))
    }

    pub fn sequence_count(&self) -> usize {
        self.sequences.len()
    }

    pub fn asset(&self, id: &AssetId) -> Option<&Asset> {
        self.assets.get(id)
    }

    pub fn assets(&self) -> impl Iterator<Item = &Asset> {
        self.assets.values()
    }

    /// Localiza um clip em qualquer sequence (ids são únicos no projeto).
    pub fn find_clip(&self, id: &ClipId) -> Option<(&SequenceId, &Sequence, &Clip)> {
        self.sequences
            .iter()
            .find_map(|(sid, s)| s.clip(id).map(|c| (sid, s.as_ref(), c)))
    }

    /// Localiza uma track em qualquer sequence (ids são únicos no projeto).
    pub fn find_track(
        &self,
        id: &crate::ids::TrackId,
    ) -> Option<(&SequenceId, &Sequence, &crate::sequence::Track)> {
        self.sequences
            .iter()
            .find_map(|(sid, s)| s.track(id).map(|t| (sid, s.as_ref(), t)))
    }

    pub(crate) fn sequence_mut(&mut self, id: &SequenceId) -> Option<&mut Sequence> {
        self.sequences.get_mut(id).map(Arc::make_mut)
    }

    pub(crate) fn insert_sequence_raw(
        &mut self,
        id: SequenceId,
        header: SequenceHeader,
    ) -> Result<(), OpError> {
        if self.sequences.contains_key(&id) {
            return Err(OpError::new(
                ErrorCode::OpMismatch,
                format!("sequence {id} already exists"),
            ));
        }
        self.sequences.insert(id, Arc::new(Sequence::new(header)));
        Ok(())
    }

    pub(crate) fn remove_sequence_raw(
        &mut self,
        id: &SequenceId,
    ) -> Result<SequenceHeader, OpError> {
        match self.sequences.get(id) {
            None => Err(OpError::new(
                ErrorCode::OpMismatch,
                format!("sequence {id} does not exist"),
            )),
            Some(s) if !s.is_empty() => Err(OpError::new(
                ErrorCode::OpMismatch,
                format!("sequence {id} is not empty"),
            )),
            Some(s) => {
                let header = s.header.clone();
                self.sequences.remove(id);
                Ok(header)
            }
        }
    }

    pub(crate) fn put_folder_raw(&mut self, id: &FolderId, f: Option<Folder>) {
        match f {
            Some(f) => self.folders.insert(id.clone(), f),
            None => self.folders.remove(id),
        };
    }

    pub(crate) fn put_deliverable_raw(&mut self, id: &DeliverableId, d: Option<Deliverable>) {
        match d {
            Some(d) => self.deliverables.insert(id.clone(), d),
            None => self.deliverables.remove(id),
        };
    }

    pub(crate) fn put_asset_raw(&mut self, id: &AssetId, asset: Option<Asset>) -> Option<Asset> {
        match asset {
            Some(a) => self.assets.insert(id.clone(), a),
            None => self.assets.remove(id),
        }
    }
}
