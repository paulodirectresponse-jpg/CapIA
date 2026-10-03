//! Mídia externa como **entrada hostil** (ADR-047): probe estruturado, normalização determinística
//! e execução limitada de processos. O resto do CapIA só enxerga [`MediaInfo`]; nunca a saída
//! textual/JSON do ffprobe. Este crate faz IO (processos) e **não** compila para WASM.

mod error;
mod ffprobe;
mod info;
mod limits;
mod normalize;
mod process;
mod thumbnail;
mod toolchain;

pub use error::{MediaError, MediaErrorCode};
pub use ffprobe::{FfprobeBackend, MediaProbe};
pub use info::{
    AudioStream, ContainerInfo, MediaInfo, MediaKind, OtherStream, StreamInfo, VideoStream,
};
pub use limits::*;
pub use normalize::{normalize_ffprobe_json, parse_decimal_ticks, parse_frame_rate};
pub use process::{RunLimits, RunOutput, run_bounded};
pub use thumbnail::{ThumbnailRequest, extract_frame_png};
pub use toolchain::{MediaConfig, MediaToolchain, ToolSource};
