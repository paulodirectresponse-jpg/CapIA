# Tools MCP (geradas)

> **Gerado** por `node tools/docs/gen-api-docs.mjs` a partir do catálogo único (ADR-102). Não edite à mão. Regras gerais, resources e falhas: [mcp.md](mcp.md).

55 tools (uma por operação do catálogo, exceto as só-REST). Nome da tool = nome da operação com `.` trocado por `_`.

| Tool | Operação / rota REST | Scope | Efeito | Classe |
|---|---|---|---|---|
| `server_health` | `server.health` · `GET /v1/health` | — | leitura | `read` |
| `server_info` | `server.info` · `GET /v1/server` | `project:read` | leitura | `read` |
| `tokens_create` | `tokens.create` · `POST /v1/tokens` | `admin:tokens` | mutante | `admin` |
| `tokens_list` | `tokens.list` · `GET /v1/tokens` | `admin:tokens` | leitura | `admin` |
| `tokens_revoke` | `tokens.revoke` · `DELETE /v1/tokens/{token_id}` | `admin:tokens` | mutante | `admin` |
| `tokens_rotate` | `tokens.rotate` · `POST /v1/tokens/{token_id}/rotate` | `admin:tokens` | mutante | `admin` |
| `audit_list` | `audit.list` · `GET /v1/audit` | `admin:tokens` | leitura | `admin` |
| `projects_create` | `projects.create` · `POST /v1/projects` | `project:write` | mutante | `write` |
| `projects_list` | `projects.list` · `GET /v1/projects` | `project:read` | leitura | `read` |
| `projects_get` | `projects.get` · `GET /v1/projects/{project_id}` | `project:read` | leitura | `read` |
| `projects_open` | `projects.open` · `POST /v1/projects/{project_id}/open` | `project:write` | mutante | `write` |
| `projects_close` | `projects.close` · `POST /v1/projects/{project_id}/close` | `project:write` | mutante | `write` |
| `projects_summary` | `projects.summary` · `GET /v1/projects/{project_id}/summary` | `project:read` | leitura | `read` |
| `uploads_create_inline` | `uploads.create_inline` · `POST /v1/uploads/inline` | `media:write` | mutante | `upload` |
| `uploads_list` | `uploads.list` · `GET /v1/uploads` | `media:read` | leitura | `read` |
| `uploads_delete` | `uploads.delete` · `DELETE /v1/uploads/{upload_id}` | `media:write` | mutante | `write` |
| `assets_import` | `assets.import` · `POST /v1/projects/{project_id}/assets` | `media:write` | mutante | `upload` |
| `assets_list` | `assets.list` · `GET /v1/projects/{project_id}/assets` | `media:read` | leitura | `read` |
| `assets_get` | `assets.get` · `GET /v1/projects/{project_id}/assets/{asset_id}` | `media:read` | leitura | `read` |
| `imports_get` | `imports.get` · `GET /v1/projects/{project_id}/imports/{ticket_id}` | `media:read` | leitura | `read` |
| `sequences_list` | `sequences.list` · `GET /v1/projects/{project_id}/sequences` | `project:read` | leitura | `read` |
| `sequences_get` | `sequences.get` · `GET /v1/projects/{project_id}/sequences/{sequence_id}` | `project:read` | leitura | `read` |
| `timeline_query` | `timeline.query` · `GET /v1/projects/{project_id}/sequences/{sequence_id}/timeline` | `project:read` | leitura | `read` |
| `commands_preview` | `commands.preview` · `POST /v1/projects/{project_id}/commands/preview` | `project:write` | mutante | `write` |
| `commands_apply` | `commands.apply` · `POST /v1/projects/{project_id}/commands/apply` | `project:write` | mutante | `write` |
| `history_list` | `history.list` · `GET /v1/projects/{project_id}/history` | `project:read` | leitura | `read` |
| `runs_create` | `runs.create` · `POST /v1/projects/{project_id}/runs` | `run:start` | mutante | `run_start` |
| `runs_list` | `runs.list` · `GET /v1/projects/{project_id}/runs` | `run:read` | leitura | `read` |
| `runs_get` | `runs.get` · `GET /v1/projects/{project_id}/runs/{run_id}` | `run:read` | leitura | `read` |
| `runs_pause` | `runs.pause` · `POST /v1/projects/{project_id}/runs/{run_id}/pause` | `run:start` | mutante | `write` |
| `runs_resume` | `runs.resume` · `POST /v1/projects/{project_id}/runs/{run_id}/resume` | `run:start` | mutante | `run_start` |
| `runs_cancel` | `runs.cancel` · `POST /v1/projects/{project_id}/runs/{run_id}/cancel` | `run:start` | mutante | `write` |
| `runs_approve` | `runs.approve` · `POST /v1/projects/{project_id}/runs/{run_id}/approvals` | `run:approve` | mutante | `approve` |
| `runs_plan` | `runs.plan` · `GET /v1/projects/{project_id}/runs/{run_id}/plan` | `run:read` | leitura | `read` |
| `runs_review` | `runs.review` · `GET /v1/projects/{project_id}/runs/{run_id}/review` | `run:read` | leitura | `read` |
| `runs_cost` | `runs.cost` · `GET /v1/projects/{project_id}/runs/{run_id}/cost` | `run:read` | leitura | `read` |
| `runs_events` | `runs.events` · `GET /v1/projects/{project_id}/runs/{run_id}/events` | `run:read` | leitura | `read` |
| `runs_variants` | `runs.variants` · `POST /v1/projects/{project_id}/runs/{run_id}/variants` | `run:start` | mutante | `run_start` |
| `memory_list` | `memory.list` · `GET /v1/projects/{project_id}/memory` | `run:read` | leitura | `read` |
| `gateway_status` | `gateway.status` · `GET /v1/gateway` | `run:read` | leitura | `read` |
| `exports_start` | `exports.start` · `POST /v1/projects/{project_id}/exports` | `export:start` | mutante | `export` |
| `exports_list` | `exports.list` · `GET /v1/projects/{project_id}/exports` | `export:read` | leitura | `read` |
| `exports_get` | `exports.get` · `GET /v1/projects/{project_id}/exports/{export_id}` | `export:read` | leitura | `read` |
| `exports_cancel` | `exports.cancel` · `POST /v1/projects/{project_id}/exports/{export_id}/cancel` | `export:start` | mutante | `write` |
| `deliverables_list` | `deliverables.list` · `GET /v1/projects/{project_id}/deliverables` | `export:read` | leitura | `read` |
| `webhooks_create` | `webhooks.create` · `POST /v1/webhooks` | `webhook:manage` | mutante | `write` |
| `webhooks_list` | `webhooks.list` · `GET /v1/webhooks` | `webhook:manage` | leitura | `read` |
| `webhooks_get` | `webhooks.get` · `GET /v1/webhooks/{webhook_id}` | `webhook:manage` | leitura | `read` |
| `webhooks_update` | `webhooks.update` · `PATCH /v1/webhooks/{webhook_id}` | `webhook:manage` | mutante | `write` |
| `webhooks_rotate_secret` | `webhooks.rotate_secret` · `POST /v1/webhooks/{webhook_id}/rotate-secret` | `webhook:manage` | mutante | `write` |
| `webhooks_delete` | `webhooks.delete` · `DELETE /v1/webhooks/{webhook_id}` | `webhook:manage` | mutante | `write` |
| `webhooks_test` | `webhooks.test` · `POST /v1/webhooks/{webhook_id}/test` | `webhook:manage` | mutante | `write` |
| `webhooks_deliveries` | `webhooks.deliveries` · `GET /v1/webhooks/{webhook_id}/deliveries` | `webhook:manage` | leitura | `read` |
| `webhooks_redeliver` | `webhooks.redeliver` · `POST /v1/webhooks/{webhook_id}/deliveries/{delivery_id}/redeliver` | `webhook:manage` | mutante | `write` |
| `events_list` | `events.list` · `GET /v1/events` | `project:read` | leitura | `read` |

Só REST (sem tool MCP):

- `uploads.create` (`POST /v1/uploads`): Stream a file into staging (raw body; Content-Length required).
