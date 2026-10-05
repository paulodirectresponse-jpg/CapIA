//! Logs locais com rotação **limitada** (Fase 6 §26). Toda linha passa pela redação (segredos + caminhos
//! pessoais) antes de tocar o disco. Limite total = `max_file_bytes × max_files` (padrão 1 MiB × 5 = 5 MiB).
//!
//! Local (Windows): `%APPDATA%\app.capia.desktop\logs\` (o diretório de dados do app, o mesmo do `capia-app.db`).

use crate::util::{now_ms, sanitize, truncate_chars};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Linhas maiores que isto são truncadas (um log nunca vira despejo de conteúdo).
pub const MAX_LINE_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogPolicy {
    pub max_file_bytes: u64,
    pub max_files: usize,
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
            max_files: 5,
        }
    }
}

impl LogPolicy {
    /// Teto de bytes em disco para um log.
    pub fn max_total_bytes(&self) -> u64 {
        self.max_file_bytes.saturating_mul(self.max_files as u64)
    }
}

#[derive(Debug)]
pub struct RotatingLog {
    dir: PathBuf,
    stem: String,
    policy: LogPolicy,
    lock: Mutex<()>,
}

impl RotatingLog {
    pub fn new(dir: &Path, stem: &str, policy: LogPolicy) -> io::Result<Self> {
        if stem.is_empty() || stem.contains(['/', '\\', '.']) || policy.max_files == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid log stem or policy",
            ));
        }
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            stem: stem.to_owned(),
            policy,
            lock: Mutex::new(()),
        })
    }

    fn path(&self, index: usize) -> PathBuf {
        if index == 0 {
            self.dir.join(format!("{}.log", self.stem))
        } else {
            self.dir.join(format!("{}.{index}.log", self.stem))
        }
    }

    /// Arquivos existentes, do mais novo para o mais antigo.
    pub fn files(&self) -> Vec<PathBuf> {
        (0..self.policy.max_files)
            .map(|i| self.path(i))
            .filter(|p| p.is_file())
            .collect()
    }

    pub fn total_bytes(&self) -> u64 {
        self.files()
            .iter()
            .filter_map(|p| fs::metadata(p).ok())
            .map(|m| m.len())
            .sum()
    }

    fn rotate(&self) -> io::Result<()> {
        let last = self.policy.max_files - 1;
        if last == 0 {
            // um único arquivo: recomeça
            return fs::write(self.path(0), b"");
        }
        let oldest = self.path(last);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for i in (0..last).rev() {
            let from = self.path(i);
            if from.exists() {
                fs::rename(&from, self.path(i + 1))?;
            }
        }
        Ok(())
    }

    /// Acrescenta uma linha `<ms> <texto>` (redigida e truncada). Rotaciona antes de passar do limite.
    pub fn append_line(&self, line: &str) -> io::Result<()> {
        let text = truncate_chars(&sanitize(line).replace(['\r', '\n'], " "), MAX_LINE_BYTES);
        let record = format!("{} {text}\n", now_ms());
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = fs::metadata(self.path(0)).map_or(0, |m| m.len());
        if current > 0 && current + record.len() as u64 > self.policy.max_file_bytes {
            self.rotate()?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(0))?;
        f.write_all(record.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::util::testutil::Tmp;

    #[test]
    fn rotation_is_bounded_in_size_and_count() {
        let t = Tmp::new("log");
        let policy = LogPolicy {
            max_file_bytes: 400,
            max_files: 3,
        };
        let log = RotatingLog::new(&t.0, "capia", policy).unwrap();
        for i in 0..500 {
            log.append_line(&format!("event number {i} with some padding text"))
                .unwrap();
        }
        assert!(log.files().len() <= 3);
        assert!(log.total_bytes() <= policy.max_total_bytes());
        // a linha mais recente sobrevive
        let newest = fs::read_to_string(log.path(0)).unwrap();
        assert!(newest.contains("event number 499"));
        // e o diretório não acumulou arquivos além do limite
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 3);
    }

    #[test]
    fn single_file_policy_restarts_instead_of_growing() {
        let t = Tmp::new("log1");
        let policy = LogPolicy {
            max_file_bytes: 200,
            max_files: 1,
        };
        let log = RotatingLog::new(&t.0, "x", policy).unwrap();
        for i in 0..100 {
            log.append_line(&format!("line {i} ........................"))
                .unwrap();
        }
        assert!(log.total_bytes() <= 200);
    }

    #[test]
    fn lines_are_redacted_and_truncated() {
        const CANARY: &str = "CNRY-log-5d2f9c01aa77b3e4";
        capia_secrets::register_global(CANARY);
        let t = Tmp::new("log2");
        let log = RotatingLog::new(&t.0, "capia", LogPolicy::default()).unwrap();
        log.append_line(&format!("key={CANARY}\nsecond line"))
            .unwrap();
        log.append_line(&"x".repeat(100_000)).unwrap();
        let text = fs::read_to_string(log.path(0)).unwrap();
        assert!(!text.contains(CANARY));
        assert!(text.len() < 2 * MAX_LINE_BYTES + 200);
        assert_eq!(
            text.lines().count(),
            2,
            "newlines inside a record are flattened"
        );
    }

    #[test]
    fn rejects_path_like_stems() {
        let t = Tmp::new("log3");
        assert!(RotatingLog::new(&t.0, "../evil", LogPolicy::default()).is_err());
        assert!(RotatingLog::new(&t.0, "a.b", LogPolicy::default()).is_err());
    }
}
