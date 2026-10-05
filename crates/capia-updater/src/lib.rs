//! Atualização assinada do CapIA (Fase 6, Track C).
//!
//! * [`manifest`]: manifesto JSON assinado (Ed25519 sobre o JSON canônico), canais `stable`/`beta`.
//! * [`verify`]: `Verifier` + Ed25519 (`ed25519-dalek`); a chave pública de produção entra na build.
//! * [`version`]: semver com pré-release e política (sem downgrade silencioso; rollback só explícito).
//! * [`state`]: máquina de estados atômica persistida (`Idle → Downloaded → Verified → Staged → Switched →
//!   Confirmed`), recuperação após morte em qualquer ponto e rollback automático.
//!
//! Sem código de rede e sem trocar arquivos: o download (`Downloader`) e a troca (`Switcher`, instalador
//! NSIS silencioso) são injetados pelo host; os testes usam falsos com injeção de falhas.

mod error;
pub mod manifest;
pub mod state;
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
pub use verify::{
    Ed25519Signer, Ed25519Verifier, Signature, TrustedKey, Verifier, VerifyError, parse_key_list,
};
pub use version::{Decision, UpdateKind, Version, compare, evaluate};
