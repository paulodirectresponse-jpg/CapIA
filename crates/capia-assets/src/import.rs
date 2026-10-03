//! Import (ADR-046/048): verificar → hash em streaming → probe → classificar → registro.
//! `prepare_import` **não escreve nada** em lugar nenhum: o chamador só persiste (de forma
//! atômica) se tudo isto tiver dado certo — probe falho não deixa meio asset registrado.

use crate::error::{AssetError, AssetErrorCode};
use crate::hash::{ContentHash, hash_file};
use crate::location::AssetLocation;
use crate::record::{AssetRecord, Availability};
use capia_media::{MAX_PATH_BYTES, MediaProbe};
use capia_model::AssetId;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct PreparedAsset {
    pub record: AssetRecord,
    /// Caminho absoluto efetivamente lido.
    pub path: PathBuf,
}

/// `ast_` + 32 primeiros hex do hash (ADR-046 §3).
pub fn asset_id_for(hash: &ContentHash) -> AssetId {
    AssetId::new(format!("ast_{}", &hash.hex()[..32]))
}

/// Nome de exibição: último componente, sem caracteres de controle, ≤ 255 caracteres.
pub fn display_name_of(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let clean: String = raw.chars().filter(|c| !c.is_control()).take(255).collect();
    if clean.is_empty() {
        "untitled".to_owned()
    } else {
        clean
    }
}

fn stat(path: &Path) -> Result<std::fs::Metadata, AssetError> {
    std::fs::metadata(path).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            AssetErrorCode::AssetFileNotFound
        } else {
            AssetErrorCode::AssetIo
        };
        AssetError::new(code, format!("cannot access `{}`: {e}", path.display()))
    })
}

/// Valida o caminho de entrada e devolve a forma absoluta (sem `canonicalize`: ADR-048 §3).
pub(crate) fn absolute_checked(path: &Path) -> Result<PathBuf, AssetError> {
    let bad = |m: &str| AssetError::new(AssetErrorCode::AssetPathInvalid, m.to_owned());
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
    std::path::absolute(path).map_err(|e| bad(&format!("cannot resolve path: {e}")))
}

pub fn prepare_import(
    path: &Path,
    project_dir: Option<&Path>,
    probe: &dyn MediaProbe,
    now_ms: u64,
) -> Result<PreparedAsset, AssetError> {
    let abs = absolute_checked(path)?;
    let before = stat(&abs)?;
    if !before.is_file() {
        return Err(AssetError::new(
            AssetErrorCode::AssetNotRegularFile,
            format!("`{}` is not a regular file", abs.display()),
        ));
    }
    if before.len() == 0 {
        return Err(AssetError::new(
            AssetErrorCode::AssetEmptyFile,
            format!("`{}` is empty", abs.display()),
        ));
    }
    let digest = hash_file(&abs)?;
    let media = probe.probe(&abs)?;
    // o arquivo não pode ter mudado entre o hash e o probe (ADR-046): senão hash e metadados
    // descreveriam conteúdos diferentes
    let after = stat(&abs)?;
    if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
        return Err(AssetError::new(
            AssetErrorCode::AssetChangedDuringImport,
            format!("`{}` changed while it was being imported", abs.display()),
        ));
    }
    let record = AssetRecord {
        asset_id: asset_id_for(&digest.hash),
        kind: media.kind.into(),
        content_hash: digest.hash,
        size_bytes: digest.size,
        display_name: display_name_of(&abs),
        location: AssetLocation::from_path(&abs, project_dir),
        known_paths: Vec::new(),
        media,
        status: Availability::Online,
        status_checked_ms: now_ms,
        imported_ms: now_ms,
    };
    Ok(PreparedAsset { record, path: abs })
}
