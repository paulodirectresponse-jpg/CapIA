//! `capia-intelligence` — a camada de inteligência assistida da Fase 4. É **cliente** do engine
//! (por fachada) e dos providers (`capia-ai`); nada aqui escreve na timeline sem
//! `preview → apply_plan` (ADR-029/030).

#![forbid(unsafe_code)]

pub mod scenes;
