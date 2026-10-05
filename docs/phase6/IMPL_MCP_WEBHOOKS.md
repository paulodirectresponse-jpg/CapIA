# Fase 6 — Implementação: MCP e Webhooks (Trilha B)

Escopo: `crates/capia-server/src/mcp.rs`, `src/webhooks.rs` e as suítes `tests/{mcp,webhooks,parity,external_flow}.rs`.
Contrato de origem: `PHASE6_API_MCP_WEBHOOKS.md` §19–§31, `PHASE6_COMPLETION.md` §15–§19, `PHASE6_PERFORMANCE_SECURITY.md` §16 e §21.

## 1. Princípio: um pipeline, três portas

REST, MCP e testes chamam **`Core::call(ctx, op, params)`**. Autenticação de escopo, validação de schema,
rate limit, gate de projeto, idempotência e auditoria acontecem ali, uma única vez. O MCP é um adaptador
JSON-RPC fino:

```
cliente MCP ── JSON-RPC ──► mcp::handle_rpc ──► Core::call (surface = "mcp") ──► Engine API
```

Nada no MCP escreve no documento, lê o banco ou toca o filesystem por conta própria. As tools são derivadas do
catálogo (`catalog::ops()` com `surface == Both`), de modo que REST e MCP veem exatamente o mesmo conjunto.
Promoção de memória e undo seletivo continuam fora do catálogo (ação só da UI), logo não existem como tool.

## 2. MCP

### 2.1 Protocolo

Revisão `2025-06-18`; `initialize` aceita também `2025-03-26` e `2024-11-05` (eco da versão pedida se suportada;
senão responde `2025-06-18`). Métodos: `initialize`, `notifications/*` (ignoradas), `ping`, `tools/list`,
`tools/call`, `resources/list`, `resources/templates/list`, `resources/read`. Qualquer outro → `-32601`.
O servidor é **sem sessão** (transporte HTTP sem estado): não exige `initialize` antes de `tools/call`; a
autenticação é por requisição.

Capacidades: `tools {listChanged:false}`, `resources {subscribe:false, listChanged:false}`. Sem `prompts`, `logging`,
`sampling`. Lotes (array) são aceitos (revisão 2025-03-26; até 32 mensagens); lote vazio/grande → `-32600`.

### 2.2 Tools

Nome = `OpDef::tool_name()` (`runs.create` → `runs_create`). `inputSchema` = schema da operação
(`additionalProperties:false` preservado) mais, **só nas mutantes**, `idempotency_key`
(`^[!-~]+$`, 1..128). Anotações:

| campo | regra |
|---|---|
| `readOnlyHint` | `!mutating` |
| `destructiveHint` (só mutantes) | `true` em `tokens_revoke`, `tokens_rotate`, `webhooks_delete`, `uploads_delete`, `runs_cancel`, `exports_cancel`; `false` nas demais (o padrão da spec seria `true`) |
| `idempotentHint` (só mutantes) | `true` onde repetir não tem efeito novo (revoke/delete/update/open/close/pause/cancel) |
| `openWorldHint` | `true` onde há rede externa (`runs_create/resume/variants`, `webhooks_test/redeliver`) |
| `_meta` | `x-capia-operation`, `x-capia-scope`, `x-capia-class`, `x-capia-rest` |

`tools/list` lista **todas** as tools, independente do token (descoberta); o escopo é aplicado na chamada.
`uploads.create` (streaming) não é tool (só REST). Uploads por MCP usam `uploads_create_inline`, limitado pelo
teto de JSON (`max_json_bytes`, 1 MiB por padrão, ~750 KB de arquivo); arquivos grandes sobem pela REST.

### 2.3 `tools/call`

Resultado de sucesso:

```json
{"content":[{"type":"text","text":"<corpo JSON>"}],
 "structuredContent": <corpo da operação>, "isError": false,
 "_meta":{"x-capia-status":200,"x-capia-request-id":"req_…","x-capia-idempotent-replay":true}}
```

Erro **de operação** (escopo, schema, conflito, não encontrado, rate limit…) volta como resultado com
`isError:true` e `structuredContent = {code, message, details?, request_id}` (mensagem redigida pelo redator
central); `_meta["x-capia-status"]` é o status HTTP equivalente e `x-capia-retry-after` quando houver.
Erro **JSON-RPC** só para forma do pedido: ver §2.6.

`idempotency_key` é retirado dos argumentos e vai para `CallCtx.idempotency_key` (com `surface:"mcp"`). A
tabela de idempotência é **a mesma da REST** (chave por token): a mesma chave repete o resultado
semântico independente da porta. Chave inválida → erro de tool `BAD_REQUEST`. Em tool de leitura a chave não
existe no schema → `INVALID_PARAMS`.

### 2.4 Resources (somente leitura)

URIs (tabela fechada; cada uma é uma operação do catálogo, logo com o mesmo escopo e o mesmo gate de projeto):

| URI | operação |
|---|---|
| `capia://projects` | `projects.list` |
| `capia://projects/{p}` | `projects.get` |
| `…/summary` | `projects.summary` |
| `…/sequences` · `…/sequences/{s}` | `sequences.list` · `sequences.get` |
| `…/assets` | `assets.list` |
| `…/runs` · `…/runs/{r}` · `…/runs/{r}/plan` · `…/runs/{r}/review` | `runs.list/get/plan/review` |
| `…/exports` | `exports.list` |

`resources/read` devolve `contents[0].text` = o **mesmo JSON** que a REST devolve (testado por igualdade).
`resources/list` lista só o que o token consegue ler (uma operação negada não contribui), com teto de 200 itens e
sem cursor. Ler não abre projetos: um projeto não aberto devolve conflito estruturado (`PROJECT_NOT_OPEN`).

### 2.5 Mapa de erros (`resources/*` e erros de protocolo)

| situação | código JSON-RPC |
|---|---|
| JSON inválido | `-32700` |
| requisição malformada (jsonrpc, `id`, `method`, lote, aninhamento) | `-32600` |
| método desconhecido | `-32601` |
| `params`/`arguments` com forma errada, tool desconhecida, URI inválido, argumentos grandes demais | `-32602` |
| sem autenticação / token revogado ou expirado / escopo insuficiente em resource | `-32001` |
| resource inexistente (URI fora da tabela, 404) | `-32002` |
| rate limit / servidor encerrando | `-32003` |
| conflito (projeto não aberto, revisão, estado) | `-32004` |
| interno | `-32603` |

`error.data` carrega o envelope `{code,message,details,request_id}` do `ApiErr`. Na HTTP, token ausente/ inválido
é `401` (antes do JSON-RPC), igual à REST.

### 2.6 Segurança

* **Sem confiança implícita.** `handle_rpc` sem `Principal` responde `-32001` a tudo. Nenhum caminho "local = admin".
* **Dados, não instruções.** Strings de tool/resource nunca viram caminho, comando, SQL ou prompt. Ids passam pelo
  schema (`^[A-Za-z0-9._:-]+$`); o URI é casado por tabela e segmentos fora do alfabeto (`%`, `?`, `#`, `/`, `..`,
  espaço, controle) são recusados. Nomes de tool com lixo são ecoados truncados e sem caracteres de controle.
* **Limites.** Corpo HTTP ≤ `max_json_bytes` (413 antes do parse); aninhamento ≤ `max_json_depth` (medido na
  mensagem inteira); `arguments` ≤ `max_json_bytes`; linha stdio ≤ `max_json_bytes`.
* **Notificação nunca executa.** `tools/call` sem `id` não roda (evita efeito colateral sem resposta).
* **Sem mais privilégio que a UI.** Nada de shell/filesystem/HTTP arbitrário/segredo como tool ou resource; o
  teste de catálogo e `tools/list` (testes em `mcp.rs`) garantem ausência de `undo`, `promote`, `shell`, `exec`,
  `credential`, `path`.
* **Escalada.** `tokens_create` por MCP segue a regra do catálogo: nenhum token concede escopo que o chamador não tem.
* **Revogação no meio da sessão.** HTTP reautentica por requisição; stdio revalida o token a **cada mensagem**.

### 2.7 Transportes

* **HTTP:** `POST /mcp` (já roteado em `server.rs`) → `mcp::handle_rpc`. Notificação → `202` sem corpo. Host/CORS/401
  idênticos à REST.
* **stdio:** `capia-server mcp-stdio --data-dir D --token-env VAR`. O segredo é lido **uma vez** da variável,
  registrado no redator e validado contra o `server.db` (falha → saída 1, sem eco do segredo; variável ausente → 2).
  Mensagens JSON-RPC delimitadas por `\n` em stdin; só respostas em stdout; diagnóstico só em stderr. EOF → saída 0.
  O processo abre o `Core` do diretório **sem socket**, com a bomba de eventos e o despachante de webhooks
  (`mcp::Headless`), para que imports, exports, Runs e webhooks progridam. Não rode `serve` e `mcp-stdio` no mesmo
  diretório ao mesmo tempo (um projeto aberto por processo).

## 3. Webhooks

### 3.1 Fluxo

```
Run/export/import ──► pump (events table + deliveries, 1 transação) ──► wake
                                                                         │
 despachante: deliveries_claim_due ─► tarefa por entrega (runtime tokio multi-thread, ≤ 32 em voo)
                                                                         │
 revalida URL ─► lê segredo do cofre ─► assina ─► POST (sem redirect, sem ler corpo) ─► delivery_finish
```

O despachante **só lê o banco** e acorda por `Core::wake`; nunca é chamado do caminho de conclusão de Run/export.
Um endpoint morto produz linhas `retrying`/`dead`, nunca atraso ou falha da Run (testado).

### 3.2 Corpo (versão 1)

```json
{"id":"<event_id>","type":"run.completed","version":1,
 "occurred_at":"2026-10-05T20:00:00.123Z","occurred_ms":1791241200123,
 "project_id":"prj_…","run_id":"run-…","export_id":null,"data":{…}}
```

Tipos: `run.started`, `run.waiting_user`, `run.completed`, `run.failed`, `run.cancelled`, `export.completed`,
`export.failed`, `asset.imported`, `webhook.test` (assinar `*` recebe todos). As chaves saem ordenadas: os bytes do
corpo são estáveis entre tentativas. `Content-Type: application/json`; `User-Agent: CapIA-Webhook/<versão>`.

### 3.3 Cabeçalhos e assinatura

| cabeçalho | valor |
|---|---|
| `X-CapIA-Timestamp` | segundos Unix da **tentativa** |
| `X-CapIA-Signature` | `v1=<hex>` = HMAC-SHA256(segredo, `"<timestamp>.<corpo cru>"`) |
| `X-CapIA-Event-Id` | `event_id` (igual a `id` do corpo; **estável entre tentativas**) |
| `X-CapIA-Delivery` | id da linha de entrega (estável entre tentativas) |
| `X-CapIA-Attempt` | 1, 2, … (volta a 1 numa reentrega manual) |

### 3.4 Algoritmo de verificação do receptor

1. Ler o corpo **cru** (antes de qualquer parse/re-serialização).
2. `ts = X-CapIA-Timestamp`; se `|agora − ts| > 300 s` → rejeitar (replay/relógio).
3. `esperado = hex(HMAC_SHA256(segredo, ts + "." + corpo))`.
4. Comparar com `X-CapIA-Signature` (`v1=…`, várias separadas por vírgula numa rotação) **em tempo constante**.
5. Deduplicar por `X-CapIA-Event-Id` (guardar ids vistos pelo menos pela janela de replay + retries): a entrega é
   *at-least-once*, e um replay **dentro** da janela passa na assinatura.
6. Responder `2xx` rápido (o corpo da resposta é ignorado). `4xx` (exceto 408/425/429) encerra as tentativas.

Rust: `capia_server::webhooks::{sign, verify, REPLAY_TOLERANCE_SECS, VerifyError}` (`verify(secret, ts, body,
header, now, tolerance)` → `Malformed | TimestampOutsideTolerance | SignatureMismatch`).
Node: `crypto.createHmac("sha256", secret).update(`${ts}.`).update(rawBody).digest("hex")` e `timingSafeEqual`.
Python: `hmac.new(secret.encode(), f"{ts}.".encode()+raw, "sha256").hexdigest()` e `hmac.compare_digest`.

### 3.5 Semântica de entrega

* **At-least-once.** Só `2xx` → `delivered`. Queda no meio da tentativa deixa `delivering`; a reabertura do servidor
  (`deliveries_recover`) a devolve a `retrying` e o evento é reenviado com os mesmos `Event-Id`/`Delivery`.
* **Unicidade.** `UNIQUE(webhook_id, event_id)` no banco: o mesmo evento nunca vira duas entregas do mesmo webhook
  (a bomba usa `event_id` determinístico, então reprocessar não duplica).
* **Resultados.**

| resposta | efeito |
|---|---|
| 2xx | `delivered` |
| 3xx | `dead` imediato ("redirects are never followed"); nunca segue |
| 4xx exceto 408/425/429 | `dead` imediato ("permanent client error") |
| 408/425/429, 5xx, timeout, erro de conexão | `retrying`, `base·2^(n−1)` limitado por `webhook_backoff_cap`, jitter ±20% (nunca acima do teto) |
| tentativa `n ≥ webhook_max_attempts` | `dead` ("gave up after n attempts: …") — dead-letter |

* **`dead` é reabrível:** `POST /v1/webhooks/{id}/deliveries/{delivery_id}/redeliver` (zera a contagem).
* **Falhas sem rede:** webhook desabilitado → `dead` "webhook disabled"; URL que deixou de passar na política →
  `dead` `URL_REJECTED: …`; segredo ausente do cofre (processo reiniciado sem cofre do SO) → `dead`
  `SECRET_UNAVAILABLE: …`; `rotate-secret` reenfileira essas entregas (`requeued`).
* **URL revalidada a cada tentativa** (política + resolvedor filtrado do `WebhookClient`: sem rede
  privada/link-local/metadata, `https` ou `http` em loopback). O corpo da resposta nunca é lido. Erros gravados
  passam pelo redator; o segredo só existe no `SecretStore` (`capia/server/webhook/<id>`).
* **Concorrência:** runtime multi-thread (4 threads), no máximo 32 entregas em voo; um endpoint lento ocupa uma vaga,
  não a fila. Prazo por tentativa = `WebhookPolicy.total_timeout`.
* **Shutdown:** o laço para em `shutting_down`, espera até 3 s pelas tentativas em voo e abandona o resto (recuperável).
* **Log de entrega:** `id, webhook_id, event_seq, event_id, attempt, state, next_attempt_ms, last_status,
  last_latency_ms, last_error, created_ms, updated_ms` (`GET /v1/webhooks/{id}/deliveries`, paginado por `after`).
* **Teste:** `POST /v1/webhooks/{id}/test` enfileira `webhook.test` (inclusive para webhook desabilitado). O harness
  de teste é `tests/common/receiver.rs` (processo de teste, loopback); não existe serviço de eco público.

## 4. Suítes de teste

| arquivo | cobre |
|---|---|
| `tests/webhooks.rs` | registro/SSRF, assinatura verificada de forma independente, backoff, 4xx permanente, dead-letter + reentrega, endpoint lento, redirect, duplicação/roteamento, webhook desabilitado, segredo indisponível + rotação, revalidação de URL, shutdown no meio da entrega, forja/replay/adulteração no receptor |
| `tests/mcp.rs` | handshake, paridade tools↔catálogo (schemas, escopos, anotações), autenticação/revogação, escopo e escalada, fuzz de forma/tamanho/aninhamento, idempotência, injeção, resources↔REST, stdio (processo filho) |
| `tests/parity.rs` | mesmo roteiro por `Core::call`, REST e MCP em diretórios novos; compara revisão, sequences/grafo, histórico (ator `api`), idempotência, escopo, códigos de erro e contagens de auditoria |
| `tests/external_flow.rs` | fluxo canônico REST e MCP com cérebro demo: projeto → upload raw+referência+briefing → import → Run (aprovações spec/plano) → variantes → export → webhooks assinados → estado relido; resultado semanticamente igual e idêntico ao que `Session` (ao vivo e arquivo reaberto) enxerga; endpoint morto não bloqueia a Run |

O export H.264 exige encoder **aprovado e disponível** (`export.encoders`). Sem ele (ambiente Linux sem NVENC/QSV
etc.) o passo `h264-mp4` é pulado **com mensagem explícita** e o fluxo exercita o preset `intermediate`; onde
houver encoder aprovado (CI Windows) o fluxo exporta `h264-mp4` e confere o codec por `ffprobe`. Sem FFmpeg os
testes de fluxo pulam com aviso, ou falham com `CAPIA_REQUIRE_FFMPEG=1`.

## 5. Limitações conhecidas

* `resources/list` não pagina (máx. 200 itens); `resources/subscribe` e notificações de lista não existem.
* Uploads grandes não cabem em `uploads_create_inline` (teto de JSON): usar REST streaming.
* O MCP stdio e o `serve` não devem compartilhar o mesmo diretório simultaneamente.
* A deduplicação de replay *dentro* da janela de 300 s é responsabilidade do receptor (por `Event-Id`).
