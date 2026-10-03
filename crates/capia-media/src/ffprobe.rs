//! `MediaProbe` sobre o `ffprobe` real (ADR-047 §2).

use crate::error::{MediaError, MediaErrorCode};
use crate::info::MediaInfo;
use crate::limits::{MAX_PATH_BYTES, MAX_PROBE_STDERR, MAX_PROBE_STDOUT};
use crate::normalize::normalize_ffprobe_json;
use crate::process::{RunLimits, run_bounded};
use crate::toolchain::MediaToolchain;
use std::ffi::OsString;
use std::path::Path;

/// Contrato de probe: o resto do CapIA depende **só** disto (e de [`MediaInfo`]).
pub trait MediaProbe: Send + Sync {
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError>;
}

#[derive(Clone, Debug)]
pub struct FfprobeBackend {
    toolchain: MediaToolchain,
}

/// Valida o caminho **antes** de entregá-lo a um processo externo e devolve a forma absoluta.
pub(crate) fn checked_input_path(path: &Path) -> Result<std::path::PathBuf, MediaError> {
    let bad = |m: &str| MediaError::new(MediaErrorCode::MediaInvalidPath, m.to_owned());
    let os = path.as_os_str();
    if os.is_empty() {
        return Err(bad("empty path"));
    }
    if os.len() > MAX_PATH_BYTES {
        return Err(bad("path is too long"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if os.as_bytes().contains(&0) {
            return Err(bad("path contains a NUL byte"));
        }
    }
    let abs = std::path::absolute(path).map_err(|e| bad(&format!("cannot resolve path: {e}")))?;
    let meta = std::fs::metadata(&abs).map_err(|e| {
        MediaError::new(
            MediaErrorCode::MediaIo,
            format!("cannot read `{}`: {e}", abs.display()),
        )
    })?;
    if !meta.is_file() {
        return Err(bad("not a regular file"));
    }
    Ok(abs)
}

/// `file:` + caminho: impede que o ffmpeg interprete `concat:`/`http:`/… no começo do nome.
pub(crate) fn file_url_arg(abs: &Path) -> OsString {
    let mut s = OsString::from("file:");
    s.push(abs.as_os_str());
    s
}

/// Argumentos do ffprobe (estruturados; nunca passam por shell). O caminho é UM argumento, depois de
/// `-i` e prefixado com `file:`; o protocolo fica restrito a `file`.
pub(crate) fn probe_args(abs: &Path) -> Vec<OsString> {
    [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-protocol_whitelist",
        "file",
        "-probesize",
        "5000000",
        "-analyzeduration",
        "5000000",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .chain(std::iter::once(file_url_arg(abs)))
    .collect()
}

impl FfprobeBackend {
    pub fn new(toolchain: MediaToolchain) -> Self {
        Self { toolchain }
    }

    pub fn toolchain(&self) -> &MediaToolchain {
        &self.toolchain
    }
}

impl MediaProbe for FfprobeBackend {
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        let abs = checked_input_path(path)?;
        let args = probe_args(&abs);
        let out = run_bounded(
            &self.toolchain.ffprobe,
            &args,
            &RunLimits {
                timeout: self.toolchain.probe_timeout,
                max_stdout: MAX_PROBE_STDOUT,
                max_stderr: MAX_PROBE_STDERR,
            },
        )?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr);
            let msg: String = msg
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("ffprobe failed")
                .chars()
                .take(300)
                .collect();
            // não vaza o caminho do usuário além do que ele mesmo passou
            return Err(MediaError::new(
                MediaErrorCode::MediaProbeFailed,
                format!("ffprobe could not read the file: {msg}"),
            ));
        }
        normalize_ffprobe_json(&out.stdout)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_input_is_one_file_url_argument_after_dash_i_and_protocols_are_restricted() {
        let evil = Path::new("/tmp/concat:a.mp4|b.mp4; rm -rf $HOME `id` --help");
        let args = probe_args(evil);
        let n = args.len();
        assert_eq!(args[n - 2], OsString::from("-i"));
        let last = args[n - 1].to_string_lossy().into_owned();
        assert!(last.starts_with("file:"), "{last}");
        assert!(
            last.ends_with("--help"),
            "the path is passed whole, never split"
        );
        let wl = args
            .iter()
            .position(|a| a == "-protocol_whitelist")
            .unwrap();
        assert_eq!(args[wl + 1], OsString::from("file"));
        // nenhum outro argumento carrega o caminho do usuário
        assert_eq!(
            args.iter()
                .filter(|a| a.to_string_lossy().contains("concat:"))
                .count(),
            1
        );
    }

    #[test]
    fn relative_looking_protocols_cannot_reach_the_backend_as_urls() {
        // caminhos relativos viram absolutos antes de qualquer processo (ver `checked_input_path`)
        let rel = Path::new("concat:x|y");
        let abs = std::path::absolute(rel).unwrap();
        assert!(abs.is_absolute());
        assert!(!abs.to_string_lossy().starts_with("concat:"));
    }
}
