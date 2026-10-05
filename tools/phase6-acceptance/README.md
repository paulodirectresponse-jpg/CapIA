# Pacote de aceitação da Fase 6 (Integração e Finalização)

Mesma filosofia do pacote da Fase 5: tudo que é automático roda sem credenciais; o que depende de **humano, hardware, certificado ou jurídico** é marcado como **externo** e **nunca** é preenchido com resultado inventado. Cada passo termina em exatamente um destes estados — nunca “pulado” em silêncio:

| Estado | Significado |
|---|---|
| `passed` | rodou (ou a evidência real foi validada) e atendeu |
| `failed` | rodou e falhou, ou a evidência é malformada/registra falha |
| `pending_external` | depende de evidência humana/hardware/certificado que **não existe ainda** |
| `not_available` | não pôde rodar **aqui** (falta o binário/teste de outra frente, variável de ambiente, `CAPIA_P6_HEAVY`); o motivo é impresso |

Só `passed` é aprovação. O agregado de uma suíte é o **pior** estado dos passos.

| Pasta | O que cobre | Comando |
|---|---|---|
| [`external-flow/`](external-flow/README.md) | fluxo canônico REST/MCP/webhook, docs/exemplos em dia, paridade UI×REST×MCP | `node tools/phase6-acceptance/external-flow/run.mjs` |
| [`installer/`](installer/README.md) | versão única, documentos de release, build, artefato, **assinatura** (externa) | `node tools/phase6-acceptance/installer/run.mjs` |
| [`update/`](update/README.md) | atualização assinada RC→RC e rollback (externo) | `node tools/phase6-acceptance/update/run.mjs` |
| [`security/`](security/README.md) | catálogo/scopes, MAC, SSRF, regressões 4/5, suíte de API/MCP, pentest independente (externo) | `node tools/phase6-acceptance/security/run.mjs` |
| [`clean-machine/`](clean-machine/README.md) | Windows 10 22H2 e 11 limpos, sem ferramentas de dev (humano/hardware) | `run-clean-machine.ps1` + `validate.mjs` |
| [`beta-feedback/`](beta-feedback/README.md) | feedback real de beta e gate “sem Blocker/Critical aberto” (humano) | `validate.mjs` |
| `performance/` | **outra frente** (Track D); o agregador a executa se existir | — |

## Agregador

`node tools/phase6-acceptance/run-all.mjs [--strict]` roda tudo e grava `target/phase6-acceptance/summary.json`. Estado final:

| Estado | Quando |
|---|---|
| `FAILURES` | alguma suíte falhou |
| `INCOMPLETE — …` | alguma suíte está `not_available` (binários/testes de outras frentes ausentes): **não** se declara engenharia completa |
| `PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING` | tudo automático passou; falta evidência externa real |
| `PHASE 6 COMPLETE` | **todas** as suítes passaram, inclusive as evidências externas reais |

`--strict` sai com 1 a menos que seja `PHASE 6 COMPLETE`.

## Convenções

- **Passos declarativos** (`steps.json`): comando com requisitos (`bin`, `file`, `env`) ou passo `external` com `evidence` + `why` (+ `validate`). Variáveis: `${root}`, `${evidence}`, `${bin:NOME}`, `${env:NOME}`. Binários: `CAPIA_<NOME>_BIN` (sem `capia-`; ex.: `CAPIA_SERVER_BIN`), `target/release`, `target/debug`, `PATH`.
- **Passos pesados** (compilam Rust, builds do desktop) exigem `CAPIA_P6_HEAVY=1`. Sem isso ficam `not_available`.
- **Evidência externa**: arquivos em `target/phase6-acceptance/evidence/<nome>-results.json` (ou `--file`). Cada pasta tem `template.json` com `"template": true` — enquanto a marca existir, o validador responde `pending_external`. Remova-a só ao registrar resultados **reais**.
- **Atestados**: os validadores exigem atestados explícitos do executor (`results_are_real`, `no_simulation`…). Plausibilidade (datas não futuras, builds do Windows coerentes, hashes bem formados, igualdade recalculada) reduz erro, mas **não substitui** a confiança no executor humano.
- Saídas em `target/phase6-acceptance/` (`CAPIA_P6_OUT` redireciona; os testes usam diretório temporário).
- Os passos que apontam para `crates/capia-server/tests/*.rs` ou `capia-desktop` usam os nomes **esperados**; ajuste o `steps.json` aos nomes finais — enquanto o arquivo não existir o passo é `not_available`, nunca `passed`.

Testes do próprio pacote: `pnpm test:tools`.
