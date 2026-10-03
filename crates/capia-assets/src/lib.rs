//! Domínio de assets (ADR-046..049): identidade estável (id + hash de conteúdo + localização),
//! hash em *streaming*, import (hash → probe → registro), disponibilidade (online/offline/modified),
//! relink por conteúdo e cache derivado descartável. Faz IO de arquivos; **não** conhece SQLite,
//! Tauri nem o Command Engine. Fora do WASM.

mod cache;
mod error;
mod fingerprint;
mod hash;
mod import;
mod location;
mod record;
mod scan;
mod verify;

pub use cache::{CacheDir, CacheKey, CacheUsage, GcReport, KeyLock, Produced, ensure_thumbnail};
pub use error::{AssetError, AssetErrorCode};
pub use fingerprint::{
    FINGERPRINT_VERSION, Fingerprint, INNER_SAMPLES, SAMPLE_LEN, fingerprint_file, sample_offsets,
};
pub use hash::{
    ContentHash, FileDigest, FileStamp, file_stamp, hash_file, hash_file_job, hash_reader,
};
pub use import::{
    PreparedAsset, absolute_checked, asset_id_for, display_name_of, prepare_import,
    prepare_import_job,
};
pub use location::{AssetLocation, relative_path, resolve_candidates};
pub use record::{AssetKind, AssetRecord, Availability};
pub use scan::{
    AmbiguousAsset, BatchRelinkReport, MatchError, MatchedAsset, RejectedCandidate, RelinkTarget,
    ScanIssue, ScanOptions, ScanReport, ScannedFile, match_candidates, scan_folder,
};
pub use verify::{RelinkCheck, VerifyReport, check_relink, quick_status, verify_content};

/// `MediaInfo` sintético para testes (sem ffprobe). Não use em produção.
#[doc(hidden)]
pub mod testing;
