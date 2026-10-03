//! Domínio de assets (ADR-046..049): identidade estável (id + hash de conteúdo + localização),
//! hash em *streaming*, import (hash → probe → registro), disponibilidade (online/offline/modified),
//! relink por conteúdo e cache derivado descartável. Faz IO de arquivos; **não** conhece SQLite,
//! Tauri nem o Command Engine. Fora do WASM.

mod cache;
mod error;
mod hash;
mod import;
mod location;
mod record;
mod verify;

pub use cache::{CacheDir, CacheKey, ensure_thumbnail};
pub use error::{AssetError, AssetErrorCode};
pub use hash::{ContentHash, FileDigest, hash_file, hash_reader};
pub use import::{PreparedAsset, asset_id_for, display_name_of, prepare_import};
pub use location::{AssetLocation, relative_path, resolve_candidates};
pub use record::{AssetKind, AssetRecord, Availability};
pub use verify::{RelinkCheck, VerifyReport, check_relink, quick_status, verify_content};

/// `MediaInfo` sintético para testes (sem ffprobe). Não use em produção.
#[doc(hidden)]
pub mod testing;
