//! Núcleo do projeto do CapIA: fachada que os adaptadores (Tauri hoje; CLI/REST/MCP depois)
//! consomem. **Scaffold:** só descreve o engine. Persistência (.capia/SQLite), sessão de projeto e
//! Engine API chegam na Fase 2 (hoje documentadas como `capia-store` + `capia-engine`; ADR-038).
//!
//! Regra (ADR-002): este crate **não** conhece Tauri, UI, IA, providers, render nem FFmpeg.

use serde::Serialize;

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
