//! `EnginePort` (ADR-110): a fronteira entre a inteligência CapIA e o editor OpenCut.
//!
//! A inteligência já fala só com o trait [`capia_intelligence::Engine`]. Este crate implementa esse
//! trait sobre a **AgentSurface** do editor (domínio TypeScript do OpenCut) por JSON-RPC:
//!
//! ```text
//! Editor Agent (compile_edit_plan) ──comandos legados──► AgentSurfaceEngine
//!      └ traduz → Plan{schemaVersion,planId,actor,baseRevision,operations}
//!      └ Transport (stdio sidecar Bun | HTTP → página do editor)
//!          └ AgentSurface.preview_plan / apply_plan  (mesmos Commands da UI)
//! ```
//!
//! Escrita só por `preview → apply` (token do plano preso ao ator). Nada aqui conhece o modelo ou o
//! Command Engine Rust legado.

#![forbid(unsafe_code)]

mod engine;
mod error;
pub mod translate;
mod transport;
pub mod units;

pub use engine::AgentSurfaceEngine;
pub use error::PortError;
pub use transport::{HttpTransport, StdioTransport, Transport};
