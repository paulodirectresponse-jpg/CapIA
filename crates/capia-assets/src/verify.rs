//! Disponibilidade e relink (ADR-048 §4/§5). mtime nunca decide sozinho.

use crate::error::{AssetError, AssetErrorCode};
use crate::hash::{FileDigest, hash_file};
use crate::location::resolve_candidates;
use crate::record::{AssetRecord, Availability};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Candidatos que existem como arquivo regular, na ordem de prioridade (no máx. 16).
fn existing_candidates(rec: &AssetRecord, project_dir: Option<&Path>) -> Vec<(PathBuf, u64)> {
    resolve_candidates(&rec.location, &rec.known_paths, project_dir)
        .into_iter()
        .filter_map(|p| {
            let m = std::fs::metadata(&p).ok()?;
            m.is_file().then_some((p, m.len()))
        })
        .take(16)
        .collect()
}

/// Checagem **barata** (só `metadata`, O(1) por asset — é o que a abertura do projeto usa): não
/// calcula hash. Prefere o candidato com o tamanho conhecido (um alias bom vale mais que um
/// caminho principal sobrescrito); se nenhum tem o tamanho, `Modified`; mesmo tamanho é `Online`
/// *não verificado*.
pub fn quick_status(
    rec: &AssetRecord,
    project_dir: Option<&Path>,
) -> (Availability, Option<PathBuf>) {
    let cands = existing_candidates(rec, project_dir);
    if let Some((p, _)) = cands.iter().find(|(_, len)| *len == rec.size_bytes) {
        return (Availability::Online, Some(p.clone()));
    }
    match cands.into_iter().next() {
        None => (Availability::Offline, None),
        Some((p, _)) => (Availability::Modified, Some(p)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    pub status: Availability,
    pub path: Option<PathBuf>,
    /// Hash recalculado (ausente se offline).
    pub actual: Option<FileDigest>,
}

/// Verificação **completa**: recalcula o hash. Só candidatos com o tamanho conhecido precisam de
/// hash (tamanho diferente já prova conteúdo diferente); o primeiro que bate vence — mesmo que seja
/// um alias. Nenhum bate ⇒ `Modified`, reportando o hash do primeiro candidato existente.
pub fn verify_content(
    rec: &AssetRecord,
    project_dir: Option<&Path>,
) -> Result<VerifyReport, AssetError> {
    let cands = existing_candidates(rec, project_dir);
    let Some((first, _)) = cands.first().cloned() else {
        return Ok(VerifyReport {
            status: Availability::Offline,
            path: None,
            actual: None,
        });
    };
    let mut first_digest = None;
    for (p, _) in cands.iter().filter(|(_, len)| *len == rec.size_bytes) {
        let digest = hash_file(p)?;
        if digest.hash == rec.content_hash {
            return Ok(VerifyReport {
                status: Availability::Online,
                path: Some(p.clone()),
                actual: Some(digest),
            });
        }
        if *p == first {
            first_digest = Some(digest);
        }
    }
    let digest = match first_digest {
        Some(d) => d,
        None => hash_file(&first)?,
    };
    Ok(VerifyReport {
        status: Availability::Modified,
        path: Some(first),
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
