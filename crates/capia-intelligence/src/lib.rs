//! `capia-intelligence` — pipelines de inteligência assistida da Fase 4 (ADR-082..085).
//!
//! A IA é **cliente do Engine API**: lê por métodos de leitura e escreve só por
//! `preview → apply_plan` com ator `Agent`. Nada aqui é necessário para editar: com todos os
//! providers desligados as tarefas locais (cenas, silêncio, análise de áudio) continuam
//! funcionando e o editor não muda.

#![forbid(unsafe_code)]

pub mod captions;
pub mod ctx;
pub mod demand;
pub mod docs;
pub mod engine;
pub mod error;
pub mod records;
pub mod reference;
pub mod scenes;
pub mod silence;
pub mod transcript;

pub use ctx::IntelCtx;
pub use engine::{Engine, SessionEngine};
pub use error::{IntelError, IntelResult};
