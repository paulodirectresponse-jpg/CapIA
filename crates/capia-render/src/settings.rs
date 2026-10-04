use crate::error::RenderError;
use crate::image::MAX_DIMENSION;
use capia_time::FrameRate;

/// Parâmetros de saída. Largura/altura/cadência/áudio são do **entregável**, não da sequence (o
/// modelo não guarda resolução).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    /// Cadência dos quadros de saída (`render_video_range`); `None` ⇒ a da sequence.
    pub frame_rate: Option<FrameRate>,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
    /// `true` (padrão, **export**): fonte indisponível/falha de decode é erro. `false` (**preview**):
    /// vira aviso `SOURCE_UNAVAILABLE` e o layer é pulado.
    pub strict_sources: bool,
}

impl RenderSettings {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            frame_rate: None,
            audio_sample_rate: 48_000,
            audio_channels: 2,
            strict_sources: true,
        }
    }

    pub fn validate(&self) -> Result<(), RenderError> {
        let bad = |m: &str| Err(RenderError::new("RENDER_SETTINGS_INVALID", m));
        if self.width == 0
            || self.height == 0
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
        {
            return bad("width/height must be within 1..=16384");
        }
        if !(8_000..=192_000).contains(&self.audio_sample_rate) {
            return bad("audio sample rate must be within 8000..=192000");
        }
        if !(1..=8).contains(&self.audio_channels) {
            return bad("audio channels must be within 1..=8");
        }
        Ok(())
    }
}
