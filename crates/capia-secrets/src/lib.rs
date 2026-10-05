//! `capia-secrets` — a única fronteira por onde um segredo (chave de API) existe em claro.
//!
//! * [`SecretString`]: `zeroize` ao cair; `Debug`/`Display` sempre redigidos; sem `Serialize`.
//! * [`SecretStore`]: guarda/lê/apaga por [`CredentialRef`]. Windows ⇒ Credential Manager (DPAPI);
//!   [`MemoryStore`] para testes/dev (volátil, nunca persiste).
//! * [`SecretRegistry`] + [`redact`]: toda string que sai do core (log, erro, auditoria, IPC,
//!   diagnóstico) passa por aqui; valores registrados e padrões conhecidos de chave somem.
//!
//! O valor secreto nunca atravessa para a WebView: a UI só vê `configured: bool`.

#![forbid(unsafe_code)]

mod panic;
mod redact;
mod store;
mod string;

pub use panic::{install_panic_hook_with, install_redacting_panic_hook};
pub use redact::{
    SecretRegistry, redact, redact_global, redact_registered_global, register_global,
};
pub use store::{CredentialRef, MemoryStore, SecretStore, StoreError, platform_store};
pub use string::SecretString;
