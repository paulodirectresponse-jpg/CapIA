//! Modelo do documento do CapIA (docs/DATA_MODEL.md). **Scaffold:** nenhuma entidade ainda.
//!
//! Entidades (Project, Sequence, Track, Clip, …), IDs e invariantes chegam na Fase 2. Este crate
//! depende só de `capia-time`, não faz IO e compila para WASM (ADR-016).

pub use capia_time::Ticks;

/// Versão do schema do documento persistido. Migrations só para frente (DATA_MODEL.md §6).
pub const DOCUMENT_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_starts_at_one() {
        assert_eq!(DOCUMENT_SCHEMA_VERSION, 1);
    }

    #[test]
    fn model_reexports_the_time_unit() {
        assert_eq!(Ticks::default(), capia_time::Ticks(0));
    }
}
