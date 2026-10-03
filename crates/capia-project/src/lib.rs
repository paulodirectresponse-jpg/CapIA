//! Fachada do projeto do CapIA: o que os adaptadores (CLI hoje; Tauri/REST/MCP depois) consomem.
//! Junta o `Engine` (Command Engine + journal em `capia-store`), o catálogo de mídia e os assets
//! (`capia-assets`, probe via o trait de `capia-media`).
//!
//! Regra (ADR-002): este crate **não** conhece Tauri, UI, IA nem providers; só fala com o FFmpeg
//! através do trait `MediaProbe`/`MediaToolchain` de `capia-media`.

use serde::Serialize;

mod assets;
mod error;
mod project;

pub use assets::{
    AssetView, ImportOutcome, ImportResult, RelinkResult, VerifyResult, expected_asset_id,
};
pub use error::ProjectError;
pub use project::{ParseError, Project, parse_transaction};

/// Versão da Engine API exposta aos adaptadores.
pub const ENGINE_API_VERSION: u32 = 1;

/// Descrição do engine em execução. Contrato JSON compartilhado com `@capia/engine-bindings`
/// (fixture em `packages/engine-bindings/fixtures/engine_info.json`, testada dos dois lados).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub engine_api_version: u32,
    pub document_schema_version: u32,
    pub command_schema_version: u32,
    pub ticks_per_second: i64,
}

/// Informações do engine para os adaptadores.
pub fn engine_info() -> EngineInfo {
    EngineInfo {
        name: "capia-engine",
        version: env!("CARGO_PKG_VERSION"),
        engine_api_version: ENGINE_API_VERSION,
        document_schema_version: capia_model::DOCUMENT_SCHEMA_VERSION,
        command_schema_version: capia_commands::COMMAND_SCHEMA_VERSION,
        ticks_per_second: capia_time::TICKS_PER_SECOND,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn engine_info_reports_the_layers_it_is_built_on() {
        let info = engine_info();
        assert_eq!(
            info.document_schema_version,
            capia_model::DOCUMENT_SCHEMA_VERSION
        );
        assert_eq!(
            info.command_schema_version,
            capia_commands::COMMAND_SCHEMA_VERSION
        );
        assert_eq!(info.ticks_per_second, 705_600_000);
    }

    /// Contrato Rust <-> TypeScript: o mesmo arquivo é lido por `engine-bindings` nos testes TS.
    #[test]
    fn json_shape_matches_the_shared_contract_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../packages/engine-bindings/fixtures/engine_info.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(engine_info()).unwrap(), fixture);
    }
}
