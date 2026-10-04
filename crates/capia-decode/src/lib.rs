//! Serviço de decode persistente (ADR-059..061): sessões de ffmpeg reaproveitadas, cache de quadros
//! por bytes, prefetch e supersession de scrub. Só decodifica; não conhece o projeto/documento.

mod cache;
mod service;

pub use cache::{ByteLru, CacheStats};
pub use service::{
    DECODE_BACKEND_VERSION, DecodeConfig, DecodeError, DecodeMetrics, DecodeService, Direction,
    FrameKey, FrameTicket, Lane, Priority, VideoSource,
};
