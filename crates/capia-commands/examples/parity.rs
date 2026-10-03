//! Paridade nativo × WASM (ADR-016): executa sequências aleatórias determinísticas pelo engine e
//! imprime o digest SHA-256 do documento final de cada uma. O mesmo programa roda nativamente e em
//! `wasm32-wasip1`; `tools/check-wasm-parity.mjs` exige saídas **idênticas** (inclusive floats de
//! keyframes/Bézier e serialização JSON).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../tests/common/mod.rs"]
mod common;

fn main() {
    let cases: u64 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    for line in common::parity_report(cases) {
        println!("{line}");
    }
}
