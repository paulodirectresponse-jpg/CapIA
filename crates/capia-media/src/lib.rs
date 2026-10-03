//! Mídia externa como **entrada hostil** (ADR-047): probe estruturado, normalização determinística
//! e execução limitada de processos. O resto do CapIA só enxerga [`MediaInfo`]; nunca a saída
//! textual/JSON do ffprobe. Este crate faz IO (processos) e **não** compila para WASM.

mod decode;
mod error;
mod failpoints;
mod ffprobe;
mod index;
mod info;
mod limits;
mod normalize;
mod process;
mod thumbnail;
mod toolchain;

pub use decode::{
    AudioPcm, AudioRequest, DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_PCM_BYTES, DecodeLimits,
    PixelFormat, RawFrame, decode_audio, decode_audio_blocks, decode_frame_at,
    decode_frame_by_index, samples_to_ticks, ticks_to_samples,
};
pub use error::{MediaError, MediaErrorCode};
pub use ffprobe::{FfprobeBackend, MediaProbe};
pub use index::{
    FrameEntry, FrameIndex, INDEX_MAGIC, INDEX_PRODUCER, INDEX_VERSION, IndexOptions,
    MAX_INDEX_FRAMES, build_frame_index,
};
pub use info::{
    AudioStream, ContainerInfo, MediaInfo, MediaKind, OtherStream, StreamInfo, VideoStream,
};
pub use limits::*;
pub use normalize::{normalize_ffprobe_json, parse_decimal_ticks, parse_frame_rate};
pub use process::{
    Flow, RunLimits, RunOutput, StreamLimits, StreamOutput, run_bounded, run_collect, run_streaming,
};
pub use thumbnail::{ThumbnailRequest, extract_frame_png};
pub use toolchain::{MediaConfig, MediaToolchain, ToolSource};
