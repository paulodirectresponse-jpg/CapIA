//! `capia-ai` — provider abstraction, Model Registry, Brain Profile, Capability Router e tool system
//! da Fase 4 (ADR-079..082).
//!
//! Regras de fronteira (verificadas por `tools/check-architecture.mjs` e testes de arquitetura):
//! * este crate **não conhece** o Command Engine, o projeto, a mídia nem o SQLite — só fala com
//!   providers e valida contratos;
//! * a única rede é a de [`http`] (cliente endurecido: SSRF, vínculo credencial↔host, limites);
//! * segredos só existem como [`capia_secrets::SecretString`], lidos no instante de montar o header.

#![forbid(unsafe_code)]

pub mod brain;
pub mod cancel;
pub mod capability;
pub mod dispatcher;
pub mod error;
pub mod fetch;
pub mod http;
pub mod probe;
pub mod prompt;
pub mod providers;
pub mod registry;
pub mod router;
pub mod stt;
pub mod testimg;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod tools;
pub mod types;
pub mod usage;
pub mod webhook;

pub use cancel::CancelToken;
pub use error::{ErrorCode, ProviderError};
