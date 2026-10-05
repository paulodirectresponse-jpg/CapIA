//! Versão e build do app. A versão vem do `[workspace.package]` (fonte única, `tools/release/check-version.mjs`);
//! o id do commit e o id do build entram por variáveis de ambiente **em tempo de compilação**, se existirem.

use serde::{Deserialize, Serialize};

/// Versão do app (a mesma de `tauri.conf.json`, dos `package.json` e do `engine_info`).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub version: String,
    /// `CAPIA_GIT_SHA` no momento da compilação (CI), se presente.
    pub git_sha: Option<String>,
    /// `CAPIA_BUILD_ID` (ex.: id da execução do CI), se presente.
    pub build_id: Option<String>,
    pub os: String,
    pub arch: String,
    /// `true` se a build não é uma release assinada (rótulo dev/test).
    pub dev_build: bool,
}

impl BuildInfo {
    pub fn current() -> Self {
        Self {
            version: APP_VERSION.to_owned(),
            git_sha: option_env!("CAPIA_GIT_SHA")
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            build_id: option_env!("CAPIA_BUILD_ID")
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            dev_build: cfg!(debug_assertions) || option_env!("CAPIA_RELEASE_BUILD").is_none(),
        }
    }

    /// Texto curto para "Sobre": `0.6.0-rc.1 (windows/x86_64, abc1234)`.
    pub fn display(&self) -> String {
        let sha = self
            .git_sha
            .as_deref()
            .map(|s| format!(", {}", &s[..s.len().min(7)]))
            .unwrap_or_default();
        format!("{} ({}/{}{sha})", self.version, self.os, self.arch)
    }
}

/// Informação de sistema **sem identificadores pessoais** (sem hostname, usuário ou caminhos).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub family: String,
    pub logical_cpus: usize,
}

impl SystemInfo {
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            family: std::env::consts::FAMILY.to_owned(),
            logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_workspace_version() {
        let b = BuildInfo::current();
        assert_eq!(b.version, env!("CARGO_PKG_VERSION"));
        assert!(b.display().starts_with(&b.version));
    }
}
