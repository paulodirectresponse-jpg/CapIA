# ADR drafts — Trilha A (capia-server: núcleo, REST, contratos)

> Rascunhos para consolidar em `docs/DECISIONS.md` (o integrador numera). Contexto: Fase 6,
> `docs/phase6/PHASE6_*.md`.

## A-1 — `capia-server` é um host headless da MESMA Engine API; catálogo único de operações

**Decisão.** O crate `capia-server` hospeda `capia-editor-api::Session` + `capia-intelligence` (os
mesmos objetos que a UI usa). Toda operação externa é uma entrada do **catálogo**
(`src/catalog.rs`): nome, scope, mutante?, classe de rate limit, rota REST, schema JSON. REST, MCP,
OpenAPI, matriz de scopes e documentação derivam dele; um único pipeline (`Core::call`) executa
autenticação → scope → schema → rate limit → gate de projeto/shutdown → idempotência → handler →
auditoria. Não existe rota/tool fora do catálogo (testes de arquitetura do catálogo garantem: scope
em tudo exceto `server.health`, rotas únicas, GET nunca muta, nenhum parâmetro de caminho/segredo,
nenhum nome perigoso).

**Alternativas.** (a) Rotas escritas à mão por transporte — rejeitado: divergência REST×MCP é o
risco central; (b) framework HTTP (axum/hyper) — rejeitado por ora: superfície de dependência e
controle fino de limites (HTTP/1.1 mínimo e endurecido em `std::net`, como o devserver).

**Consequências.** Paridade UI↔REST↔MCP é por construção. Mudar uma operação = mudar o catálogo.

## A-2 — Escrita externa só por `preview → apply_plan` com `Actor::Api` por token

`commands.preview`/`commands.apply` usam `Session::agent_preview/agent_apply` (só Rust) com
`Actor::Api("token:<id>")`: o plan token HMAC do engine é preso ao ator, então outro token não
aplica o plano de quem fez o preview. Concorrência otimista: `expected_revision` → 409
`REVISION_CONFLICT`; plano velho → 409 `PLAN_STATE_CHANGED`. Gravações internas da sessão do
servidor (import de mídia finalizado no `pump`) usam o ator `System("capia-server")`. **Fora do
catálogo de propósito** (privilégio ≤ UI e regra "a IA só propõe"): undo/undo seletivo, aprovação de
memória (User/Client), configuração de providers/credenciais/gateway/geração, qualquer caminho de
arquivo do cliente. `ai.run.decide` aceita `decided_by` (`api:<token>`) para a trilha de auditoria
das aprovações; a UI continua gravando `user`.

## A-3 — Tokens, scopes, rotação

Token `capia_<64 hex>` (256 bits do SO), mostrado uma única vez; o banco guarda só o SHA-256
(índice único). Revogação, expiração e rotação atômica (novo + revoga antigo na mesma transação).
Sem escalada: um token só concede (create) ou rotaciona tokens cujos scopes ele possui.
`Authorization` inválido/revogado/expirado → a mesma resposta 401 (sem oráculo). O segredo é
registrado no redator global do processo (nunca sai em log/erro). Scopes: `project:read|write`,
`media:read|write`, `run:read|start|approve`, `export:read|start`, `webhook:manage`,
`admin:tokens`; `run:approve` é separado de `run:start` (menor privilégio).

## A-4 — Idempotência e auditoria de toda escrita externa

`Idempotency-Key` (REST) / `idempotency_key` (MCP) por (token, chave): replay devolve o mesmo
resultado semântico (cabeçalho `Idempotent-Replay`); mesma chave com pedido diferente → 422
`IDEMPOTENCY_KEY_REUSED`; em andamento → 409; `pending` deixado por processo que caiu →
`IDEMPOTENCY_INDETERMINATE` (o cliente verifica o estado e usa outra chave — nunca executa duas
vezes). Respostas com segredo único (tokens/webhooks) nunca entram na tabela de idempotência: o
replay vem sem o segredo (`secret_unavailable_on_replay`). Auditoria (`audit`): toda escrita e toda
recusa/erro com token, superfície, operação, revisões antes/depois e chave; leituras bem-sucedidas
não escrevem no banco; o log é aparado (100 mil entradas).

## A-5 — Um projeto aberto por vez; troca exclui chamadas em voo

O engine hospeda um projeto por sessão. Rotas `/v1/projects/{id}/…` exigem esse projeto aberto
(409 `PROJECT_NOT_OPEN`). `projects.create/open/close` só com Runs/exports ociosos (409
`PROJECT_BUSY`) e sob um `RwLock`: uma chamada validada para P1 nunca roda contra P2 (sem TOCTOU).
Projetos moram em `<data>/projects/<id>/project.capia`; **nenhum caminho do cliente** entra na API.

## A-6 — Uploads: streaming para staging, sniff, mídia durável, nada de caminhos na resposta

`POST /v1/uploads` (Content-Length obrigatório; sem `Transfer-Encoding`): hash durante a escrita,
teto por tipo/tamanho/cota/tempo/concorrência, *sniff* por bytes (nunca nome/MIME do cliente),
`*.part` → `rename` atômico, idempotente por conteúdo (token+hash+nome). Importar move a mídia do
staging para `<projeto>/media/<upload_id>/` (durável) e entra pelo sistema de assets (hash + probe +
catálogo na mesma transação). Documentos de uma Run são resolvidos pelo servidor a partir do
`upload_id`. Respostas nunca carregam caminhos do servidor (exceto o `path` de um export, que está
sob a raiz de saída escolhida pelo servidor).

## A-7 — Transporte: loopback, Host, CORS, limites, backpressure

Padrão loopback. Bind remoto exige `--allow-remote` **e** `--remote-tls-terminated-by-proxy` (nunca
token em texto claro por padrão; sem TLS nativo — proxy reverso documentado). `Host` só do próprio
servidor (421; anti DNS-rebinding). CORS fechado: qualquer `Origin` não listado → 403 (CSRF de
navegador); `*` proibido. Limites: linha 8 KiB, cabeçalhos 32 KiB/64, JSON 1 MiB e profundidade 32,
prazo de 10 s para o cabeçalho (slowloris), pool de workers fixo + fila finita (503 `OVERLOADED`,
nunca thread por conexão), SSE com teto. Shutdown gracioso: recusa escrita (503), pausa Runs
(retomáveis), espera o que está em voo, fecha o projeto.

## A-8 — Segredos de webhook no cofre; sem cofre ⇒ memória

O segredo de assinatura de um webhook fica só no `SecretStore` (Credential Manager no Windows). Sem
cofre do SO (Linux/CI) o armazenamento é em memória (nunca arquivo em claro): após reinício as
entregas do webhook ficam `dead` com `SECRET_UNAVAILABLE` até `webhooks.rotate_secret`, que as
reenfileira.
