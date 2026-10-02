//! Command Engine do CapIA (docs/COMMAND_SYSTEM.md). **Scaffold:** nenhum comando ainda.
//!
//! Comandos, transações, `operation_id` (ADR-029), `preview`/`apply_plan` (ADR-030), undo/redo e
//! histórico chegam na Fase 2, validados pela suíte `tests/acceptance` (ADR-036). Este crate
//! depende de `capia-model`, não faz IO e compila para WASM (ADR-016).

/// Versão do contrato de comandos (upcasters de comandos antigos, COMMAND_SYSTEM.md §10).
pub const COMMAND_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_schema_starts_at_one() {
        assert_eq!(COMMAND_SCHEMA_VERSION, 1);
    }

    #[test]
    fn commands_build_on_the_document_schema() {
        assert_eq!(capia_model::DOCUMENT_SCHEMA_VERSION, 1);
    }
}
