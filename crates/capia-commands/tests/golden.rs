//! Golden digest do engine: o estado final de 25 sequências aleatórias determinísticas é travado
//! por um digest. Qualquer mudança de **semântica** de comando/modelo/serialização muda o digest —
//! e precisa ser uma decisão revisada (atualize a constante junto com a justificativa no commit).
//! O mesmo relatório é comparado entre nativo e WASM por `tools/check-wasm-parity.mjs`.

mod common;

/// `cargo run -q -p capia-commands --example parity -- 25 | tail -1`
const GOLDEN_COMBINED_25: &str = "a5289259ee641e5a25c54bdece1a3255992b48499c45fa514e5c6e8ac144ce40";

#[test]
fn engine_state_digest_is_stable() {
    let report = common::parity_report(25);
    assert_eq!(
        report.last().map(String::as_str),
        Some(format!("combined {GOLDEN_COMBINED_25}").as_str()),
        "o comportamento do engine mudou; se for intencional, atualize GOLDEN_COMBINED_25"
    );
    assert_eq!(report.len(), 26);
}
