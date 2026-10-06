//! Atualização assinada do CapIA (Fase 6, Track C).
//!
//! * [`manifest`]: manifesto JSON assinado (Ed25519 sobre o JSON canônico), canais `stable`/`beta`.
//! * [`verify`]: `Verifier` + Ed25519 (`ed25519-dalek`); a chave pública de produção entra na build.
//! * [`version`]: semver com pré-release e política (sem downgrade silencioso; rollback só explícito).
//! * [`state`]: máquina de estados atômica persistida (`Idle → Downloaded → Verified → Staged → Switched →
//!   Confirmed`), recuperação após morte em qualquer ponto e rollback automático.
//!
//! * [`switcher`]: `InstallerSwitcher` — a troca real é do instalador NSIS silencioso (por usuário), com
//!   instalador anterior retido para o rollback; execução de processo sem shell, com timeout.
//!
//! Sem código de rede: o download (`Downloader`) é injetado pelo host; o `Switcher` real não sobrescreve
//! binários por conta própria. Os testes usam falsos com injeção de falhas e processos reais (Unix).

mod error;
pub mod manifest;
pub mod state;
pub mod switcher;
pub mod verify;
pub mod version;

pub use error::UpdateError;
pub use manifest::{
    Artifact, Channel, MANIFEST_SCHEMA, PRODUCT, UpdateManifest, canonical_json, signing_payload,
};
pub use state::{
    AlwaysAllow, Downloader, FileStateStore, HEALTH_FILE, HostPolicy, Outcome, RecoveryReport,
    RollbackInfo, STATE_FILE, StateStore, SwitchGate, Switcher, UpdateRecord, UpdateState, Updater,
    UpdaterConfig, sha256_file, write_atomic,
};
pub use switcher::{CommandRunner, InstallerSwitcher, ProcessRunner};
pub use verify::{
    Ed25519Signer, Ed25519Verifier, Signature, TrustedKey, Verifier, VerifyError, parse_key_list,
};
pub use version::{Decision, UpdateKind, Version, check_manifest, compare, evaluate};
