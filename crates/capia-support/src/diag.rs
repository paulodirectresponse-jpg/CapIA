//! Pacote de diagnóstico (Fase 6 §25): acionado pelo usuário, com **preview** do que será incluído.
//!
//! Inclui: versão/build, informação de sistema sem identificadores pessoais, preferências de privacidade,
//! logs (cauda, redigidos), erros estruturados recentes, registros locais de crash. **Nunca**: chaves,
//! mídia, prompts, conteúdo de projeto/timeline, caminhos pessoais, variáveis de ambiente. Todo texto
//! passa por `capia_secrets::redact_global` (+ remoção de caminhos pessoais) **depois** de lido do disco —
//! mesmo que alguém tenha escrito um segredo cru num arquivo de log, ele não sai no pacote.

use crate::buildinfo::{BuildInfo, SystemInfo};
use crate::crash::CrashStore;
use crate::error::SupportError;
use crate::logs::{LogPolicy, RotatingLog};
use crate::settings::SupportSettings;
use crate::util::{now_ms, sanitize};
use serde::Serialize;
use serde_json::json;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Declaração mostrada ao usuário no preview.
pub const NEVER_INCLUDED: &[&str] = &[
    "API keys, tokens and any credential",
    "Media files (video, audio, images, proxies, frames)",
    "AI prompts, transcripts and model responses",
    "Project files and timeline contents",
    "Personal paths (your user name is replaced by `~`)",
    "Environment variables and machine/host names",
];

const LOG_TAIL_BYTES: u64 = 256 * 1024;
const MAX_ERROR_LINES: usize = 50;
const MAX_CRASH_RECORDS: usize = 10;
pub const ERRORS_STEM: &str = "errors";

/// Diretórios do app usados pelo suporte, sob o diretório de dados do usuário.
#[derive(Clone, Debug)]
pub struct SupportDirs {
    pub logs: PathBuf,
    pub crash: PathBuf,
}

impl SupportDirs {
    pub fn under(app_data: &Path) -> Self {
        Self {
            logs: app_data.join("logs"),
            crash: app_data.join("support").join("crash"),
        }
    }

    pub fn ensure(&self) -> Result<(), SupportError> {
        fs::create_dir_all(&self.logs)?;
        fs::create_dir_all(&self.crash)?;
        Ok(())
    }

    /// Log de erros estruturados (JSON por linha), com rotação limitada.
    pub fn error_log(&self) -> Result<StructuredErrorLog, SupportError> {
        Ok(StructuredErrorLog {
            log: RotatingLog::new(&self.logs, ERRORS_STEM, LogPolicy::default())?,
        })
    }

    pub fn app_log(&self) -> Result<RotatingLog, SupportError> {
        Ok(RotatingLog::new(&self.logs, "capia", LogPolicy::default())?)
    }
}

#[derive(Debug)]
pub struct StructuredErrorLog {
    log: RotatingLog,
}

impl StructuredErrorLog {
    pub fn record(&self, component: &str, code: &str, message: &str) {
        let line = json!({"component": component, "code": code, "message": message}).to_string();
        let _ = self.log.append_line(&line);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreviewEntry {
    pub path: String,
    pub bytes: u64,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticPreview {
    pub entries: Vec<PreviewEntry>,
    pub total_bytes: u64,
    pub never_included: Vec<String>,
}

struct Entry {
    path: String,
    data: Vec<u8>,
    description: String,
}

#[derive(Debug)]
pub struct DiagnosticBuilder {
    build: BuildInfo,
    system: SystemInfo,
    dirs: SupportDirs,
    settings: SupportSettings,
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Últimos `max` bytes de um arquivo, como texto (UTF-8 com substituição).
fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    // descarta a linha cortada no começo
    Ok(if start > 0 {
        text.split_once('\n')
            .map_or(String::new(), |(_, r)| r.to_owned())
    } else {
        text
    })
}

impl DiagnosticBuilder {
    pub fn new(build: BuildInfo, dirs: SupportDirs, settings: SupportSettings) -> Self {
        Self {
            build,
            system: SystemInfo::current(),
            dirs,
            settings,
        }
    }

    fn json_entry<T: Serialize>(path: &str, value: &T, description: &str) -> Entry {
        let data = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned());
        Entry {
            path: path.to_owned(),
            data: sanitize(&data).into_bytes(),
            description: description.to_owned(),
        }
    }

    fn collect(&self) -> Vec<Entry> {
        let mut out = vec![
            Self::json_entry(
                "app.json",
                &self.build,
                "App version, build id, OS and architecture",
            ),
            Self::json_entry(
                "system.json",
                &self.system,
                "OS, architecture and CPU count (no host or user names)",
            ),
            Self::json_entry(
                "settings.json",
                &json!({
                    "crash_reporting": self.settings.crash_reporting,
                    "update_channel": self.settings.update_channel,
                }),
                "Your privacy and update-channel choices",
            ),
        ];

        // logs de aplicação (tudo que é `*.log` fora do log de erros), só a cauda
        let mut logs: Vec<PathBuf> = fs::read_dir(&self.dirs.logs)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension().is_some_and(|e| e == "log")
                    && !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(ERRORS_STEM))
            })
            .collect();
        logs.sort();
        for p in logs {
            let Ok(text) = read_tail(&p, LOG_TAIL_BYTES) else {
                continue;
            };
            let name = safe_name(
                &p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            out.push(Entry {
                path: format!("logs/{name}"),
                data: sanitize(&text).into_bytes(),
                description: "Application log (redacted, most recent part)".to_owned(),
            });
        }

        // erros estruturados recentes (mais novos primeiro entre arquivos; saída em ordem cronológica)
        if let Ok(errors) = RotatingLog::new(&self.dirs.logs, ERRORS_STEM, LogPolicy::default()) {
            let mut lines: Vec<String> = Vec::new();
            for f in errors.files() {
                if let Ok(t) = read_tail(&f, LOG_TAIL_BYTES) {
                    let mut chunk: Vec<String> = t.lines().map(str::to_owned).collect();
                    chunk.append(&mut lines);
                    lines = chunk;
                }
                if lines.len() >= MAX_ERROR_LINES {
                    break;
                }
            }
            let keep = lines.len().saturating_sub(MAX_ERROR_LINES);
            let body = lines[keep..].join("\n");
            if !body.is_empty() {
                out.push(Entry {
                    path: "errors.jsonl".to_owned(),
                    data: sanitize(&body).into_bytes(),
                    description: format!(
                        "Recent structured errors (last {MAX_ERROR_LINES}, redacted)"
                    ),
                });
            }
        }

        // registros locais de crash (já redigidos na origem; redigidos de novo na saída)
        if let Ok(store) = CrashStore::new(&self.dirs.crash, MAX_CRASH_RECORDS) {
            let list = store.list();
            let skip = list.len().saturating_sub(MAX_CRASH_RECORDS);
            for (p, _) in list.into_iter().skip(skip) {
                if let Ok(text) = fs::read_to_string(&p) {
                    let name = safe_name(
                        &p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    );
                    out.push(Entry {
                        path: format!("crash/{name}"),
                        data: sanitize(&text).into_bytes(),
                        description:
                            "Local crash record (redacted; never uploaded unless you opted in)"
                                .to_owned(),
                    });
                }
            }
        }

        let manifest = json!({
            "generated_ms": now_ms(),
            "app_version": self.build.version,
            "files": out.iter().map(|e| json!({"path": e.path, "bytes": e.data.len(), "description": e.description})).collect::<Vec<_>>(),
            "never_included": NEVER_INCLUDED,
        });
        out.push(Self::json_entry(
            "manifest.json",
            &manifest,
            "List of everything in this bundle",
        ));
        out
    }

    /// O que **exatamente** iria no pacote, para o usuário conferir antes de compartilhar.
    pub fn preview(&self) -> DiagnosticPreview {
        Self::preview_of(&self.collect())
    }

    fn preview_of(entries: &[Entry]) -> DiagnosticPreview {
        let list: Vec<PreviewEntry> = entries
            .iter()
            .map(|e| PreviewEntry {
                path: e.path.clone(),
                bytes: e.data.len() as u64,
                description: e.description.clone(),
            })
            .collect();
        DiagnosticPreview {
            total_bytes: list.iter().map(|e| e.bytes).sum(),
            entries: list,
            never_included: NEVER_INCLUDED.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// Grava o ZIP (staging + rename). Devolve o que foi escrito (igual ao preview, salvo mudanças de log
    /// entre as duas chamadas).
    pub fn write_zip(&self, out: &Path) -> Result<DiagnosticPreview, SupportError> {
        let entries = self.collect();
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = out.with_extension("zip.tmp");
        {
            let mut zip = zip::ZipWriter::new(File::create(&tmp)?);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for e in &entries {
                zip.start_file(&e.path, opts)?;
                zip.write_all(&e.data)?;
            }
            zip.finish()?;
        }
        fs::rename(&tmp, out)?;
        Ok(Self::preview_of(&entries))
    }

    /// Variante em diretório (para inspeção/CI).
    pub fn write_dir(&self, dir: &Path) -> Result<DiagnosticPreview, SupportError> {
        let entries = self.collect();
        for e in &entries {
            let p = dir.join(&e.path);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(p, &e.data)?;
        }
        Ok(Self::preview_of(&entries))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::util::testutil::Tmp;

    fn builder(t: &Tmp) -> (DiagnosticBuilder, SupportDirs) {
        let dirs = SupportDirs::under(&t.0);
        dirs.ensure().unwrap();
        (
            DiagnosticBuilder::new(
                BuildInfo::current(),
                dirs.clone(),
                SupportSettings::default(),
            ),
            dirs,
        )
    }

    #[test]
    fn preview_matches_what_is_written() {
        let t = Tmp::new("diag");
        let (b, dirs) = builder(&t);
        dirs.app_log().unwrap().append_line("started").unwrap();
        dirs.error_log()
            .unwrap()
            .record("media", "PROBE_FAILED", "ffprobe exited 1");
        let pv = b.preview();
        let paths: Vec<_> = pv.entries.iter().map(|e| e.path.as_str()).collect();
        for want in [
            "app.json",
            "system.json",
            "settings.json",
            "logs/capia.log",
            "errors.jsonl",
            "manifest.json",
        ] {
            assert!(paths.contains(&want), "missing {want}: {paths:?}");
        }
        assert!(!pv.never_included.is_empty());
        let out = t.0.join("out").join("diag.zip");
        let written = b.write_zip(&out).unwrap();
        assert_eq!(written.entries, pv.entries);
        let mut z = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        assert_eq!(z.len(), pv.entries.len());
        let mut m = String::new();
        z.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut m)
            .unwrap();
        assert!(m.contains("never_included"));
    }

    #[test]
    fn bundle_never_contains_the_canary_even_when_written_raw() {
        const CANARY: &str = "CNRY-diag-a1b2c3d4e5f60718";
        capia_secrets::register_global(CANARY);
        let t = Tmp::new("diag2");
        let (b, dirs) = builder(&t);
        // escrita crua (sem passar pela redação do RotatingLog), como um log de terceiros faria
        fs::write(
            dirs.logs.join("raw.log"),
            format!("1 token {CANARY}\n2 ok\n"),
        )
        .unwrap();
        fs::write(
            dirs.logs.join("errors.log"),
            format!("{{\"message\":\"{CANARY}\"}}\n"),
        )
        .unwrap();
        fs::write(
            dirs.crash.join("crash-0000000000001-aa.json"),
            format!("{{\"message\":\"{CANARY}\"}}"),
        )
        .unwrap();
        let dir = t.0.join("outdir");
        b.write_dir(&dir).unwrap();
        let zip = t.0.join("o.zip");
        b.write_zip(&zip).unwrap();
        let mut found = 0;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    found += 1;
                    assert!(
                        !String::from_utf8_lossy(&fs::read(e.path()).unwrap()).contains(CANARY)
                    );
                }
            }
        }
        assert!(found >= 6);
        let mut z = zip::ZipArchive::new(File::open(&zip).unwrap()).unwrap();
        for i in 0..z.len() {
            let mut s = Vec::new();
            z.by_index(i).unwrap().read_to_end(&mut s).unwrap();
            assert!(!String::from_utf8_lossy(&s).contains(CANARY));
        }
        // o próprio arquivo ZIP bruto também não contém (deflate não esconderia texto curto, mas confere)
        assert!(!String::from_utf8_lossy(&fs::read(&zip).unwrap()).contains(CANARY));
    }

    #[test]
    fn only_known_file_kinds_are_collected() {
        let t = Tmp::new("diag3");
        let (b, dirs) = builder(&t);
        fs::write(
            dirs.logs.join("project.capia"),
            b"SQLite format 3 secret project",
        )
        .unwrap();
        fs::write(dirs.logs.join("clip.mp4"), b"mediabytes").unwrap();
        fs::write(dirs.logs.join("notes.txt"), b"private notes").unwrap();
        let pv = b.preview();
        assert!(
            pv.entries
                .iter()
                .all(|e| !e.path.contains("capia") || e.path == "logs/capia.log")
        );
        assert!(
            pv.entries
                .iter()
                .all(|e| !e.path.ends_with(".mp4") && !e.path.ends_with(".txt"))
        );
    }

    #[test]
    fn log_tail_and_error_count_are_bounded() {
        let t = Tmp::new("diag4");
        let (b, dirs) = builder(&t);
        let big = "line of text that repeats\n".repeat(100_000);
        fs::write(dirs.logs.join("big.log"), big).unwrap();
        let errs = (0..500)
            .map(|i| format!("{{\"n\":{i}}}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dirs.logs.join("errors.log"), errs).unwrap();
        let pv = b.preview();
        let big_entry = pv
            .entries
            .iter()
            .find(|e| e.path == "logs/big.log")
            .unwrap();
        assert!(big_entry.bytes <= LOG_TAIL_BYTES + 64);
        let e = pv
            .entries
            .iter()
            .find(|e| e.path == "errors.jsonl")
            .unwrap();
        let out = t.0.join("d");
        b.write_dir(&out).unwrap();
        let text = fs::read_to_string(out.join("errors.jsonl")).unwrap();
        assert!(e.bytes > 0);
        assert_eq!(text.lines().count(), MAX_ERROR_LINES);
        assert!(text.contains("\"n\":499"));
    }

    #[test]
    fn personal_home_path_is_scrubbed() {
        let t = Tmp::new("diag5");
        let (b, dirs) = builder(&t);
        if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned())
            && home.len() > 3
        {
            fs::write(
                dirs.logs.join("p.log"),
                format!("opened {home}/Videos/x.mp4\n"),
            )
            .unwrap();
            let out = t.0.join("d2");
            b.write_dir(&out).unwrap();
            let text = fs::read_to_string(out.join("logs/p.log")).unwrap();
            assert!(!text.contains(&home), "{text}");
            assert!(text.contains("~/Videos/x.mp4"));
        }
    }
}
