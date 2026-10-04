//! Render **headless e determinístico** (ADR-064..067): `Document/Sequence → RenderGraph →
//! plano por instante → decode (via trait) → compositor CPU / mixer`. Sem IO, sem FFmpeg, sem UI:
//! a mídia decodificada entra por [`MediaSource`] e o mesmo caminho serve preview e export
//! (PREVIEW_RENDER §4). O compositor CPU é a **referência semântica** do futuro compositor GPU.
//!
//! Semântica fixada (ADR-065/066): RGBA8 com alpha **reto** (não pré-multiplicado); `source-over`
//! em inteiros com arredondamento half-up; amostragem bilinear em ponto fixo 8 bits; tela do
//! quadro final começa em preto opaco; áudio f32 com *hard clip* em [-1, 1] no fim.

mod audio;
mod error;
mod graph;
mod image;
mod mixer;
mod render;
mod settings;
mod source;

pub use audio::{AudioBuffer, resample_linear_position, resample_sinc};
pub use error::{RenderError, RenderWarning};
pub use graph::{
    GraphClip, GraphClipKind, GraphSequence, GraphTrack, LayerKind, LayerPlan, MAX_NEST_DEPTH,
    RenderGraph, Transform,
};
pub use image::{Image, MAX_DIMENSION, blend_over, blit_layer, parse_color};
pub use mixer::mix_audio_range;
pub use render::{RenderedFrame, frame_digest, frame_time, render_frame, render_video_range};
pub use settings::RenderSettings;
pub use source::{AudioRequest, MediaSource, SourceError};
