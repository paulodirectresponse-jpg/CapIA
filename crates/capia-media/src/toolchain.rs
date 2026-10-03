//! Localização do FFmpeg/ffprobe (ADR-047 §4): configurado pelo app → variáveis de ambiente →
//! diretório *bundled* → `PATH`. Nada encontrado ⇒ `MEDIA_BACKEND_NOT_FOUND`.

use crate::error::{MediaError, MediaErrorCode};
use crate::limits::DEFAULT_PROBE_TIMEOUT;
use crate::process::{RunLimits, run_bounded};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct MediaConfig {
    /// Caminho explícito do `ffprobe` (maior prioridade).
    pub ffprobe: Option<PathBuf>,
    /// Caminho explícito do `ffmpeg`.
    pub ffmpeg: Option<PathBuf>,
    /// Diretório *bundled* (instalador futuro). Padrão: `<diretório do executável>/ffmpeg`.
    pub bundled_dir: Option<PathBuf>,
    /// Timeout do probe (padrão 30 s).
    pub probe_timeout: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Configured,
    Environment,
    Bundled,
    Path,
}

#[derive(Clone, Debug)]
pub struct MediaToolchain {
    pub ffprobe: PathBuf,
    pub ffprobe_source: ToolSource,
    /// Ausente ⇒ só probe (miniaturas falham com `MEDIA_BACKEND_NOT_FOUND`).
    pub ffmpeg: Option<PathBuf>,
    /// Primeira linha de `ffprobe -version` (diagnóstico/CI).
    pub version: String,
    pub probe_timeout: Duration,
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

fn find_in_dir(dir: &Path, base: &str) -> Option<PathBuf> {
    let p = dir.join(exe_name(base));
    p.is_file().then_some(p)
}

fn find_in_path(base: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .find_map(|d| find_in_dir(&d, base))
}

fn not_found(msg: String) -> MediaError {
    MediaError::new(MediaErrorCode::MediaBackendNotFound, msg)
}

/// Um caminho **explícito** (config/env) que não existe é erro de configuração: não cai
/// silenciosamente para o `PATH`.
fn explicit(p: &Path, what: &str) -> Result<PathBuf, MediaError> {
    if p.is_file() {
        Ok(p.to_path_buf())
    } else {
        Err(not_found(format!(
            "configured {what} path does not exist: {}",
            p.display()
        )))
    }
}

fn resolve(
    base: &str,
    configured: Option<&Path>,
    env: &str,
    bundled: Option<&Path>,
) -> Result<Option<(PathBuf, ToolSource)>, MediaError> {
    if let Some(p) = configured {
        return explicit(p, base).map(|p| Some((p, ToolSource::Configured)));
    }
    if let Some(v) = std::env::var_os(env).filter(|v| !v.is_empty()) {
        return explicit(Path::new(&v), base).map(|p| Some((p, ToolSource::Environment)));
    }
    if let Some(d) = bundled
        && let Some(p) = find_in_dir(d, base)
    {
        return Ok(Some((p, ToolSource::Bundled)));
    }
    Ok(find_in_path(base).map(|p| (p, ToolSource::Path)))
}

impl MediaToolchain {
    pub fn locate(config: &MediaConfig) -> Result<Self, MediaError> {
        let bundled = config.bundled_dir.clone().or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|d| d.join("ffmpeg")))
        });
        let probe = resolve(
            "ffprobe",
            config.ffprobe.as_deref(),
            "CAPIA_FFPROBE",
            bundled.as_deref(),
        )?;
        let Some((ffprobe, ffprobe_source)) = probe else {
            return Err(not_found(
                "ffprobe was not found (configure a path, set CAPIA_FFPROBE, bundle it or put it on PATH)"
                    .into(),
            ));
        };
        let ffmpeg = resolve(
            "ffmpeg",
            config.ffmpeg.as_deref(),
            "CAPIA_FFMPEG",
            bundled.as_deref(),
        )?
        .map(|(p, _)| p);
        let version = Self::query_version(&ffprobe)?;
        Ok(Self {
            ffprobe,
            ffprobe_source,
            ffmpeg,
            version,
            probe_timeout: config.probe_timeout.unwrap_or(DEFAULT_PROBE_TIMEOUT),
        })
    }

    fn query_version(ffprobe: &Path) -> Result<String, MediaError> {
        let out = run_bounded(
            ffprobe,
            &[OsString::from("-version")],
            &RunLimits {
                timeout: Duration::from_secs(10),
                max_stdout: 64 * 1024,
                max_stderr: 4 * 1024,
            },
        )?;
        if !out.status.success() {
            return Err(MediaError::new(
                MediaErrorCode::MediaBackendFailed,
                format!("`{} -version` failed", ffprobe.display()),
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_missing_configured_path_is_an_error_not_a_silent_fallback() {
        let cfg = MediaConfig {
            ffprobe: Some(PathBuf::from("/definitely/not/here/ffprobe")),
            ..MediaConfig::default()
        };
        let e = MediaToolchain::locate(&cfg).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaBackendNotFound);
    }

    #[test]
    fn nothing_anywhere_is_backend_not_found() {
        // bundled vazio + PATH vazio: usamos um diretório inexistente e nenhum PATH utilizável
        let found = resolve(
            "capia-no-such-tool-xyz",
            None,
            "CAPIA_NO_SUCH_ENV",
            Some(Path::new("/nonexistent")),
        );
        assert!(found.unwrap().is_none());
    }
}
