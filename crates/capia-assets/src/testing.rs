//! Apoio a testes (sem ffprobe). Não use em produção.

use capia_media::{
    AudioStream, ContainerInfo, MediaError, MediaInfo, MediaKind, MediaProbe, StreamInfo,
    VideoStream,
};
use capia_time::{Rational, TICKS_PER_SECOND, Ticks};
use std::path::Path;

pub fn synthetic_info(kind: MediaKind, seconds: i64) -> MediaInfo {
    let d = Ticks(seconds * TICKS_PER_SECOND);
    let video = StreamInfo::Video(VideoStream {
        index: 0,
        codec: "h264".into(),
        width: 64,
        height: 48,
        pixel_aspect: Some(Rational::ONE),
        display_aspect: Rational::new(4, 3).ok(),
        frame_rate: Some(Rational::from_int(25)),
        nominal_frame_rate: Some(Rational::from_int(25)),
        time_base: Rational::new(1, 12800).ok(),
        pixel_format: Some("yuv420p".into()),
        rotation: 0,
        bit_rate: None,
        duration: Some(d),
        frames: None,
        has_alpha: Some(false),
        attached_picture: false,
    });
    let audio = StreamInfo::Audio(AudioStream {
        index: 1,
        codec: "aac".into(),
        channels: 2,
        sample_rate: 48000,
        bit_rate: None,
        duration: Some(d),
        time_base: Rational::new(1, 48000).ok(),
    });
    let (streams, dv, da, dur) = match kind {
        MediaKind::Video => (vec![video, audio], Some(0), Some(1), Some(d)),
        MediaKind::Audio => (vec![audio], None, Some(1), Some(d)),
        MediaKind::Image => (vec![video], Some(0), None, None),
    };
    MediaInfo {
        kind,
        container: ContainerInfo {
            formats: vec!["synthetic".into()],
            duration: dur,
            bit_rate: None,
            stream_count: u32::try_from(streams.len()).unwrap_or(0),
        },
        streams,
        default_video: dv,
        default_audio: da,
        duration: dur,
        warnings: Vec::new(),
    }
}

/// Probe falso: classifica por extensão (`.mp4`→vídeo, `.wav`→áudio, `.png`→imagem) e rejeita
/// `.bad` como arquivo inválido. Determinístico, sem processo externo.
#[derive(Clone, Debug, Default)]
pub struct StaticProbe;

impl MediaProbe for StaticProbe {
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("mp4") => Ok(synthetic_info(MediaKind::Video, 10)),
            Some("wav") => Ok(synthetic_info(MediaKind::Audio, 5)),
            Some("png") => Ok(synthetic_info(MediaKind::Image, 0)),
            _ => Err(MediaError::new(
                capia_media::MediaErrorCode::MediaProbeFailed,
                "synthetic probe: not media",
            )),
        }
    }
}
