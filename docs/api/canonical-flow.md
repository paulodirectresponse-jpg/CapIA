# Fluxo canônico externo

**Objetivo:** um sistema externo entrega **copy (briefing) + vídeo bruto + vídeo de referência** e recebe de volta **variantes editáveis e um export**, com aprovação humana onde a política exigir e um **webhook** na conclusão. O mesmo fluxo por UI, REST ou MCP chega ao mesmo estado autoritativo do projeto (revisão, sequences, grafo de clips, registros de Run, sondagem do export).

Implementações executáveis: [`examples/rest/`](../../examples/rest/) (curl, PowerShell, Node, Python), [`examples/mcp/`](../../examples/mcp/) e [`examples/webhook-receiver/`](../../examples/webhook-receiver/). Referência de cada rota: [rest-reference.md](rest-reference.md).

> **Formas de resposta.** O servidor repassa o resultado dos serviços da Engine API e da IA. Os exemplos assumem: `projects.create` → `{ "project": { "id" } }`; uploads → `{ "upload": { "id" } }`; importação → `{ "ticket": { "id", "state", "asset_id" } }`; Run → `{ "run": { "id", "status", "stage", "pending", "sequences" } }` (forma do serviço `ai.run.*`); export → `{ "export": { "id", "state" } }`. Os clientes de exemplo aceitam também `{ "id" }` no topo. Confirme contra o servidor real (item `external-flow` da aceitação).

## Pré-requisitos

- Servidor em execução em `127.0.0.1:<porta>` ([../user/09-api-local.md](../user/09-api-local.md)) e **IA configurada** (um provider com chave no cofre — a REST não envia chaves; [../user/03-provedores-de-ia.md](../user/03-provedores-de-ia.md)). Com a IA desligada, `runs.create` responde `AI_OFF` (409) e o resto da API de edição continua funcionando.
- Token com os scopes: `project:read project:write media:read media:write run:read run:start run:approve export:read export:start` (+ `webhook:manage` para o passo 0). Em produção, separe: o cliente que inicia Runs não precisa de `run:approve`.
- FFmpeg disponível no servidor (importação, preview e export dependem dele).

## Os 12 passos

Todas as chamadas levam `Authorization: Bearer $CAPIA_TOKEN`; as mutantes levam `Idempotency-Key` (mesma chave ao repetir).

**0. (opcional) Registrar o webhook** — `POST /v1/webhooks` `{"url":"http://127.0.0.1:9000/","events":["run.completed","run.failed","run.waiting_user","export.completed","export.failed"]}`. Guarde o segredo da resposta (**só aparece agora**) e suba o receptor de exemplo com ele.

1. **Criar/abrir o projeto** — `POST /v1/projects` `{"name":"Campanha X"}` (cria e abre; o servidor hospeda um projeto aberto por vez). Para um existente: `GET /v1/projects` e `POST /v1/projects/{id}/open`.
2. **Enviar o vídeo bruto** — `POST /v1/uploads/inline` `{"filename":"raw.mp4","content_base64":"…"}` (arquivos pequenos) ou `POST /v1/uploads` com o arquivo em streaming (`uploads.create`, só REST; parâmetros `filename`/`expected_sha256` — encaminhamento exato é interface prevista). Depois **importar**: `POST /v1/projects/{id}/assets` `{"upload_id":"…"}` → **202** com o ticket; consulte `GET /v1/projects/{id}/imports/{ticket_id}` até `completed` (o hash, a sondagem e a identidade por conteúdo são do sistema de assets; o servidor nunca confia em nome/MIME do cliente). O `asset_id` sai do ticket (ou de `GET …/assets`).
3. **Enviar/indicar a referência** — mesmo caminho do passo 2 para um arquivo de referência (será analisado pelo Reference Analyzer). Uma **URL aprovada** de referência é obtida pelo Asset Gateway dentro da Run (adapter `ApprovedUrl`, política de licença e aprovação), não por um campo `url` da API.
4. **Submeter copy/briefing** — vai em `brief_text` (≤ 60 000 caracteres) do passo 5; documentos `.docx/.pdf/.txt/.md` entram como assets e seus ids em `documents` (≤ 12). Opcionalmente `client_id` (memória por cliente) e `note`.
5. **Iniciar a Run** — `POST /v1/projects/{id}/runs` `{"brief_text":"…","assets":["<bruto>"],"references":["<ref>"],"start":true,"budget":{"max_cost_micros":2000000}}` → **202** com `run.id`. `policy` e `budget` ajustam aprovações e tetos ([../user/05-aprovacoes-e-orcamentos.md](../user/05-aprovacoes-e-orcamentos.md)); sem eles valem os padrões seguros.
6. **Acompanhar** — `GET …/runs/{run_id}` (status e estágio: `understand → plan → validate_plan → acquire → edit → review → correct → done`), `GET …/runs/{run_id}/events?after=<seq>`, `GET …/runs/{run_id}/cost`; ou espere o webhook. SSE é interface prevista.
7. **Tratar aprovações** — quando `run.status == "waiting_user"`, `run.pending` traz a decisão (`id`, `kind`, `question`, `options[]`, `consequences`). `POST …/runs/{run_id}/approvals` `{"decision_id":"…","option":"approve"}` (scope `run:approve`). Inspecione antes: `GET …/runs/{run_id}/plan` e `GET …/runs/{run_id}/review`. Os exemplos **só aprovam sozinhos** os tipos listados em `--approve`/`APPROVE_KINDS`; gasto, geração e licença ficam para um humano.
8. **Esperar a conclusão** — `completed`, `failed` ou `cancelled`. `paused` (p. ex. após reinício do servidor) precisa de `POST …/runs/{run_id}/resume`.
9. **Listar variantes** — `POST …/runs/{run_id}/variants` `{"count":3,"axis":["hook"]}` → **202** (Runs filhas em grupo); acompanhe com `GET …/runs` e liste os resultados com `GET …/sequences` (sequences distintas e **100 % editáveis** na UI).
10. **Exportar** — `POST …/exports` `{"items":[{"sequence":"<id>","preset":"h264-mp4"}]}` → **202**; consulte `GET …/exports/{export_id}` e liste `GET …/deliverables`. Só encoders **aprovados** são usados; sem encoder H.264 aprovado a exportação falha com erro claro (sem fallback silencioso).
11. **Webhook** — o seu receptor recebe `run.completed` e `export.completed` assinados ([webhooks.md](webhooks.md)).
12. **Verificar o estado na UI** — abra o projeto no CapIA: as sequences da Run estão na timeline, editáveis, com o histórico atribuído à Run (`run:<id>`) e o **desfazer da Run** disponível (só pela UI). Por API: `GET …/summary`, `GET …/history`, `GET …/sequences/{id}/timeline`.

## Falhas comuns

| Sintoma | Causa provável | O que fazer |
|---|---|---|
| `409 AI_OFF` / `NOT_CONFIGURED` em `runs.create` | IA desligada ou sem provider | configure em Configurações → IA ([03](../user/03-provedores-de-ia.md)); o editor segue sem IA |
| `409 NO_PROJECT_OPEN` / `PROJECT_NOT_OPEN` | o projeto da rota não é o aberto | `projects.open` |
| `409 TOO_MANY_RUNS` / `RUN_BUSY` | limite de Runs simultâneas | aguarde ou cancele |
| `409 STALE_PLAN`/`REVISION_CONFLICT` | o projeto mudou (UI, outro cliente) | refaça a prévia e decida de novo |
| `waiting_user` que não sai | decisão sem aprovação | `runs.approve` com `run:approve` (ou pela UI) |
| `403` | token sem o scope | crie token com o scope necessário |
| `429` | rate limit | respeite `Retry-After` |
| Timeout num POST | resposta perdida | repita com a **mesma** `Idempotency-Key` |
