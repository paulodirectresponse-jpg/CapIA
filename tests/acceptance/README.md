# Testes de aceitação de comportamento

`timeline/*.json` — cenários **do CapIA** (dados) para a camada de edição. São o **critério de aceitação da Fase 2** de `capia-commands`: o harness do engine carrega cada cenário, monta o `given`, executa `when` e confere `then` (incluindo atomicidade nos erros e exatidão em Ticks).

- **Harness executável:** `crates/capia-commands/tests/acceptance.rs` (`cargo test -p capia-commands --test acceptance`). Monta o estado inicial por ops primitivas, roda `when` pelo **Command Engine real** e confere `then`; em erro exige documento idêntico.
- **120 cenários** = 108 da M03 + 12 da M05 que cobrem as decisões D-S7 definitivas (ADR-039): RTM-018/019, RPL-021..025, SNP-017..019, KF-022/023; RTM-006 e SNP-014..016 foram atualizados.
- Formato, regras e decisões em `docs/spikes/S7-timeline-acceptance.md` e `docs/DECISIONS.md` (ADR-036, ADR-039).
- Unidade de tempo nos cenários: **frames** (inteiros ou frações `"n/d"`); campo `fps` (padrão 30; `"30000/1001"` para NTSC). O harness converte para Ticks e exige exatidão. Campos extras de track: `sync_lock`, `group` (ripple com escopo).
- Cada cenário tem `provenance` (política em `docs/PROVENANCE.md`). Cenários **não** copiam testes de terceiros.
- O oráculo Python em `spikes/s7-oracle/oracle.py` está **congelado** (descartável): provou a consistência dos 108 cenários originais na M03 e **não** acompanha as decisões da M05. A fonte de verdade é o engine Rust.
