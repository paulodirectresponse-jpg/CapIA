//! Primeira mídia derivada: um quadro PNG (ADR-049 §4).

use crate::error::{MediaError, MediaErrorCode};
use crate::ffprobe::{checked_input_path, file_url_arg};
use crate::limits::{MAX_PROBE_STDERR, MAX_THUMBNAIL_BYTES};
use crate::process::{RunLimits, run_bounded};
use crate::toolchain::MediaToolchain;
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbnailRequest {
    /// Instante do quadro, em ticks (≥ 0).
    pub at: Ticks,
    /// Maior lado da miniatura (16..=1024).
    pub max_dim: u32,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// `Ticks` → `"S.uuuuuu"` por aritmética inteira (microssegundos, truncado).
fn seconds_arg(t: Ticks) -> String {
    let micros = i128::from(t.0.max(0)) * 1_000_000 / i128::from(TICKS_PER_SECOND);
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

pub fn extract_frame_png(
    toolchain: &MediaToolchain,
    path: &Path,
    req: ThumbnailRequest,
) -> Result<Vec<u8>, MediaError> {
    let Some(ffmpeg) = &toolchain.ffmpeg else {
        return Err(MediaError::new(
            MediaErrorCode::MediaBackendNotFound,
            "ffmpeg was not found (needed to generate thumbnails)",
        ));
    };
    if req.at.0 < 0 || !(16..=1024).contains(&req.max_dim) {
        return Err(MediaError::new(
            MediaErrorCode::MediaMetadataInvalid,
            "invalid thumbnail request (time must be ≥ 0 and size within 16..=1024)",
        ));
    }
    let abs = checked_input_path(path)?;
    let d = req.max_dim;
    let filter = format!("scale='min({d},iw)':'min({d},ih)':force_original_aspect_ratio=decrease");
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-ss",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(OsString::from(seconds_arg(req.at)));
    args.push(OsString::from("-i"));
    args.push(file_url_arg(&abs));
    for a in [
        "-frames:v",
        "1",
        "-an",
        "-sn",
        "-vf",
        &filter,
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "pipe:1",
    ] {
        args.push(OsString::from(a));
    }
    let out = run_bounded(
        ffmpeg,
        &args,
        &RunLimits {
            timeout: Duration::from_secs(60),
            max_stdout: MAX_THUMBNAIL_BYTES,
            max_stderr: MAX_PROBE_STDERR,
        },
    )?;
    if !out.status.success() || !out.stdout.starts_with(&PNG_SIGNATURE) {
        return Err(MediaError::new(
            MediaErrorCode::MediaProbeFailed,
            "ffmpeg could not produce a frame at that time",
        ));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_are_formatted_with_integer_math() {
        assert_eq!(seconds_arg(Ticks(0)), "0.000000");
        assert_eq!(seconds_arg(Ticks(TICKS_PER_SECOND * 3 / 2)), "1.500000");
        assert_eq!(seconds_arg(Ticks(-5)), "0.000000");
        assert_eq!(seconds_arg(Ticks(i64::MAX)).split('.').count(), 2);
    }
}
