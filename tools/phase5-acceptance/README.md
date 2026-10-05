# Pacote de aceitação da Fase 5 (Autonomia)

Tudo que é automático roda **sem credenciais e sem rede** (provider Replay, catálogo Replay, geração Replay, FFmpeg local) e grava JSON legível por máquina em `target/phase5-acceptance/`. O que depende de humano ou de provider real está marcado como **externo** e nunca é preenchido com número inventado: sem resultados reais, o status é `pending_external`.

| Pacote | Comando (1 clique) | O que prova | Externo? |
|---|---|---|---|
| Autonomia | `node tools/phase5-acceptance/autonomy/run.mjs` | Run brief → timeline editável; cenários (perguntas, aprovações, replan limitado, gateway, geração, drift manual, cancelamento, REVIEW→CORRECT, pausa); variantes + undo seletivo; pipeline headless pelo serviço `ai.*`; propriedades (transições legais, orçamento ≤ teto, promoção de memória só com humano); schema 5 | não |
| Crash/retomada | `node tools/phase5-acceptance/crash-resume/run.mjs` | failpoints por fronteira de stage e de efeito; SIGKILL real + retomada em **outro** processo; sem edição/download/geração duplicados; stages `started → interrupted` | não |
| Segurança | `node tools/phase5-acceptance/security/run.mjs` | brief/modelo hostis, canário de segredo, aprovações e gasto, promoção de memória, Run aninhada, integridade preview/apply, AI Off, `SafeFetcher` (SSRF, redirect, tamanho, tipo, allow-list), `pnpm check:arch`, UI sem escrita direta | não |
| Gateway/geração | `node tools/phase5-acceptance/gateway/run.mjs` | licença desconhecida → aprovação, proveniência, orçamento, adapter desligado ≠ app quebrado, dedup por hash, retry/429, crash em ACQUIRE | não |
| Memória | `node tools/phase5-acceptance/memory/run.mjs` | proposta ≠ ativa, promoção só com aprovação, precedência, isolamento por cliente, rejeitado não ressuscita, exclusão sem conteúdo no log, UI inerte até o clique | não |
| Demandas reais | `node tools/phase5-acceptance/real-demands/validate.mjs [--file <resultados.json>]` | **valida** o pacote humano (formato em `real-demands/template.json`): ≥ 10 demandas distintas, nota inteira 1–5, `run_id` e `brief_file`, provider/modelo/data/avaliador, média ≥ 4,0 | **sim** — sem arquivo ⇒ `pending_external` |
| Agregador | `node tools/phase5-acceptance/run-all.mjs` | roda as 5 suítes automáticas + o validador; grava `summary.json` | herda o externo |

Resultado do agregador: `PHASE 5 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING` (suítes automáticas passam, demandas reais pendentes), `PHASE 5 ACCEPTED` (só com resultados humanos reais válidos) ou `FAILURES`.

## Demandas reais (humano)

1. Gerar ≥ 10 Runs com um provider real (chave informada pela UI, cofre do SO) a partir de briefs reais; guardar o `run_id` e o arquivo do brief de cada uma.
2. Um humano avalia cada resultado de 1 a 5 (≥ "utilizável com ajustes leves" na média, ≥ 4,0) e preenche `target/phase5-acceptance/real-demands-results.json` no formato de `real-demands/template.json`.
3. `node tools/phase5-acceptance/real-demands/validate.mjs` → `real-demands-summary.json`. O validador não gera nem corrige notas.

## Estado das pendências externas
Ver `docs/STATUS.md` (seção Fase 5): avaliação humana de ≥ 10 demandas reais, execução com providers/chaves reais e checagens humanas/hardware herdadas **não foram executadas** neste ambiente.
