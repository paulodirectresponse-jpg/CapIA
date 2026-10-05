//! Fixtures e utilitários de medição da Fase 6 (Track D-1). **Crate só de desenvolvimento**: não faz
//! parte do produto e nenhum crate de produto pode depender dele fora de `[dev-dependencies]`
//! (verificado por `tools/check-architecture.mjs`).
//!
//! * [`build_large_project`] produz um `.capia` REAL (≥ 30 sequences, ≥ 5.000 clips, nested,
//!   legendas, áudio, keyframes, catálogo grande e histórico de AI Runs) **somente** pelo Command
//!   Engine / Project API / stores públicos — nada de SQL cru, nada que o produto não faça.
//! * [`bench`] — estatística (p50/p95/máx), informação da máquina e relatório JSON.
//! * [`proc`] — RSS / descritores / threads do processo (Linux; `None` nas demais plataformas).
//! * [`query`] — consulta pura de intervalo da timeline (o que a UI virtualizada faz).
//! * [`api`] — sonda da Engine API (`Session::call`) com medição.

pub mod api;
pub mod bench;
mod builder;
pub mod proc;
pub mod query;
mod rng;
mod spec;

pub use builder::{build_large_project, offline_toolchain};
pub use spec::{FixtureError, LargeSpec, LargeStats};

/// Um frame a 30 fps em `Ticks` (705.600.000 / 30).
pub const FRAME: i64 = 23_520_000;
