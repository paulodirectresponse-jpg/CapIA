//! Limites contra metadados absurdos e saídas gigantes (ADR-047 §3/§5).

use std::time::Duration;

/// Teto do stdout do ffprobe (JSON).
pub const MAX_PROBE_STDOUT: usize = 8 * 1024 * 1024;
/// Teto do stderr capturado.
pub const MAX_PROBE_STDERR: usize = 64 * 1024;
/// Timeout padrão do probe.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Tamanho máximo (bytes) de um caminho aceito.
pub const MAX_PATH_BYTES: usize = 4096;
/// Duração máxima plausível de uma mídia (1.000 h).
pub const MAX_MEDIA_SECONDS: i64 = 1_000 * 3600;
/// Dimensão máxima de imagem/vídeo.
pub const MAX_DIMENSION: u32 = 65_536;
/// Frame rate máximo plausível.
pub const MAX_FPS: i64 = 1_000;
/// Canais de áudio máximos.
pub const MAX_CHANNELS: u32 = 64;
/// Sample rate máximo (Hz).
pub const MAX_SAMPLE_RATE: u32 = 768_000;
/// Nº máximo de streams aceitos num container.
pub const MAX_STREAMS: usize = 256;
/// Teto do PNG de uma miniatura.
pub const MAX_THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;
