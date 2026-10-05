# API local do CapIA (REST `/v1`, MCP, webhooks)

O `capia-server` (Fase 6, ADR-102) é o host headless da **mesma Engine API** que a UI usa: REST, MCP e webhooks são adaptadores sobre os mesmos serviços, com os mesmos scopes, `operation_id`s, revisões e travas. Não existe segunda lógica de edição: o cliente externo escreve na timeline **somente** por `preview → apply_plan` (ator `Api`) e por AI Runs (ator `run:<id>`), com undo.

> **Estado honesto (0.6.0-rc.1).** O **contrato** (catálogo de 56 operações, scopes, erros, esquema do banco do servidor) está no código e é a fonte desta documentação. A **implementação** dos transportes (REST, MCP, entrega de webhooks) é uma frente da Fase 6 ainda em integração: onde um detalhe depende dela e não está no catálogo/no banco, o texto diz **“interface prevista”**. Nada aqui foi medido contra um servidor em produção; veja `docs/phase6/IMPL_DOCS_ACCEPTANCE.md`.

| Documento | Conteúdo |
|---|---|
| [auth-and-scopes.md](auth-and-scopes.md) | tokens, rotação, matriz de scopes (gerada) |
| [rest-reference.md](rest-reference.md) | uma seção por operação (gerada do catálogo) |
| [openapi.json](openapi.json) | esqueleto OpenAPI 3.1 (gerado) |
| [mcp.md](mcp.md), [mcp-tools.md](mcp-tools.md) | tools (gerado), resources, auth, falhas, paridade |
| [webhooks.md](webhooks.md) | eventos, assinatura, replay, retries |
| [security.md](security.md) | modelo de ameaças resumido |
| [canonical-flow.md](canonical-flow.md) | cópia + bruto + referência → export → webhook, em 12 passos |
| [`../../examples/`](../../examples/) | curl, PowerShell, Node, Python, MCP e receptores de webhook |

## 1. Convenções

- **Base:** `http://127.0.0.1:<porta>/v1`. O servidor escuta **só em loopback** por padrão (ver [../user/09-api-local.md](../user/09-api-local.md)). Bind remoto é opt-in explícito, exige TLS (proxy reverso) e token; tokens nunca devem trafegar em texto claro fora de loopback.
- **Formato:** JSON UTF-8 (`Content-Type: application/json`) em requisições com corpo e em todas as respostas. Corpos têm tamanho e profundidade limitados; JSON malformado → `400 BAD_JSON`; campo desconhecido → `422 INVALID_PARAMS` (os schemas têm `additionalProperties: false`).
- **Autenticação:** `Authorization: Bearer <token>` em tudo, exceto `GET /v1/health`.
- **`request_id`:** cada resposta, de sucesso ou erro, identifica a requisição (cabeçalho e, em erros, o campo `request_id` do corpo). Cite-o ao pedir suporte: ele aparece na auditoria (`audit.list`).
- **Tempo:** timestamps de eventos em ISO-8601 UTC **e** em milissegundos Unix (`occurred_at`, `occurred_ms`). Dentro do documento de timeline o tempo é inteiro (`Ticks`); a API nunca expõe segundos em ponto flutuante como fonte de verdade.
- **CORS (requisito de projeto):** negado por padrão (a API é para processos locais, não para páginas web); origens explícitas só por configuração.
- **Host header (requisito de projeto):** requisições com `Host` inesperado devem ser recusadas (defesa contra DNS rebinding).

### Envelope de erro

Todo erro tem o mesmo formato:

```json
{
  "code": "REVISION_CONFLICT",
  "message": "o projeto mudou desde a prévia",
  "details": { "expected_revision": 12, "current_revision": 14 },
  "request_id": "req_…"
}
```

`code` é estável e legível por máquina (`message` é para humanos e pode mudar; passa pelo redator central e **nunca** contém segredo). `details` é opcional. Códigos do Engine API/IA são mapeados para HTTP assim (`crates/capia-server/src/error.rs`):

| HTTP | Códigos |
|---|---|
| 400 | `BAD_JSON`, `BAD_REQUEST`, `INVALID_PARAMS` (erros de parâmetro do engine) |
| 401 | `UNAUTHORIZED` |
| 403 | `PERMISSION_DENIED` (scope ou permissão de tool) |
| 404 | `NOT_FOUND`, `RUN_NOT_FOUND`, `ASSET_NOT_FOUND`, `SEQUENCE_NOT_FOUND`, `UNKNOWN_METHOD` |
| 409 | `NO_PROJECT_OPEN`, `PROJECT_NOT_OPEN`, `STALE_PLAN`, `PLAN_STALE`, `PLAN_STATE_CHANGED`, `PLAN_EXPIRED`, `PLAN_DRIFT`, `REVISION_CONFLICT`, `CONFLICT`, `OVERLAP`, `ILLEGAL_TRANSITION`, `RUN_BUSY`, `TOO_MANY_RUNS`, `INVALID_STATE`, `AI_OFF`, `AI_DISABLED`, `NOT_CONFIGURED` |
| 422 | `INVALID_PARAMS` (validação do servidor) e qualquer código do engine sem mapeamento específico (“entendi, mas recuso”) |
| 429 | rate limit por token e classe (`Retry-After` em segundos) |
| 500 | `INTERNAL`, `POISONED` |
| 503 | serviço indisponível (`Retry-After`) |

Nota: `INVALID_PARAMS` aparece em 400 e 422 conforme a origem; trate ambos como “corrija o pedido”. Clientes devem decidir por `code`, e por classe de status só como fallback.

### Versionamento e depreciação

- Toda rota pública vive sob `/v1`. Dentro de `v1` só há **mudanças aditivas**: novos campos opcionais em requisições, novos campos em respostas, novas rotas, novos `code`, novos tipos de evento. **Clientes devem ignorar campos e eventos que não conhecem.**
- Mudança incompatível (remover/renomear campo, mudar semântica, tornar algo obrigatório) só em uma nova versão (`/v2`) servida **em paralelo** a `/v1`.
- Depreciação: a operação continua funcionando e passa a responder com o cabeçalho `Deprecation` (e `Sunset` quando houver data); o aviso entra no `CHANGELOG.md`. Mínimo de **uma versão menor** de convivência durante o RC/beta; a janela definitiva será fixada na 1.0.
- A versão do produto (`0.6.0-rc.1`) é única e aparece em `server.info`, no about do app e no diagnóstico. A versão da API é o prefixo `/v1`; o esquema de eventos de webhook tem seu próprio `version` (hoje `1`).

## 2. Operações longas: 202, status, eventos, webhook

Operações que podem demorar (importar asset, iniciar/retomar Run, gerar variantes, exportar, entregar teste de webhook) **nunca seguram a requisição HTTP**. O padrão:

1. `POST …` → **`202 Accepted`** com o id da operação (ticket de importação, `run`, `export`).
2. **Status por polling:** `GET …/imports/{ticket_id}`, `GET …/runs/{run_id}`, `GET …/exports/{export_id}`.
3. **Eventos por polling incremental:** `GET …/runs/{run_id}/events?after=<seq>` e `GET /v1/events?after=<seq>&limit=` (cursor monotônico `seq`; guarde o último visto).
4. **Webhook** na conclusão (`run.completed`, `export.completed`, …), sem nunca bloquear a Run se o seu endpoint estiver fora ([webhooks.md](webhooks.md)).
5. **SSE** (`text/event-stream`) para clientes locais é **interface prevista**: não existe rota SSE no catálogo atual (o catálogo só tem polling por `after`). Quando existir, será aditivo.

Estados terminais de uma Run: `completed`, `failed`, `cancelled`. `waiting_user` significa que há uma decisão pendente (`run.pending`) — resolva com `runs.approve` ([../user/05-aprovacoes-e-orcamentos.md](../user/05-aprovacoes-e-orcamentos.md)); `paused` exige `runs.resume` (o servidor **nunca** retoma sozinho uma Run interrompida).

## 3. Paginação

Listagens (`projects.list`, `assets.list`, `uploads.list`, `history.list`, `exports.list`, `deliverables.list`, `webhooks.deliveries`, `events.list`, `timeline.query`, `audit.list`) aceitam `after` (cursor opaco devolvido na página anterior; `audit.list` usa o número de sequência) e `limit` (1–200; `audit.list` até 500). A resposta traz `next` quando há mais itens; **ausência de `next` = fim**. Não construa cursores: use o valor devolvido. `runs.list` aceita só `limit` (as Runs mais recentes).

## 4. Concorrência otimista

O engine tem uma única autoridade e cada documento tem uma **revisão** inteira. Escritas são condicionais:

- **Edição estruturada:** `commands.preview` aceita `expected_revision` e devolve um **plan token** com o resultado previsto; `commands.apply` recebe o `plan_token`. Se o projeto mudou entre a prévia e a aplicação (outro cliente, a UI, uma Run), a aplicação falha com **409** (`STALE_PLAN`/`PLAN_DRIFT`/`REVISION_CONFLICT`) e **nada** é aplicado: refaça a prévia sobre o estado atual e decida de novo.
- **Decisões de Run:** `runs.approve` está **presa ao digest** do plano/estado que originou a decisão; se o plano mudou, a decisão é velha e recebe 409 (`PLAN_STATE_CHANGED`).
- O banco do servidor registra `revision_before`/`revision_after` por chamada na auditoria.

## 5. Idempotência (`Idempotency-Key`)

Toda operação **mutante** aceita `Idempotency-Key: <1–128 caracteres>` (use um UUID por **intenção**, e a **mesma** chave ao repetir após timeout, queda de rede ou resposta perdida). A chave vale **por token** e é associada à operação e a um hash do pedido. Estados (`IdemBegin`, `crates/capia-store/src/serverdb.rs`):

| Situação | Resultado |
|---|---|
| Chave nova | executa; guarda o status e o corpo da resposta |
| Mesma chave, **mesmo** pedido, já concluído | **replay**: devolve o mesmo status e corpo guardados, sem executar de novo (o efeito acontece uma vez) |
| Mesma chave, pedido **diferente** (outra operação ou outro corpo) | recusado (conflito de chave; **não** executa) |
| Outra requisição com a mesma chave ainda em andamento | recusado como “em andamento”: espere e repita com a mesma chave |
| Chave ficou “em andamento” de um processo que caiu (efeito **indeterminado**) | recusado como indeterminado: **consulte o estado** (ex.: `runs.list`, `assets.list`, `history.list`) antes de decidir; o servidor não adivinha se o efeito ocorreu |
| Falha transitória **antes** de qualquer efeito | a chave é liberada e a repetição executa normalmente |

Os `code`/HTTP exatos desses quatro casos de recusa são definidos pela implementação do servidor (interface prevista; esperado: 409 ou 422 com `code` próprio). Independentemente deles: **replay devolve o resultado original; qualquer outro caso nunca executa o efeito duas vezes.** Além disso, os `operation_id`s do engine (derivados por tarefa+passo+índice para a IA) tornam comandos repetidos idempotentes na camada de baixo.

## 6. Limites e rate limits

Cada operação tem uma **classe** (`read`, `write`, `upload`, `run_start`, `approve`, `export`, `admin`) e o limite é por token e classe. Excedido → `429` estruturado com `Retry-After`. Há ainda tetos de concorrência (`TOO_MANY_RUNS`) e de tamanho de corpo/upload. Os valores numéricos estão em `server.info` (campo de limites; interface prevista) e podem mudar entre versões.

## 7. Regenerar esta documentação

`node tools/docs/gen-api-docs.mjs` (ou `pnpm docs:api`) regenera `rest-reference.md`, `mcp-tools.md`, `openapi.json` e o bloco de scopes de `auth-and-scopes.md` a partir do catálogo; `--check` falha se estiverem defasados. Entrada: `--catalog <arquivo|->` com o JSON do catálogo (hoje o fixture `tools/docs/fixtures/catalog.json`; depois, a saída de `capia-server catalog`).
