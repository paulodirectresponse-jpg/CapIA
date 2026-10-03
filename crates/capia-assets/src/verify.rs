//! Disponibilidade e relink (ADR-048 §4/§5). mtime nunca decide sozinho.

use crate::error::{AssetError, AssetErrorCode};
use crate::hash::{FileDigest, hash_file};
use crate::location::resolve_candidates;
use crate::record::{AssetRecord, Availability};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Primeiro candidato que é arquivo regular.
fn first_existing(rec: &AssetRecord, project_dir: Option<&Path>) -> Option<(PathBuf, u64)> {
    resolve_candidates(&rec.location, &rec.known_paths, project_dir)
        .into_iter()
        .find_map(|p| {
            let m = std::fs::metadata(&p).ok()?;
            m.is_file().then_some((p, m.len()))
        })
}

/// Checagem **barata** (só `metadata`, O(1) por asset — é o que a abertura do projeto usa): não
/// calcula hash. Tamanho diferente já é `Modified`; mesmo tamanho é `Online` *não verificado*.
pub fn quick_status(
    rec: &AssetRecord,
    project_dir: Option<&Path>,
) -> (Availability, Option<PathBuf>) {
    match first_existing(rec, project_dir) {
        None => (Availability::Offline, None),
        Some((p, len)) if len != rec.size_bytes => (Availability::Modified, Some(p)),
        Some((p, _)) => (Availability::Online, Some(p)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    pub status: Availability,
    pub path: Option<PathBuf>,
    /// Hash recalculado (ausente se offline).
    pub actual: Option<FileDigest>,
}

/// Verificação **completa**: recalcula o hash do arquivo encontrado.
pub fn verify_content(
    rec: &AssetRecord,
    project_dir: Option<&Path>,
) -> Result<VerifyReport, AssetError> {
    let Some((path, _)) = first_existing(rec, project_dir) else {
        return Ok(VerifyReport {
            status: Availability::Offline,
            path: None,
            actual: None,
        });
    };
    let digest = hash_file(&path)?;
    let status = if digest.hash == rec.content_hash {
        Availability::Online
    } else {
        Availability::Modified
    };
    Ok(VerifyReport {
        status,
        path: Some(path),
        actual: Some(digest),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelinkCheck {
    pub digest: FileDigest,
    pub path: PathBuf,
}

/// Confere um candidato de relink: só o **mesmo conteúdo** é aceito. Conteúdo diferente ⇒
/// `ASSET_HASH_MISMATCH` estruturado com os dois hashes (nunca equivalência silenciosa). O
/// *force-relink* explícito fica para uma missão futura (ADR-048 §5).
pub fn check_relink(rec: &AssetRecord, candidate: &Path) -> Result<RelinkCheck, AssetError> {
    let path = crate::import::absolute_checked(candidate)?;
    let meta = std::fs::metadata(&path).map_err(|e| {
        AssetError::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                AssetErrorCode::AssetFileNotFound
            } else {
                AssetErrorCode::AssetIo
            },
            format!("cannot access `{}`: {e}", path.display()),
        )
    })?;
    if !meta.is_file() {
        return Err(AssetError::new(
            AssetErrorCode::AssetNotRegularFile,
            format!("`{}` is not a regular file", path.display()),
        ));
    }
    let digest = hash_file(&path)?;
    if digest.hash != rec.content_hash {
        return Err(AssetError::new(
            AssetErrorCode::AssetHashMismatch,
            format!(
                "`{}` does not have the content of asset {} (refusing to relink silently)",
                path.display(),
                rec.asset_id
            ),
        )
        .with_details(json!({
            "asset_id": rec.asset_id.as_str(),
            "expected_hash": rec.content_hash.as_str(),
            "found_hash": digest.hash.as_str(),
            "expected_size": rec.size_bytes,
            "found_size": digest.size,
            "path": path.display().to_string(),
        })));
    }
    Ok(RelinkCheck { digest, path })
}
