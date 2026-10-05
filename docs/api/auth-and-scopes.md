# Autenticação e scopes

Toda rota do catálogo, **exceto `server.health`**, exige um token no cabeçalho `Authorization: Bearer <token>`. O mesmo token e os mesmos scopes valem para REST e MCP: não há “modo admin implícito” em nenhuma superfície.

## Tokens

| Aspecto | Comportamento |
|---|---|
| Criação | `POST /v1/tokens` (`tokens.create`, exige `admin:tokens`) com `name` (1–80 caracteres), `scopes` (≥ 1, sem duplicata, sem scope desconhecido) e `expires_in_seconds` opcional (60 s a 1 ano) |
| Segredo | devolvido **uma única vez**, na resposta da criação (ou da rotação). Depois disso nenhuma rota o devolve de novo — `tokens.list` mostra só metadados |
| Armazenamento | o servidor guarda apenas o **SHA-256** do segredo (64 hex, índice único) e um prefixo curto para identificação, mais `created`, `expires`, `last_used` e `revoked`. Nunca o segredo (`crates/capia-store/src/serverdb.rs`, tabela `api_tokens`) |
| Formato | opaco; nos exemplos desta documentação usamos o placeholder `capia_REDACTED`. **Nunca** versione um token real |
| Rotação | `POST /v1/tokens/{token_id}/rotate`: devolve um novo segredo (uma vez) e revoga o antigo (`rotated_from` fica registrado) |
| Revogação | `DELETE /v1/tokens/{token_id}`; efeito imediato nas próximas requisições |
| Expiração | opcional; token expirado é tratado como inválido (401) |
| Auditoria | `GET /v1/audit` (`audit.list`, `admin:tokens`): superfície (REST/MCP), operação, resultado, revisão antes/depois, `Idempotency-Key` e `request_id`; nunca o segredo |

O primeiro token (bootstrap) é emitido localmente pelo próprio executável do servidor — veja [../user/09-api-local.md](../user/09-api-local.md) (interface prevista: o comando exato é definido pela implementação do servidor). A partir daí, quem tem `admin:tokens` cria os demais.

### Boas práticas

- Um token por cliente/integração, com o **menor conjunto de scopes** que funciona (ex.: um painel de leitura só com `project:read` + `run:read`).
- Tokens de longa duração só em cofre de segredos (variável de ambiente do processo, cofre do SO, secret store do CI). Não em Git, logs nem tickets.
- Rotacione quando alguém sai do projeto e a cada mudança de máquina. Se suspeitar de vazamento, **revogue primeiro**, investigue depois (a auditoria mostra o que o token fez).
- Dê `admin:tokens` apenas a quem precisa gerenciar tokens. Esse scope também lê a auditoria.

## Erros de autenticação e autorização

| Situação | HTTP | `code` |
|---|---|---|
| Sem cabeçalho / token inválido, expirado ou revogado | 401 | `UNAUTHORIZED` |
| Token válido, mas sem o scope da operação | 403 | `PERMISSION_DENIED` (ver nota) |
| Rate limit da classe da operação excedido | 429 | com `Retry-After` |

Nota: o conjunto exato de `code` para 403/429 é definido pela implementação do servidor; o envelope é sempre `{code, message, details?, request_id}` (ver [README.md](README.md)). Mensagens de erro passam pelo redator central: nenhum segredo aparece nelas.

## Matriz de scopes

Gerada do catálogo (`node tools/docs/gen-api-docs.mjs`). O catálogo é a **única** fonte: REST, MCP e OpenAPI derivam dele, e um teste do servidor garante que nenhuma operação fica sem scope (exceto o health público).

<!-- BEGIN GENERATED:scope-matrix (tools/docs/gen-api-docs.mjs) -->

| Scope | Operações (REST/MCP) | Mutantes |
|---|---|---|
| `project:read` | `server.info`, `projects.list`, `projects.get`, `projects.summary`, `sequences.list`, `sequences.get`, `timeline.query`, `history.list`, `events.list` | 0/9 |
| `project:write` | `projects.create`, `projects.open`, `projects.close`, `commands.preview`, `commands.apply` | 5/5 |
| `media:read` | `uploads.list`, `assets.list`, `assets.get`, `imports.get` | 0/4 |
| `media:write` | `uploads.create`, `uploads.create_inline`, `uploads.delete`, `assets.import` | 4/4 |
| `run:read` | `runs.list`, `runs.get`, `runs.plan`, `runs.review`, `runs.cost`, `runs.events`, `memory.list`, `gateway.status` | 0/8 |
| `run:start` | `runs.create`, `runs.pause`, `runs.resume`, `runs.cancel`, `runs.variants` | 5/5 |
| `run:approve` | `runs.approve` | 1/1 |
| `export:read` | `exports.list`, `exports.get`, `deliverables.list` | 0/3 |
| `export:start` | `exports.start`, `exports.cancel` | 2/2 |
| `webhook:manage` | `webhooks.create`, `webhooks.list`, `webhooks.get`, `webhooks.update`, `webhooks.rotate_secret`, `webhooks.delete`, `webhooks.test`, `webhooks.deliveries`, `webhooks.redeliver` | 6/9 |
| `admin:tokens` | `tokens.create`, `tokens.list`, `tokens.revoke`, `tokens.rotate`, `audit.list` | 3/5 |
| _(público)_ | `server.health` | 0/1 |

Por rota:

| Operação | Método e rota | Scope | Classe | Mutante |
|---|---|---|---|---|
| `server.health` | `GET /v1/health` | — | `read` | não |
| `server.info` | `GET /v1/server` | `project:read` | `read` | não |
| `tokens.create` | `POST /v1/tokens` | `admin:tokens` | `admin` | sim |
| `tokens.list` | `GET /v1/tokens` | `admin:tokens` | `admin` | não |
| `tokens.revoke` | `DELETE /v1/tokens/{token_id}` | `admin:tokens` | `admin` | sim |
| `tokens.rotate` | `POST /v1/tokens/{token_id}/rotate` | `admin:tokens` | `admin` | sim |
| `audit.list` | `GET /v1/audit` | `admin:tokens` | `admin` | não |
| `projects.create` | `POST /v1/projects` | `project:write` | `write` | sim |
| `projects.list` | `GET /v1/projects` | `project:read` | `read` | não |
| `projects.get` | `GET /v1/projects/{project_id}` | `project:read` | `read` | não |
| `projects.open` | `POST /v1/projects/{project_id}/open` | `project:write` | `write` | sim |
| `projects.close` | `POST /v1/projects/{project_id}/close` | `project:write` | `write` | sim |
| `projects.summary` | `GET /v1/projects/{project_id}/summary` | `project:read` | `read` | não |
| `uploads.create` | `POST /v1/uploads` | `media:write` | `upload` | sim |
| `uploads.create_inline` | `POST /v1/uploads/inline` | `media:write` | `upload` | sim |
| `uploads.list` | `GET /v1/uploads` | `media:read` | `read` | não |
| `uploads.delete` | `DELETE /v1/uploads/{upload_id}` | `media:write` | `write` | sim |
| `assets.import` | `POST /v1/projects/{project_id}/assets` | `media:write` | `upload` | sim |
| `assets.list` | `GET /v1/projects/{project_id}/assets` | `media:read` | `read` | não |
| `assets.get` | `GET /v1/projects/{project_id}/assets/{asset_id}` | `media:read` | `read` | não |
| `imports.get` | `GET /v1/projects/{project_id}/imports/{ticket_id}` | `media:read` | `read` | não |
| `sequences.list` | `GET /v1/projects/{project_id}/sequences` | `project:read` | `read` | não |
| `sequences.get` | `GET /v1/projects/{project_id}/sequences/{sequence_id}` | `project:read` | `read` | não |
| `timeline.query` | `GET /v1/projects/{project_id}/sequences/{sequence_id}/timeline` | `project:read` | `read` | não |
| `commands.preview` | `POST /v1/projects/{project_id}/commands/preview` | `project:write` | `write` | sim |
| `commands.apply` | `POST /v1/projects/{project_id}/commands/apply` | `project:write` | `write` | sim |
| `history.list` | `GET /v1/projects/{project_id}/history` | `project:read` | `read` | não |
| `runs.create` | `POST /v1/projects/{project_id}/runs` | `run:start` | `run_start` | sim |
| `runs.list` | `GET /v1/projects/{project_id}/runs` | `run:read` | `read` | não |
| `runs.get` | `GET /v1/projects/{project_id}/runs/{run_id}` | `run:read` | `read` | não |
| `runs.pause` | `POST /v1/projects/{project_id}/runs/{run_id}/pause` | `run:start` | `write` | sim |
| `runs.resume` | `POST /v1/projects/{project_id}/runs/{run_id}/resume` | `run:start` | `run_start` | sim |
| `runs.cancel` | `POST /v1/projects/{project_id}/runs/{run_id}/cancel` | `run:start` | `write` | sim |
| `runs.approve` | `POST /v1/projects/{project_id}/runs/{run_id}/approvals` | `run:approve` | `approve` | sim |
| `runs.plan` | `GET /v1/projects/{project_id}/runs/{run_id}/plan` | `run:read` | `read` | não |
| `runs.review` | `GET /v1/projects/{project_id}/runs/{run_id}/review` | `run:read` | `read` | não |
| `runs.cost` | `GET /v1/projects/{project_id}/runs/{run_id}/cost` | `run:read` | `read` | não |
| `runs.events` | `GET /v1/projects/{project_id}/runs/{run_id}/events` | `run:read` | `read` | não |
| `runs.variants` | `POST /v1/projects/{project_id}/runs/{run_id}/variants` | `run:start` | `run_start` | sim |
| `memory.list` | `GET /v1/projects/{project_id}/memory` | `run:read` | `read` | não |
| `gateway.status` | `GET /v1/gateway` | `run:read` | `read` | não |
| `exports.start` | `POST /v1/projects/{project_id}/exports` | `export:start` | `export` | sim |
| `exports.list` | `GET /v1/projects/{project_id}/exports` | `export:read` | `read` | não |
| `exports.get` | `GET /v1/projects/{project_id}/exports/{export_id}` | `export:read` | `read` | não |
| `exports.cancel` | `POST /v1/projects/{project_id}/exports/{export_id}/cancel` | `export:start` | `write` | sim |
| `deliverables.list` | `GET /v1/projects/{project_id}/deliverables` | `export:read` | `read` | não |
| `webhooks.create` | `POST /v1/webhooks` | `webhook:manage` | `write` | sim |
| `webhooks.list` | `GET /v1/webhooks` | `webhook:manage` | `read` | não |
| `webhooks.get` | `GET /v1/webhooks/{webhook_id}` | `webhook:manage` | `read` | não |
| `webhooks.update` | `PATCH /v1/webhooks/{webhook_id}` | `webhook:manage` | `write` | sim |
| `webhooks.rotate_secret` | `POST /v1/webhooks/{webhook_id}/rotate-secret` | `webhook:manage` | `write` | sim |
| `webhooks.delete` | `DELETE /v1/webhooks/{webhook_id}` | `webhook:manage` | `write` | sim |
| `webhooks.test` | `POST /v1/webhooks/{webhook_id}/test` | `webhook:manage` | `write` | sim |
| `webhooks.deliveries` | `GET /v1/webhooks/{webhook_id}/deliveries` | `webhook:manage` | `read` | não |
| `webhooks.redeliver` | `POST /v1/webhooks/{webhook_id}/deliveries/{delivery_id}/redeliver` | `webhook:manage` | `write` | sim |
| `events.list` | `GET /v1/events` | `project:read` | `read` | não |

<!-- END GENERATED:scope-matrix -->

### Notas sobre os scopes

- `run:start` cobre também **pausar, retomar e cancelar** uma Run (`runs.pause|resume|cancel`) e iniciar variantes; **aprovar** uma decisão é um scope separado, `run:approve` (separação de funções: quem inicia não precisa poder aprovar gastos).
- `media:write` cobre upload, exclusão de upload e importação de asset; `media:read` cobre listagem/consulta.
- `export:start` cobre iniciar e cancelar exports; `export:read` cobre status e entregáveis.
- `webhook:manage` cobre todo o ciclo dos webhooks, incluindo reentrega e teste.
- `project:write` inclui `commands.apply`: um cliente com esse scope edita a timeline (sempre por `preview → apply_plan`, com `Actor::Api`, com undo).
