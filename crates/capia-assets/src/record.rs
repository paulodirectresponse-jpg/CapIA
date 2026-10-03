//! Registro de asset no catálogo do projeto (ADR-046/048).

use crate::hash::ContentHash;
use crate::location::AssetLocation;
use capia_media::{MediaInfo, MediaKind};
use capia_model::AssetId;
use serde::{Deserialize, Serialize};

/// Tipos iniciais. Outros (música, SFX, fonte, LUT, documento…) entram quando houver pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Video,
    Audio,
    Image,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

impl From<MediaKind> for AssetKind {
    fn from(k: MediaKind) -> Self {
        match k {
            MediaKind::Video => Self::Video,
            MediaKind::Audio => Self::Audio,
            MediaKind::Image => Self::Image,
        }
    }
}

/// Disponibilidade **derivada** do disco (nunca editada à mão; ADR-048 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// O arquivo existe e tem o tamanho conhecido (ou o hash foi reverificado).
    Online,
    /// Nenhum caminho candidato é acessível.
    Offline,
    /// Acessível, mas o conteúdo difere do que foi importado.
    Modified,
}

impl Availability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Modified => "modified",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "online" => Some(Self::Online),
            "offline" => Some(Self::Offline),
            "modified" => Some(Self::Modified),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetRecord {
    pub asset_id: AssetId,
    pub kind: AssetKind,
    pub content_hash: ContentHash,
    pub size_bytes: u64,
    pub display_name: String,
    pub location: AssetLocation,
    /// Outros caminhos onde o mesmo conteúdo já foi visto (aliases de import), em texto.
    #[serde(default)]
    pub known_paths: Vec<String>,
    pub media: MediaInfo,
    pub status: Availability,
    /// Instante da última checagem de `status` (volátil: nunca entra no documento).
    pub status_checked_ms: u64,
    pub imported_ms: u64,
}
