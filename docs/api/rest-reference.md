# Referência REST (`/v1`)

> **Gerado** por `node tools/docs/gen-api-docs.mjs` a partir do catálogo único de operações (`crates/capia-server/src/catalog.rs`, ADR-102). **Não edite à mão** — mude o catálogo e regenere. Convenções gerais (envelope de erro, idempotência, paginação, assíncrono) estão em [README.md](README.md); scopes em [auth-and-scopes.md](auth-and-scopes.md).

O catálogo tem **56 operações** (55 também como tools MCP). Toda rota, exceto `server.health`, exige `Authorization: Bearer <token>`.

Os exemplos usam `$CAPIA_PORT` e `$CAPIA_TOKEN` (placeholders; nunca coloque um token real em script versionado). Os ids (`prj_example`, `run_example`…) são ilustrativos. O corpo das respostas não é descrito aqui: o servidor repassa o resultado dos serviços da Engine API e da IA — ver [canonical-flow.md](canonical-flow.md) para os campos que o fluxo usa.

## Índice

- **Servidor:** [`server.health`](#serverhealth), [`server.info`](#serverinfo)
- **Tokens:** [`tokens.create`](#tokenscreate), [`tokens.list`](#tokenslist), [`tokens.revoke`](#tokensrevoke), [`tokens.rotate`](#tokensrotate)
- **Auditoria:** [`audit.list`](#auditlist)
- **Projetos:** [`projects.create`](#projectscreate), [`projects.list`](#projectslist), [`projects.get`](#projectsget), [`projects.open`](#projectsopen), [`projects.close`](#projectsclose), [`projects.summary`](#projectssummary)
- **Uploads:** [`uploads.create`](#uploadscreate), [`uploads.create_inline`](#uploadscreate_inline), [`uploads.list`](#uploadslist), [`uploads.delete`](#uploadsdelete)
- **Assets:** [`assets.import`](#assetsimport), [`assets.list`](#assetslist), [`assets.get`](#assetsget), [`imports.get`](#importsget)
- **Sequences e timeline:** [`sequences.list`](#sequenceslist), [`sequences.get`](#sequencesget), [`timeline.query`](#timelinequery)
- **Comandos (preview → apply):** [`commands.preview`](#commandspreview), [`commands.apply`](#commandsapply), [`history.list`](#historylist)
- **AI Runs:** [`runs.create`](#runscreate), [`runs.list`](#runslist), [`runs.get`](#runsget), [`runs.pause`](#runspause), [`runs.resume`](#runsresume), [`runs.cancel`](#runscancel), [`runs.approve`](#runsapprove), [`runs.plan`](#runsplan), [`runs.review`](#runsreview), [`runs.cost`](#runscost), [`runs.events`](#runsevents), [`runs.variants`](#runsvariants), [`memory.list`](#memorylist), [`gateway.status`](#gatewaystatus)
- **Exports e entregáveis:** [`exports.start`](#exportsstart), [`exports.list`](#exportslist), [`exports.get`](#exportsget), [`exports.cancel`](#exportscancel), [`deliverables.list`](#deliverableslist)
- **Webhooks:** [`webhooks.create`](#webhookscreate), [`webhooks.list`](#webhookslist), [`webhooks.get`](#webhooksget), [`webhooks.update`](#webhooksupdate), [`webhooks.rotate_secret`](#webhooksrotate_secret), [`webhooks.delete`](#webhooksdelete), [`webhooks.test`](#webhookstest), [`webhooks.deliveries`](#webhooksdeliveries), [`webhooks.redeliver`](#webhooksredeliver)
- **Eventos:** [`events.list`](#eventslist)

## Servidor

### `server.health`

Liveness probe (public, minimal).

- **Rota:** `GET /v1/health`
- **Scope:** _nenhum (público)_
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`server_health`)
- **Exige o projeto aberto:** não

_Sem parâmetros._

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/health"
```

### `server.info`

Server, engine and limits information.

- **Rota:** `GET /v1/server`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`server_info`)
- **Exige o projeto aberto:** não

_Sem parâmetros._

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/server" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Tokens

### `tokens.create`

Create a token (the secret is shown once).

- **Rota:** `POST /v1/tokens`
- **Scope:** `admin:tokens`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `admin`
- **Sucesso:** HTTP 201
- **Superfície:** REST + MCP (`tokens_create`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `expires_in_seconds` | corpo | não | integer, 60..31536000 |
| `name` | corpo | sim | string, 1..80 caracteres |
| `scopes` | corpo | sim | array, ≤ 11 itens |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/tokens" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"name":"ci-bot","scopes":["project:read","run:read"],"expires_in_seconds":86400}'
```

Corpo:

```json
{
  "name": "ci-bot",
  "scopes": [
    "project:read",
    "run:read"
  ],
  "expires_in_seconds": 86400
}
```

### `tokens.list`

List tokens (never the secrets).

- **Rota:** `GET /v1/tokens`
- **Scope:** `admin:tokens`
- **Efeito:** somente leitura · classe de rate limit `admin`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`tokens_list`)
- **Exige o projeto aberto:** não

_Sem parâmetros._

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/tokens" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `tokens.revoke`

Revoke a token.

- **Rota:** `DELETE /v1/tokens/{token_id}`
- **Scope:** `admin:tokens`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `admin`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`tokens_revoke`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `token_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X DELETE "http://127.0.0.1:$CAPIA_PORT/v1/tokens/tok_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `tokens.rotate`

Rotate a token: new secret (shown once), old one revoked.

- **Rota:** `POST /v1/tokens/{token_id}/rotate`
- **Scope:** `admin:tokens`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `admin`
- **Sucesso:** HTTP 201
- **Superfície:** REST + MCP (`tokens_rotate`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `token_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/tokens/tok_example/rotate" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

## Auditoria

### `audit.list`

Audit log of external calls.

- **Rota:** `GET /v1/audit`
- **Scope:** `admin:tokens`
- **Efeito:** somente leitura · classe de rate limit `admin`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`audit_list`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `after` | query | não | integer, 0..9223372036854776000 |
| `limit` | query | não | integer, 1..500 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/audit" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Projetos

### `projects.create`

Create (and open) a project in the server's project store.

- **Rota:** `POST /v1/projects`
- **Scope:** `project:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 201
- **Superfície:** REST + MCP (`projects_create`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `name` | corpo | sim | string, 1..80 caracteres |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"name":"Meu projeto"}'
```

Corpo:

```json
{
  "name": "Meu projeto"
}
```

### `projects.list`

List the server's projects.

- **Rota:** `GET /v1/projects`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`projects_list`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `after` | query | não | string, 1..128 caracteres |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `projects.get`

One project.

- **Rota:** `GET /v1/projects/{project_id}`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`projects_get`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `projects.open`

Open a project (the engine hosts one at a time).

- **Rota:** `POST /v1/projects/{project_id}/open`
- **Scope:** `project:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`projects_open`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/open" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `projects.close`

Close the open project.

- **Rota:** `POST /v1/projects/{project_id}/close`
- **Scope:** `project:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`projects_close`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/close" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `projects.summary`

Revision, sequences, clips, assets and run counts.

- **Rota:** `GET /v1/projects/{project_id}/summary`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`projects_summary`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/summary" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Uploads

### `uploads.create`

Stream a file into staging (raw body; Content-Length required).

- **Rota:** `POST /v1/uploads`
- **Scope:** `media:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `upload`
- **Sucesso:** HTTP 201
- **Superfície:** somente REST (transporte em streaming que o MCP não tem)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `expected_sha256` | corpo | não | string, `^[0-9a-f]{64}$` |
| `filename` | corpo | sim | string, 1..255 caracteres |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/uploads" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"filename":"raw.mp4"}'
```

Corpo:

```json
{
  "filename": "raw.mp4"
}
```

### `uploads.create_inline`

Stage a small file sent as base64 (8 MiB max).

- **Rota:** `POST /v1/uploads/inline`
- **Scope:** `media:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `upload`
- **Sucesso:** HTTP 201
- **Superfície:** REST + MCP (`uploads_create_inline`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `content_base64` | corpo | sim | string, 1..11534336 caracteres |
| `expected_sha256` | corpo | não | string, `^[0-9a-f]{64}$` |
| `filename` | corpo | sim | string, 1..255 caracteres |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/uploads/inline" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"filename":"raw.mp4","content_base64":"<BASE64_DO_ARQUIVO>"}'
```

Corpo:

```json
{
  "filename": "raw.mp4",
  "content_base64": "<BASE64_DO_ARQUIVO>"
}
```

### `uploads.list`

List staged uploads.

- **Rota:** `GET /v1/uploads`
- **Scope:** `media:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`uploads_list`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `after` | query | não | string, 1..128 caracteres |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/uploads" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `uploads.delete`

Delete a staged upload.

- **Rota:** `DELETE /v1/uploads/{upload_id}`
- **Scope:** `media:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`uploads_delete`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `upload_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X DELETE "http://127.0.0.1:$CAPIA_PORT/v1/uploads/upl_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

## Assets

### `assets.import`

Import a staged upload through the asset system (hash + probe + catalog).

- **Rota:** `POST /v1/projects/{project_id}/assets`
- **Scope:** `media:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `upload`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`assets_import`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `upload_id` | corpo | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/assets" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"upload_id":"upl_example"}'
```

Corpo:

```json
{
  "upload_id": "upl_example"
}
```

### `assets.list`

List assets.

- **Rota:** `GET /v1/projects/{project_id}/assets`
- **Scope:** `media:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`assets_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | string, 1..128 caracteres |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/assets" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `assets.get`

One asset.

- **Rota:** `GET /v1/projects/{project_id}/assets/{asset_id}`
- **Scope:** `media:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`assets_get`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `asset_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/assets/ast_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `imports.get`

State of an asynchronous import.

- **Rota:** `GET /v1/projects/{project_id}/imports/{ticket_id}`
- **Scope:** `media:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`imports_get`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `ticket_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/imports/tkt_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Sequences e timeline

### `sequences.list`

List sequences.

- **Rota:** `GET /v1/projects/{project_id}/sequences`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`sequences_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/sequences" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `sequences.get`

One sequence (full content; use timeline.query for large ones).

- **Rota:** `GET /v1/projects/{project_id}/sequences/{sequence_id}`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`sequences_get`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `sequence_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/sequences/seq_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `timeline.query`

Clips of a sequence by time range, paginated, with a content digest.

- **Rota:** `GET /v1/projects/{project_id}/sequences/{sequence_id}/timeline`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`timeline_query`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `sequence_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `from_ticks` | query | não | integer, 0..9223372036854776000 |
| `limit` | query | não | integer, 1..500 |
| `to_ticks` | query | não | integer, 0..9223372036854776000 |
| `track` | query | não | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/sequences/seq_example/timeline" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Comandos (preview → apply)

### `commands.preview`

Phase 1 of the write gate: validate a transaction and get a plan token (nothing is written).

- **Rota:** `POST /v1/projects/{project_id}/commands/preview`
- **Scope:** `project:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`commands_preview`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `commands` | corpo | sim | array, ≤ 500 itens |
| `expected_revision` | corpo | não | integer, 0..9223372036854776000 |
| `label` | corpo | não | string, 1..120 caracteres |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/commands/preview" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"label":"Ajustar título","expected_revision":12,"commands":[{"...":"comando estruturado do Command Engine (ver docs/COMMAND_SYSTEM.md)"}]}'
```

Corpo:

```json
{
  "label": "Ajustar título",
  "expected_revision": 12,
  "commands": [
    {
      "...": "comando estruturado do Command Engine (ver docs/COMMAND_SYSTEM.md)"
    }
  ]
}
```

### `commands.apply`

Phase 2 of the write gate: apply only the reviewed plan.

- **Rota:** `POST /v1/projects/{project_id}/commands/apply`
- **Scope:** `project:write`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`commands_apply`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `plan_token` | corpo | sim | string, 1..4096 caracteres |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/commands/apply" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"plan_token":"<plan_token devolvido por commands.preview>"}'
```

Corpo:

```json
{
  "plan_token": "<plan_token devolvido por commands.preview>"
}
```

### `history.list`

Document history entries.

- **Rota:** `GET /v1/projects/{project_id}/history`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`history_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | integer, 0..9223372036854776000 |
| `limit` | query | não | integer, 1..500 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/history" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## AI Runs

### `runs.create`

Start an AI Run from brief + raw footage + references.

- **Rota:** `POST /v1/projects/{project_id}/runs`
- **Scope:** `run:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `run_start`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`runs_create`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `assets` | corpo | não | array, ≤ 50 itens |
| `brief_text` | corpo | não | string, 1..60000 caracteres |
| `budget` | corpo | não | object, ≤ 12 chaves |
| `client_id` | corpo | não | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `deliverables` | corpo | não | array, ≤ 24 itens |
| `documents` | corpo | não | array, ≤ 12 itens |
| `note` | corpo | não | string, 1..2000 caracteres |
| `policy` | corpo | não | object, ≤ 8 chaves |
| `references` | corpo | não | array, ≤ 8 itens |
| `start` | corpo | não | boolean |
| `variants` | corpo | não | object, ≤ 8 chaves |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"brief_text":"Produto: Curso X. Público: iniciantes. Oferta: 30% off. CTA: Inscreva-se.","assets":["ast_example"],"references":["ast_reference"],"start":true,"budget":{"max_cost_micros":2000000}}'
```

Corpo:

```json
{
  "brief_text": "Produto: Curso X. Público: iniciantes. Oferta: 30% off. CTA: Inscreva-se.",
  "assets": [
    "ast_example"
  ],
  "references": [
    "ast_reference"
  ],
  "start": true,
  "budget": {
    "max_cost_micros": 2000000
  }
}
```

### `runs.list`

List runs.

- **Rota:** `GET /v1/projects/{project_id}/runs`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `runs.get`

Run snapshot (stages, effects, provenance, pending decision).

- **Rota:** `GET /v1/projects/{project_id}/runs/{run_id}`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_get`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `runs.pause`

Pause a run.

- **Rota:** `POST /v1/projects/{project_id}/runs/{run_id}/pause`
- **Scope:** `run:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_pause`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/pause" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `runs.resume`

Resume a paused run.

- **Rota:** `POST /v1/projects/{project_id}/runs/{run_id}/resume`
- **Scope:** `run:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `run_start`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`runs_resume`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/resume" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `runs.cancel`

Cancel a run.

- **Rota:** `POST /v1/projects/{project_id}/runs/{run_id}/cancel`
- **Scope:** `run:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_cancel`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/cancel" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `runs.approve`

Answer the pending approval/decision of a run.

- **Rota:** `POST /v1/projects/{project_id}/runs/{run_id}/approvals`
- **Scope:** `run:approve`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `approve`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_approve`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `decision_id` | corpo | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `option` | corpo | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `payload` | corpo | não | object, ≤ 32 chaves |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/approvals" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"decision_id":"dec_example","option":"approve"}'
```

Corpo:

```json
{
  "decision_id": "dec_example",
  "option": "approve"
}
```

### `runs.plan`

Production plan, edit plans and validation of a run.

- **Rota:** `GET /v1/projects/{project_id}/runs/{run_id}/plan`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_plan`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/plan" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `runs.review`

Reviews (critic) of a run.

- **Rota:** `GET /v1/projects/{project_id}/runs/{run_id}/review`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_review`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/review" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `runs.cost`

Usage and budget of a run.

- **Rota:** `GET /v1/projects/{project_id}/runs/{run_id}/cost`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_cost`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/cost" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `runs.events`

Run event log (paginated by seq).

- **Rota:** `GET /v1/projects/{project_id}/runs/{run_id}/events`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`runs_events`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | integer, 0..9223372036854776000 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/events" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `runs.variants`

Create variants from a completed run.

- **Rota:** `POST /v1/projects/{project_id}/runs/{run_id}/variants`
- **Scope:** `run:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `run_start`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`runs_variants`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `run_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `axis` | corpo | não | array, ≤ 8 itens |
| `count` | corpo | sim | integer, 1..20 |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/runs/run_example/variants" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"count":3,"axis":["hook"]}'
```

Corpo:

```json
{
  "count": 3,
  "axis": [
    "hook"
  ]
}
```

### `memory.list`

Project/client/user memory items (read-only: promotion is a UI action).

- **Rota:** `GET /v1/projects/{project_id}/memory`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`memory_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/memory" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `gateway.status`

Asset Gateway and generation status.

- **Rota:** `GET /v1/gateway`
- **Scope:** `run:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`gateway_status`)
- **Exige o projeto aberto:** não

_Sem parâmetros._

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/gateway" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Exports e entregáveis

### `exports.start`

Queue exports of sequences (server chooses the output location).

- **Rota:** `POST /v1/projects/{project_id}/exports`
- **Scope:** `export:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `export`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`exports_start`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `items` | corpo | sim | array, ≤ 24 itens |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/exports" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"items":[{"sequence":"seq_example","preset":"h264-mp4","name":"vertical-9x16"}]}'
```

Corpo:

```json
{
  "items": [
    {
      "sequence": "seq_example",
      "preset": "h264-mp4",
      "name": "vertical-9x16"
    }
  ]
}
```

### `exports.list`

List exports.

- **Rota:** `GET /v1/projects/{project_id}/exports`
- **Scope:** `export:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`exports_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | string, 1..128 caracteres |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/exports" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `exports.get`

Export state, report and probe.

- **Rota:** `GET /v1/projects/{project_id}/exports/{export_id}`
- **Scope:** `export:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`exports_get`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `export_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/exports/exp_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `exports.cancel`

Cancel the batch of an export.

- **Rota:** `POST /v1/projects/{project_id}/exports/{export_id}/cancel`
- **Scope:** `export:start`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`exports_cancel`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `export_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/exports/exp_example/cancel" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `deliverables.list`

Completed exports (deliverables) of the project.

- **Rota:** `GET /v1/projects/{project_id}/deliverables`
- **Scope:** `export:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`deliverables_list`)
- **Exige o projeto aberto:** sim (`{project_id}` precisa ser o projeto aberto no servidor)

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `project_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | string, 1..128 caracteres |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/projects/prj_example/deliverables" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Webhooks

### `webhooks.create`

Register a webhook (the signing secret is shown once).

- **Rota:** `POST /v1/webhooks`
- **Scope:** `webhook:manage`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 201
- **Superfície:** REST + MCP (`webhooks_create`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `description` | corpo | não | string, 0..200 caracteres |
| `events` | corpo | sim | array, ≤ 16 itens |
| `url` | corpo | sim | string, 1..2048 caracteres |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/webhooks" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"url":"http://127.0.0.1:9000/capia-webhook","events":["run.completed","run.failed","export.completed"],"description":"receptor local"}'
```

Corpo:

```json
{
  "url": "http://127.0.0.1:9000/capia-webhook",
  "events": [
    "run.completed",
    "run.failed",
    "export.completed"
  ],
  "description": "receptor local"
}
```

### `webhooks.list`

List webhooks (never the secrets).

- **Rota:** `GET /v1/webhooks`
- **Scope:** `webhook:manage`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`webhooks_list`)
- **Exige o projeto aberto:** não

_Sem parâmetros._

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/webhooks" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `webhooks.get`

One webhook.

- **Rota:** `GET /v1/webhooks/{webhook_id}`
- **Scope:** `webhook:manage`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`webhooks_get`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `webhooks.update`

Update url/events/enabled/description.

- **Rota:** `PATCH /v1/webhooks/{webhook_id}`
- **Scope:** `webhook:manage`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`webhooks_update`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `description` | corpo | não | string, 0..200 caracteres |
| `enabled` | corpo | não | boolean |
| `events` | corpo | não | array, ≤ 16 itens |
| `url` | corpo | não | string, 1..2048 caracteres |

Exemplo:

```bash
curl -sS -X PATCH "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{"enabled":false}'
```

Corpo:

```json
{
  "enabled": false
}
```

### `webhooks.rotate_secret`

New signing secret (shown once).

- **Rota:** `POST /v1/webhooks/{webhook_id}/rotate-secret`
- **Scope:** `webhook:manage`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`webhooks_rotate_secret`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example/rotate-secret" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `webhooks.delete`

Delete a webhook and its delivery log.

- **Rota:** `DELETE /v1/webhooks/{webhook_id}`
- **Scope:** `webhook:manage`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`webhooks_delete`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X DELETE "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `webhooks.test`

Queue a synthetic `webhook.test` delivery.

- **Rota:** `POST /v1/webhooks/{webhook_id}/test`
- **Scope:** `webhook:manage`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`webhooks_test`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example/test" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

### `webhooks.deliveries`

Delivery log (attempt, status, latency, next retry, terminal state).

- **Rota:** `GET /v1/webhooks/{webhook_id}/deliveries`
- **Scope:** `webhook:manage`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`webhooks_deliveries`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `after` | query | não | integer, 0..9223372036854776000 |
| `limit` | query | não | integer, 1..200 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example/deliveries" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

### `webhooks.redeliver`

Re-deliver one delivery (dead letters included).

- **Rota:** `POST /v1/webhooks/{webhook_id}/deliveries/{delivery_id}/redeliver`
- **Scope:** `webhook:manage`
- **Efeito:** mutante (aceita `Idempotency-Key`) · classe de rate limit `write`
- **Sucesso:** HTTP 202 (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)
- **Superfície:** REST + MCP (`webhooks_redeliver`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `webhook_id` | caminho | sim | string, 1..128 caracteres, `^[A-Za-z0-9._:-]+$` |
| `delivery_id` | caminho | sim | integer, 1..9223372036854776000 |

Exemplo:

```bash
curl -sS -X POST "http://127.0.0.1:$CAPIA_PORT/v1/webhooks/whk_example/deliveries/dlv_example/redeliver" \
  -H "Authorization: Bearer $CAPIA_TOKEN" \
  -H "Idempotency-Key: $(uuidgen)"
```

## Eventos

### `events.list`

Server events after a cursor (polling; SSE at /v1/events/stream).

- **Rota:** `GET /v1/events`
- **Scope:** `project:read`
- **Efeito:** somente leitura · classe de rate limit `read`
- **Sucesso:** HTTP 200
- **Superfície:** REST + MCP (`events_list`)
- **Exige o projeto aberto:** não

| Parâmetro | Onde | Obrigatório | Restrições |
|---|---|---|---|
| `after` | query | não | integer, 0..9223372036854776000 |
| `limit` | query | não | integer, 1..500 |

Exemplo:

```bash
curl -sS -X GET "http://127.0.0.1:$CAPIA_PORT/v1/events" \
  -H "Authorization: Bearer $CAPIA_TOKEN"
```

## Eventos publicados

Tipos aceitos em `webhooks.create.events` (`*` assina todos): `run.started`, `run.waiting_user`, `run.completed`, `run.failed`, `run.cancelled`, `export.completed`, `export.failed`, `asset.imported`, `webhook.test`. Ver [webhooks.md](webhooks.md).
