//! Base de tempo do CapIA (ADR-007, docs/TIMELINE_ENGINE.md §1).
//!
//! Tempo é **inteiro**: [`Ticks`] (1/705.600.000 s), [`Rational`] normalizado e [`FrameRate`].
//! Nenhuma API pública usa ponto flutuante. Este crate não faz IO e compila para WASM (ADR-016).
//!
//! Arredondamento (D-S7-8): `floor` para "qual frame contém t"; **half-up** para alinhar bordas de
//! vídeo (com duração mínima de 1 frame). Áudio, mapeamento de fonte e tempo interno ficam em ticks.

mod frame_rate;
mod rational;
mod ticks;

pub use frame_rate::FrameRate;
pub use rational::Rational;
pub use ticks::{MAX_TIMELINE_TICKS, TICKS_PER_SECOND, Ticks, TimeError, TimeRange};
