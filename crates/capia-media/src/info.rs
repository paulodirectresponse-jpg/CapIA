//! Modelo **normalizado** de mídia (ADR-047 §1/§5). Sem caminhos, sem timestamps voláteis, sem maps:
//! o mesmo arquivo gera sempre o mesmo valor.

use capia_time::{Rational, Ticks};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoStream {
    pub index: u32,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// Sample (pixel) aspect ratio; `None` se ausente/inválido.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_aspect: Option<Rational>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_aspect: Option<Rational>,
    /// Frame rate médio (cai para o nominal quando o médio é inválido).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<Rational>,
    /// Frame rate nominal (`r_frame_rate`); difere de `frame_rate` em VFR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal_frame_rate: Option<Rational>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_base: Option<Rational>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_format: Option<String>,
    /// Rotação de exibição em graus horários: 0, 90, 180 ou 270.
    #[serde(default)]
    pub rotation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Ticks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
    /// `Some(true/false)` só quando o `pix_fmt` permite afirmar (ADR-047 §6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_alpha: Option<bool>,
    /// Capa/miniatura embutida (não é vídeo de verdade).
    #[serde(default)]
    pub attached_picture: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Ticks>,
    /// Time base dos pacotes do stream (unidade do `pts` do índice de áudio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_base: Option<Rational>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtherStream {
    pub index: u32,
    pub codec_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamInfo {
    Video(VideoStream),
    Audio(AudioStream),
    Other(OtherStream),
}

impl StreamInfo {
    pub fn index(&self) -> u32 {
        match self {
            Self::Video(v) => v.index,
            Self::Audio(a) => a.index,
            Self::Other(o) => o.index,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerInfo {
    /// Nomes de formato do ffprobe, **ordenados** (ex.: `["3g2","3gp","m4a","mj2","mov","mp4"]`).
    pub formats: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Ticks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<u64>,
    pub stream_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub kind: MediaKind,
    pub container: ContainerInfo,
    /// Ordenados por `index`.
    pub streams: Vec<StreamInfo>,
    /// Índice do stream de vídeo padrão (primeiro que não é capa embutida).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_video: Option<u32>,
    /// Índice do stream de áudio padrão (primeiro de áudio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_audio: Option<u32>,
    /// Duração do arquivo: container, ou o maior stream padrão. `None` para imagem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Ticks>,
    /// Campos inválidos descartados (determinísticos).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl MediaInfo {
    pub fn video(&self) -> Option<&VideoStream> {
        self.default_video.and_then(|i| {
            self.streams.iter().find_map(|s| match s {
                StreamInfo::Video(v) if v.index == i => Some(v),
                _ => None,
            })
        })
    }

    pub fn audio(&self) -> Option<&AudioStream> {
        self.default_audio.and_then(|i| {
            self.streams.iter().find_map(|s| match s {
                StreamInfo::Audio(a) if a.index == i => Some(a),
                _ => None,
            })
        })
    }

    pub fn has_video(&self) -> bool {
        self.default_video.is_some() && self.kind != MediaKind::Audio
    }

    pub fn has_audio(&self) -> bool {
        self.default_audio.is_some()
    }
}
