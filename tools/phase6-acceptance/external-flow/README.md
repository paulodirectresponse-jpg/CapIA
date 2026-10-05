# external-flow

Aceitação do fluxo externo: **copy + bruto + referência → Run → aprovações → variantes → export → webhook**, por REST e MCP, com o mesmo resultado da UI.

`node tools/phase6-acceptance/external-flow/run.mjs [--list] [--only id] [--strict]` — passos em [`steps.json`](steps.json):

| Passo | Tipo | Estado típico hoje |
|---|---|---|
| `catalog-docs-in-sync` | comando (`gen-api-docs.mjs --check`) | `passed` |
| `examples-unit-tests`, `webhook-receiver-verification` | comando | `passed` |
| `server-binary` | comando; precisa de `capia-server` | `not_available` até compilar (`CAPIA_SERVER_BIN` ou `target/`) |
| `server-conformance-rest-mcp` | cargo test; precisa do arquivo de teste da frente do servidor + `CAPIA_P6_HEAVY=1` | `not_available` |
| `live-canonical-flow-rest` | roda `examples/rest/client.mjs` contra um servidor **em execução**; exige `CAPIA_PORT`, `CAPIA_TOKEN`, `CAPIA_FLOW_RAW`, `CAPIA_FLOW_REF`, `CAPIA_FLOW_BRIEF` (+ opcional `CAPIA_FLOW_APPROVE`) | `not_available` |
| `live-canonical-flow-mcp-smoke` | `examples/mcp/stdio-client.mjs` (initialize/tools/list/server_info) | `not_available` |
| `ui-rest-mcp-parity` | **externo**: `validate-parity.mjs` | `pending_external` |

Como obter um servidor para os passos “live” (sem fabricar nada): `capia-server token create --data-dir D --name e2e --scopes …`, `capia-server serve --data-dir D`, configure um provider de IA real (ou o cérebro Replay do ambiente de teste) e exporte as variáveis. **Sem um provider configurado a Run responde `AI_OFF`/`NOT_CONFIGURED`** — isso é falha legítima, não passa.

## Evidência de paridade (humano + servidor real)

1. Execute a **mesma tarefa** (mesmo briefing, bruto, referência, política e orçamento) pela UI, por REST e por MCP, em projetos separados e idênticos na origem.
2. Para cada superfície, registre o **estado**: revisão do projeto, nº de sequences, clips, variantes, estado da Run e sondagem do export (codec, largura, altura, duração em Ticks).
3. Registre se o webhook de conclusão chegou e teve a assinatura verificada, e se REST e MCP usaram os mesmos scopes.
4. Preencha `target/phase6-acceptance/evidence/external-flow-parity-results.json` no formato de [`template-parity.json`](template-parity.json) (remova `"template": true`).
5. `node tools/phase6-acceptance/external-flow/validate-parity.mjs` — o validador **recalcula** a igualdade campo a campo; não confia em `equal: true`.

Limitação honesta: LLMs reais não são determinísticos; a paridade só é testável com o provider Replay (determinístico) ou comparando estruturalmente execuções com a mesma saída de plano. Anote no arquivo qual foi usado (`task.description`).
