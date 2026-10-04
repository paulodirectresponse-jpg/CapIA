//! Serviço de decode persistente (ADR-059..061): sessões de ffmpeg reaproveitadas, cache de quadros
//! por bytes, prefetch e supersession de scrub. Só decodifica; não conhece o projeto/documento.

mod audio;
mod cache;
mod service;

pub use audio::{
    AudioSource, PCM_BACKEND_VERSION, PCM_BLOCK_FRAMES, PCM_READ_AHEAD_BLOCKS, PcmCache, PcmMetrics,
};
pub use cache::{ByteLru, CacheStats};
pub use service::{
    DECODE_BACKEND_VERSION, DecodeConfig, DecodeError, DecodeMetrics, DecodeService, Direction,
    FrameKey, FrameTicket, Lane, Priority, VideoSource,
};
