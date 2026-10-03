//! Cache derivado (ADR-049): chave determinística pelo **conteúdo**, diretório separado do
//! projeto, escrita atômica, **descartável** — nada essencial vive só aqui.

use crate::error::{AssetError, AssetErrorCode};
use crate::hash::{ContentHash, hash_file};
use crate::record::{AssetKind, AssetRecord};
use capia_media::{MediaErrorCode, MediaToolchain, ThumbnailRequest, extract_frame_png};
use capia_time::Ticks;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    op: String,
    hex: String,
}

fn bad(msg: &str) -> AssetError {
    AssetError::new(AssetErrorCode::AssetPathInvalid, msg.to_owned())
}

fn field(h: &mut Sha256, s: &str) {
    // prefixado por tamanho: "ab"+"c" ≠ "a"+"bc"
    h.update((s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}

impl CacheKey {
    /// `op` e `ext` viram nomes de diretório/arquivo: só `[a-z0-9_-]`.
    pub fn new(
        content: &ContentHash,
        op: &str,
        params: &[(&str, &str)],
        producer: &str,
    ) -> Result<Self, AssetError> {
        if op.is_empty()
            || op.len() > 32
            || !op
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(bad("invalid cache operation name"));
        }
        let mut sorted: Vec<_> = params.to_vec();
        sorted.sort_unstable();
        let mut h = Sha256::new();
        field(&mut h, "capia-cache-v1");
        field(&mut h, content.as_str());
        field(&mut h, op);
        field(&mut h, producer);
        for (k, v) in sorted {
            field(&mut h, k);
            field(&mut h, v);
        }
        let digest = h.finalize();
        let hex = digest.iter().fold(String::new(), |mut s, b| {
            use core::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        });
        Ok(Self {
            op: op.to_owned(),
            hex,
        })
    }

    pub fn hex(&self) -> &str {
        &self.hex
    }

    pub fn op(&self) -> &str {
        &self.op
    }
}

/// Diretório de cache. Apagá-lo inteiro é sempre seguro.
#[derive(Clone, Debug)]
pub struct CacheDir {
    root: PathBuf,
}

static TMP: AtomicU64 = AtomicU64::new(0);

impl CacheDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Padrão: `<projeto>.capia-cache/` ao lado do arquivo do projeto (ADR-049 §3).
    pub fn beside_project(project: &Path) -> Self {
        let mut name = project.file_name().unwrap_or_default().to_os_string();
        name.push("-cache");
        Self::new(project.with_file_name(name))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, key: &CacheKey, ext: &str) -> Result<PathBuf, AssetError> {
        if ext.is_empty() || ext.len() > 8 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(bad("invalid cache extension"));
        }
        Ok(self
            .root
            .join(key.op())
            .join(&key.hex[..2])
            .join(format!("{}.{ext}", key.hex)))
    }

    /// Entrada existente (arquivo regular) ou `None`.
    pub fn get(&self, key: &CacheKey, ext: &str) -> Option<PathBuf> {
        let p = self.path_for(key, ext).ok()?;
        std::fs::metadata(&p).ok()?.is_file().then_some(p)
    }

    /// Escrita atômica: arquivo temporário no mesmo diretório + `rename`.
    pub fn put(&self, key: &CacheKey, ext: &str, bytes: &[u8]) -> Result<PathBuf, AssetError> {
        let final_path = self.path_for(key, ext)?;
        let io = |e: std::io::Error| {
            AssetError::new(AssetErrorCode::AssetIo, format!("cache write failed: {e}"))
        };
        let dir = final_path
            .parent()
            .ok_or_else(|| bad("invalid cache path"))?;
        std::fs::create_dir_all(dir).map_err(io)?;
        let tmp = dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TMP.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::write(&tmp, bytes).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(e)
        })?;
        std::fs::rename(&tmp, &final_path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(e)
        })?;
        Ok(final_path)
    }

    /// Apaga todo o cache (idempotente).
    pub fn clear(&self) -> Result<(), AssetError> {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AssetError::new(
                AssetErrorCode::AssetIo,
                format!("cannot clear the cache: {e}"),
            )),
        }
    }
}

/// Miniatura de um frame (ADR-049 §4), como entrada do cache chaveada por conteúdo. Em cache
/// ausente confere o hash do arquivo **antes** de gerar: nunca guarda, sob o hash vigente, o
/// quadro de um arquivo diferente.
pub fn ensure_thumbnail(
    toolchain: &MediaToolchain,
    record: &AssetRecord,
    file: &Path,
    at: Ticks,
    max_dim: u32,
    cache: &CacheDir,
) -> Result<PathBuf, AssetError> {
    if record.kind == AssetKind::Audio {
        return Err(AssetError::new(
            AssetErrorCode::Media(MediaErrorCode::MediaUnsupportedFormat),
            "audio assets have no video frame to thumbnail",
        ));
    }
    // imagens não têm linha do tempo: sempre o quadro 0
    let at = if record.kind == AssetKind::Image {
        Ticks::ZERO
    } else {
        at
    };
    let producer = format!("thumb/1;{}", toolchain.version);
    let key = CacheKey::new(
        &record.content_hash,
        "thumbnail",
        &[("at", &at.0.to_string()), ("max", &max_dim.to_string())],
        &producer,
    )?;
    if let Some(p) = cache.get(&key, "png") {
        return Ok(p);
    }
    let digest = hash_file(file)?;
    if digest.hash != record.content_hash {
        return Err(AssetError::new(
            AssetErrorCode::AssetHashMismatch,
            "the file no longer has the content of this asset; no thumbnail generated",
        ));
    }
    let png = extract_frame_png(toolchain, file, ThumbnailRequest { at, max_dim })?;
    cache.put(&key, "png", &png)
}
