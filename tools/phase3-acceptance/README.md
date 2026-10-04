# Pacote de aceitação da Fase 3 (humano/hardware)

O que a engenharia **não consegue** provar sozinha, em um clique cada:

| Item | Como | Resultado |
|---|---|---|
| **Residual de GPU real (ADR-069)** | `powershell -ExecutionPolicy Bypass -File .\gpu-residual.ps1` em PC com GPU real | `PASS`/`FAIL` na tela + `gpu-residual-result.json` |
| **≥ 3 usuários reais (ROADMAP)** | `TASK.md` (anúncio UGC 30–45 s) com `make-sample.mjs`, `timer.html`, `ISSUE-FORM.md` | `node validate-results.mjs results/*.json` → `ATENDIDO`/`NÃO ATENDIDO`/`PENDENTE` |
| Benchmark local de UX da timeline (metas §6, GPU real) | `CAPIA_PERF_STRICT=1 pnpm --filter @capia/e2e e2e perf` (após `pnpm build` e `cargo build --release -p capia-devserver`) | `target/perf/phase3-ui-perf.json` com passa/falha por meta |

Regras: **nunca** preencha resultados que não aconteceram; o validador devolve `PENDENTE` sem arquivos
reais. O app tem **Configurações → Copiar diagnóstico** (versões e tempos; sem caminhos nem nomes de arquivo).

Arquivos: `TASK.md` · `CHECKLIST.md` · `ISSUE-FORM.md` · `results-template.json` · `timer.html` ·
`make-sample.mjs` · `validate-results.mjs` · `gpu-residual.ps1`.
