//! Autonomia (Fase 5, ADR-087..): AI Run persistente e retomável sobre o Engine API. A IA continua
//! **cliente**: escreve só por `preview → apply_plan` com o ator da Run; nenhum caminho de edição
//! depende dela; sem Run, o editor é idêntico.

pub mod critic;
pub mod failpoint;
pub mod gateway;
pub mod generation;
pub mod machine;
pub mod memory;
pub mod model;
pub mod orchestrator;
pub mod plan;
pub mod roles;
pub mod stages;
pub mod vision;
