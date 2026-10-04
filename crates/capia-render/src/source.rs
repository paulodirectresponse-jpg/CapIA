use crate::audio::AudioBuffer;
use crate::image::Image;
use capia_model::AssetId;
use capia_time::Ticks;
use std::sync::Arc;

/// Falha de uma fonte de mídia (arquivo offline, decode que falhou…).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceError {
    pub code: String,
    pub message: String,
}

impl SourceError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl core::fmt::Display for SourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for SourceError {}

/// Pedido de áudio decodificado **na taxa e canais de saída**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioRequest {
    /// Primeira amostra (por canal), na taxa `sample_rate`, contada do início da mídia.
    pub start_sample: u64,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u32,
}

/// Mídia decodificada. Implementada pela camada de projeto (decode service + caches); nos testes,
/// por fontes sintéticas. **Nunca** devolve proxy: a fonte é sempre o original.
pub trait MediaSource: Send + Sync {
    /// Quadro mostrado no tempo de fonte `source_t` (relativo ao 1º quadro): o último com tempo ≤
    /// `source_t`. `Ok(None)` se `source_t` é anterior ao 1º quadro.
    fn video_frame(
        &self,
        asset: &AssetId,
        source_t: Ticks,
    ) -> Result<Option<Arc<Image>>, SourceError>;

    /// Imagem estática (PNG/JPEG…).
    fn still_image(&self, asset: &AssetId) -> Result<Arc<Image>, SourceError>;

    /// PCM f32 intercalado; o que passar do fim da mídia vem como silêncio (nunca erro).
    fn audio(&self, asset: &AssetId, req: AudioRequest) -> Result<AudioBuffer, SourceError>;

    /// Dica: o chamador não precisa mais do que está em andamento (scrub rápido). Fontes com decode
    /// assíncrono abortam os pedidos pendentes; o padrão não faz nada.
    fn cancel_pending(&self) {}
}
