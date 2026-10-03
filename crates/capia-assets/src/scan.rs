//! Varredura de pasta e relink em lote (ADR-058). **Nunca por nome**: um arquivo só é candidato
//! se tem o **tamanho** do asset; depois a impressão rápida filtra; só o **SHA-256 completo**
//! decide. Varredura segura: profundidade e nº de arquivos limitados, symlinks/junctions não
//! seguidos por padrão, sem laços, erros de permissão registrados (não fatais).

use crate::error::{AssetError, AssetErrorCode};
use crate::fingerprint::fingerprint_file;
use crate::hash::{ContentHash, FileDigest, hash_file_job};
use crate::record::AssetRecord;
use capia_model::AssetId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub max_depth: u32,
    pub max_files: u64,
    /// Seguir symlinks/junctions (padrão: **não**). Mesmo seguindo, laços são detectados.
    pub follow_links: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_depth: 16,
            max_files: 500_000,
            follow_links: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub files: Vec<ScannedFile>,
    pub dirs_visited: u64,
    /// Atingiu `max_files`: a lista é parcial.
    pub truncated: bool,
    /// Diretórios abaixo de `max_depth` que não foram visitados.
    pub depth_limited: u64,
    pub skipped_links: u64,
    pub errors: Vec<ScanIssue>,
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT (symlink e junction)
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn cancelled() -> AssetError {
    AssetError::new(AssetErrorCode::AssetCancelled, "scan was cancelled")
}

fn issue(path: &Path, e: &std::io::Error) -> ScanIssue {
    ScanIssue {
        path: path.display().to_string(),
        code: if e.kind() == std::io::ErrorKind::PermissionDenied {
            "PERMISSION_DENIED".into()
        } else {
            "IO_ERROR".into()
        },
        message: e.to_string(),
    }
}

/// Lista os arquivos regulares de `root` (largura-primeiro, ordem determinística).
pub fn scan_folder(
    root: &Path,
    opts: &ScanOptions,
    cancel: &dyn Fn() -> bool,
) -> Result<ScanReport, AssetError> {
    let meta = std::fs::metadata(root).map_err(|e| {
        AssetError::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                AssetErrorCode::AssetFileNotFound
            } else {
                AssetErrorCode::AssetIo
            },
            format!("cannot access `{}`: {e}", root.display()),
        )
    })?;
    if !meta.is_dir() {
        return Err(AssetError::new(
            AssetErrorCode::AssetPathInvalid,
            format!("`{}` is not a directory", root.display()),
        ));
    }
    let mut report = ScanReport::default();
    let mut seen_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    if let Ok(c) = std::fs::canonicalize(root) {
        seen_dirs.insert(c);
    }
    let mut level: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut depth = 0u32;
    while !level.is_empty() {
        let mut next: Vec<PathBuf> = Vec::new();
        for dir in level {
            if cancel() {
                return Err(cancelled());
            }
            let rd = match std::fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) => {
                    report.errors.push(issue(&dir, &e));
                    continue;
                }
            };
            report.dirs_visited += 1;
            let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for e in entries {
                let path = e.path();
                let meta = match e.metadata() {
                    Ok(m) => m,
                    Err(err) => {
                        report.errors.push(issue(&path, &err));
                        continue;
                    }
                };
                let link = is_link(&meta);
                if link && !opts.follow_links {
                    report.skipped_links += 1;
                    continue;
                }
                // seguindo links, o alvo real decide o tipo
                let meta = if link {
                    match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(err) => {
                            report.errors.push(issue(&path, &err));
                            continue;
                        }
                    }
                } else {
                    meta
                };
                if meta.is_dir() {
                    if depth >= opts.max_depth {
                        report.depth_limited += 1;
                        continue;
                    }
                    // laços (links apontando para um ancestral) e diretórios repetidos
                    match std::fs::canonicalize(&path) {
                        Ok(c) => {
                            if !seen_dirs.insert(c) {
                                report.skipped_links += 1;
                                continue;
                            }
                        }
                        Err(err) => {
                            report.errors.push(issue(&path, &err));
                            continue;
                        }
                    }
                    next.push(path);
                } else if meta.is_file() {
                    if report.files.len() as u64 >= opts.max_files {
                        report.truncated = true;
                        return Ok(report);
                    }
                    report.files.push(ScannedFile {
                        path,
                        size: meta.len(),
                    });
                }
            }
        }
        level = next;
        depth += 1;
    }
    Ok(report)
}

/// O que procurar por um asset.
#[derive(Clone, Debug)]
pub struct RelinkTarget {
    pub asset_id: AssetId,
    pub size: u64,
    pub hash: ContentHash,
    pub fingerprint: Option<String>,
}

impl From<&AssetRecord> for RelinkTarget {
    fn from(r: &AssetRecord) -> Self {
        Self {
            asset_id: r.asset_id.clone(),
            size: r.size_bytes,
            hash: r.content_hash.clone(),
            fingerprint: r.fingerprint.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedAsset {
    pub asset_id: AssetId,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguousAsset {
    pub asset_id: AssetId,
    /// Vários arquivos com o conteúdo certo: nada é relinkado sem escolha explícita.
    pub candidates: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub asset_id: AssetId,
    pub path: PathBuf,
    pub reason: String,
    pub found_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchError {
    pub asset_id: Option<AssetId>,
    pub path: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchRelinkReport {
    pub matched: Vec<MatchedAsset>,
    pub unresolved: Vec<AssetId>,
    pub ambiguous: Vec<AmbiguousAsset>,
    pub rejected: Vec<RejectedCandidate>,
    pub errors: Vec<MatchError>,
    pub scanned_files: u64,
    pub scan_truncated: bool,
    pub skipped_links: u64,
    pub depth_limited_dirs: u64,
    /// Arquivos que passaram por SHA-256 completo (custo real da operação).
    pub hashed_files: u64,
    /// Candidatos de mesmo tamanho descartados pela impressão rápida (sem hash completo).
    pub dropped_by_fingerprint: u64,
}

/// Casa `targets` com os arquivos da varredura. `progress(alvos_processados, total)`.
pub fn match_candidates(
    targets: &[RelinkTarget],
    scan: &ScanReport,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<BatchRelinkReport, AssetError> {
    let mut report = BatchRelinkReport {
        scanned_files: scan.files.len() as u64,
        scan_truncated: scan.truncated,
        skipped_links: scan.skipped_links,
        depth_limited_dirs: scan.depth_limited,
        ..Default::default()
    };
    for i in &scan.errors {
        report.errors.push(MatchError {
            asset_id: None,
            path: Some(i.path.clone()),
            code: i.code.clone(),
            message: i.message.clone(),
        });
    }
    let mut by_size: HashMap<u64, Vec<&ScannedFile>> = HashMap::new();
    for f in &scan.files {
        by_size.entry(f.size).or_default().push(f);
    }
    let mut fps: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut digests: HashMap<PathBuf, Result<FileDigest, AssetError>> = HashMap::new();
    let total = targets.len() as u64;
    for (n, t) in targets.iter().enumerate() {
        if cancel() {
            return Err(cancelled());
        }
        let mut verified: Vec<PathBuf> = Vec::new();
        for f in by_size.get(&t.size).map(Vec::as_slice).unwrap_or_default() {
            if cancel() {
                return Err(cancelled());
            }
            // 1) impressão rápida (se o asset tem uma e o arquivo é legível)
            if let Some(want) = &t.fingerprint {
                let got = fps
                    .entry(f.path.clone())
                    .or_insert_with(|| fingerprint_file(&f.path).ok().map(|x| x.to_text()));
                if got.as_deref() != Some(want.as_str()) {
                    report.dropped_by_fingerprint += 1;
                    continue;
                }
            }
            // 2) SHA-256 completo decide (uma vez por arquivo, mesmo com vários alvos)
            let d = digests.entry(f.path.clone()).or_insert_with(|| {
                report.hashed_files += 1;
                hash_file_job(&f.path, cancel, &mut |_, _| {})
            });
            match d {
                Ok(d) if d.hash == t.hash => verified.push(f.path.clone()),
                Ok(d) => report.rejected.push(RejectedCandidate {
                    asset_id: t.asset_id.clone(),
                    path: f.path.clone(),
                    reason: "same size and fingerprint but different content (SHA-256)".into(),
                    found_hash: d.hash.to_string(),
                }),
                Err(e) if e.code == AssetErrorCode::AssetCancelled => return Err(e.clone()),
                Err(e) => report.errors.push(MatchError {
                    asset_id: Some(t.asset_id.clone()),
                    path: Some(f.path.display().to_string()),
                    code: e.code.as_str().to_owned(),
                    message: e.message.clone(),
                }),
            }
        }
        match verified.len() {
            0 => report.unresolved.push(t.asset_id.clone()),
            1 => report.matched.push(MatchedAsset {
                asset_id: t.asset_id.clone(),
                size: t.size,
                path: verified.remove(0),
            }),
            _ => report.ambiguous.push(AmbiguousAsset {
                asset_id: t.asset_id.clone(),
                candidates: verified,
            }),
        }
        progress(n as u64 + 1, total);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(n: &str) -> Self {
            let p = std::env::temp_dir().join(format!("capia-scan-{n}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn scan_respects_depth_and_file_limits_and_is_deterministic() {
        let t = Tmp::new("limits");
        for d in 0..6 {
            let rel = (0..=d)
                .map(|i| format!("d{i}"))
                .collect::<Vec<_>>()
                .join("/");
            put(&t.0, &format!("{rel}/f.bin"), b"x");
        }
        let all = scan_folder(&t.0, &ScanOptions::default(), &|| false).unwrap();
        assert_eq!(all.files.len(), 6);
        assert_eq!(
            all,
            scan_folder(&t.0, &ScanOptions::default(), &|| false).unwrap()
        );
        let shallow = scan_folder(
            &t.0,
            &ScanOptions {
                max_depth: 2,
                ..Default::default()
            },
            &|| false,
        )
        .unwrap();
        assert!(shallow.files.len() < 6 && shallow.depth_limited > 0);
        let capped = scan_folder(
            &t.0,
            &ScanOptions {
                max_files: 2,
                ..Default::default()
            },
            &|| false,
        )
        .unwrap();
        assert!(capped.truncated && capped.files.len() == 2);
        assert!(scan_folder(&t.0.join("nope"), &ScanOptions::default(), &|| false).is_err());
        assert!(scan_folder(&t.0.join("d0/f.bin"), &ScanOptions::default(), &|| false).is_err());
        assert_eq!(
            scan_folder(&t.0, &ScanOptions::default(), &|| true)
                .unwrap_err()
                .code,
            AssetErrorCode::AssetCancelled
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped_by_default_and_loops_never_hang() {
        let t = Tmp::new("links");
        put(&t.0, "real/a.bin", b"aaa");
        std::os::unix::fs::symlink(t.0.join("real"), t.0.join("alias")).unwrap();
        std::os::unix::fs::symlink(&t.0, t.0.join("real/loop")).unwrap();
        std::os::unix::fs::symlink(t.0.join("real/a.bin"), t.0.join("file_link")).unwrap();
        let r = scan_folder(&t.0, &ScanOptions::default(), &|| false).unwrap();
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.skipped_links, 3);
        // seguindo links: não entra em laço e não duplica diretórios já vistos
        let f = scan_folder(
            &t.0,
            &ScanOptions {
                follow_links: true,
                ..Default::default()
            },
            &|| false,
        )
        .unwrap();
        assert!(f.files.len() <= 2, "{:?}", f.files);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directories_are_reported_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let t = Tmp::new("perm");
        put(&t.0, "ok/a.bin", b"a");
        put(&t.0, "locked/b.bin", b"b");
        std::fs::set_permissions(t.0.join("locked"), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        let r = scan_folder(&t.0, &ScanOptions::default(), &|| false).unwrap();
        std::fs::set_permissions(t.0.join("locked"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        // root ignora permissões: nesse caso lê tudo; senão registra o erro e segue
        assert!(r.files.iter().any(|f| f.path.ends_with("a.bin")));
        if r.files.len() == 1 {
            assert_eq!(r.errors.len(), 1);
            assert_eq!(r.errors[0].code, "PERMISSION_DENIED");
        }
    }
}
