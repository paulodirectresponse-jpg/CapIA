# Testes de aceitação de comportamento

`timeline/*.json` — cenários **do CapIA** (dados) para a camada de edição. São o **critério de aceitação da Fase 2** de `capia-commands`: o harness do engine carrega cada cenário, monta o `given`, executa `when` e confere `then` (incluindo atomicidade nos erros e exatidão em Ticks).

- Formato, regras e decisões em `docs/spikes/S7-timeline-acceptance.md`.
- Unidade de tempo nos cenários: **frames** (inteiros ou frações `"n/d"`); campo `fps` (padrão 30; `"30000/1001"` para NTSC). O harness converte para Ticks e exige exatidão.
- Cada cenário tem `provenance` (política em `docs/PROVENANCE.md`). Cenários **não** copiam testes de terceiros.
- O oráculo Python em `spikes/s7-oracle/oracle.py` é descartável: serviu para provar consistência dos cenários; **não** é o engine. Rodar: `python3 spikes/s7-oracle/oracle.py tests/acceptance/timeline`.
