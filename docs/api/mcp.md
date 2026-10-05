# MCP (Model Context Protocol)

O servidor MCP do CapIA é um **adaptador sobre o mesmo catálogo de operações e as mesmas facades da REST** — não é um segundo backend. Um agente externo (por exemplo, um cliente que fala MCP) vê as mesmas operações, com os mesmos scopes, validações, travas e erros que um cliente REST.

> **Estado.** O contrato (nomes de tools, schemas, scopes, efeitos) vem do catálogo e está estável. Os **comandos de inicialização** (`capia-server mcp-stdio …`, `capia-server serve …`) e o caminho do endpoint HTTP são **interface prevista**: confirme com `capia-server --help` na sua versão. Os exemplos em `examples/mcp/` foram testados só contra um servidor MCP falso.

## Regra de nomes

`<grupo>.<ação>` do catálogo vira `<grupo>_<ação>`: `runs.create` → `runs_create`, `uploads.create_inline` → `uploads_create_inline`, `webhooks.rotate_secret` → `webhooks_rotate_secret`. A lista completa (gerada do catálogo) está em [mcp-tools.md](mcp-tools.md): **55 tools**. A única operação sem tool é `uploads.create` (upload em streaming, que o MCP não transporta): use `uploads_create_inline` (conteúdo em base64, para arquivos pequenos) ou envie o arquivo por REST e importe por `assets_import`.

Cada tool carrega:

- **`inputSchema`:** o JSON Schema da operação do catálogo (os mesmos limites e `additionalProperties: false` da REST). Argumentos inválidos → erro de validação, sem efeito.
- **Metadados de efeito colateral** (interface prevista no formato MCP `annotations`): `readOnlyHint = true` para leituras (`mutating: false` no catálogo), `false` para mutantes; mais o scope exigido e a classe de rate limit. Ferramentas mutantes que gastam dinheiro ou mexem na timeline (`runs_create`, `runs_approve`, `exports_start`, `commands_apply`) são sempre auditadas.
- **Idempotência:** como na REST, as tools mutantes aceitam uma chave de idempotência (interface prevista: argumento `idempotency_key`); veja [README.md](README.md#5-idempotência-idempotency-key).

## O que o MCP **não** expõe

Por construção (as tools simplesmente não existem): shell, HTTP arbitrário, leitura/escrita arbitrária do sistema de arquivos, leitura de configurações ou de **valores de segredos** (chaves de provider, segredos de webhook depois da criação). Texto vindo do cliente é tratado como dado: um argumento de tool que diga “ignore as regras” não muda nenhuma permissão, e conteúdo de documento/transcrição que chega a uma Run entra como `untrusted_data`.

## Autenticação e scopes

- **stdio:** o cliente MCP lança o processo; o token vem de uma variável de ambiente nomeada por `--token-env` (nunca como argumento de linha de comando, que aparece na lista de processos).
- **HTTP:** `Authorization: Bearer <token>` em cada requisição, em loopback.
- Os **mesmos scopes** da REST valem por tool ([auth-and-scopes.md](auth-and-scopes.md)). Não existe confiança implícita de admin por ser local: um token só com `project:read` + `run:read` vê, mas não inicia Runs nem aprova. As tools `tokens_*` e `audit_list` exigem `admin:tokens` — **não** dê esse scope a um agente de IA.
- Um agente que inicia Runs **não** deve ter `run:approve`: assim as decisões de gasto, geração e licença ficam com um humano ([../user/05-aprovacoes-e-orcamentos.md](../user/05-aprovacoes-e-orcamentos.md)).

## Resources (interface prevista)

Leituras também ficam disponíveis como resources `capia://…`, para clientes que preferem anexar contexto a chamar tools:

| URI | Conteúdo | Equivale a |
|---|---|---|
| `capia://projects` | projetos do servidor | `projects.list` |
| `capia://projects/{project_id}/summary` | resumo do projeto | `projects.summary` |
| `capia://projects/{project_id}/assets` | assets importados | `assets.list` |
| `capia://projects/{project_id}/sequences` | sequences | `sequences.list` |
| `capia://projects/{project_id}/runs/{run_id}` | status da Run | `runs.get` |
| `capia://projects/{project_id}/runs/{run_id}/plan` | plano da Run | `runs.plan` |
| `capia://projects/{project_id}/runs/{run_id}/review` | revisão do Critic | `runs.review` |

Resources respeitam os mesmos scopes (`project:read`, `media:read`, `run:read`). O conteúdo é JSON e idêntico ao da tool equivalente.

## Falhas

| Situação | Como aparece |
|---|---|
| JSON-RPC malformado, método desconhecido | erro JSON-RPC (`-32700`, `-32601`…) |
| Tool inexistente ou argumento fora do schema | erro de parâmetros; nada é executado |
| Token ausente/inválido (HTTP) | `401` antes de qualquer JSON-RPC |
| Scope insuficiente | resultado de tool com `isError: true` e o envelope `{code: "PERMISSION_DENIED", message, request_id}` |
| Erro de negócio (revisão desatualizada, Run ocupada, IA desligada…) | resultado de tool com `isError: true` e **o mesmo envelope e `code` da REST** (`REVISION_CONFLICT`, `RUN_BUSY`, `AI_OFF`…) |
| Rate limit | `isError: true` com `code` de limite e tempo para repetir |

O agente deve ler o `code` e decidir: `REVISION_CONFLICT`/`STALE_PLAN` → refazer a prévia; `waiting_user` (status da Run) → pedir a um humano; `AI_OFF` → informar que a IA está desligada (o editor segue funcionando).

## Paridade REST ↔ MCP ↔ UI

A paridade é **por construção**: REST, MCP e OpenAPI derivam do mesmo catálogo, e um teste de arquitetura do servidor garante que os dois transportes veem o mesmo conjunto de operações (exceto o upload em streaming). Para cada tool mutante, a suíte de conformidade da Fase 6 compara **efeito** (revisão do projeto, grafo de clips, variantes, registros de Run, sondagem do export) — não texto de resposta. O resultado dessa suíte, quando executada contra o servidor real, é parte do item `external-flow` de `tools/phase6-acceptance/`; nesta versão da documentação ela **não** foi executada.

## Configurar um cliente

`examples/mcp/` tem as configurações e um cliente Node mínimo. Resumo:

**stdio** (o cliente lança o servidor) — interface prevista:

```json
{
  "mcpServers": {
    "capia": {
      "command": "capia-server",
      "args": ["mcp-stdio", "--data-dir", "<DATA_DIR>", "--token-env", "CAPIA_TOKEN"],
      "env": { "CAPIA_TOKEN": "capia_REDACTED" }
    }
  }
}
```

**HTTP** (servidor já em execução) — interface prevista:

```
capia-server serve --data-dir <DATA_DIR> [--port N]     # 127.0.0.1 por padrão
```

```json
{
  "mcpServers": {
    "capia": {
      "type": "http",
      "url": "http://127.0.0.1:<PORT>/mcp",
      "headers": { "Authorization": "Bearer capia_REDACTED" }
    }
  }
}
```

O token é **placeholder**: crie o seu com `tokens_create`/REST (com o menor conjunto de scopes) e injete por variável de ambiente.

### Exemplo: iniciar uma Run e consultar o status

```
node examples/mcp/stdio-client.mjs --data-dir <DATA_DIR> --run \
  --project <project_id> --brief "Produto… Público… Oferta… CTA…" --assets ast_1,ast_2
```

Equivale a `runs_create` (`start: true`) seguido de `runs_get` em laço até `completed|failed|cancelled` ou `waiting_user`. O script **não aprova** nada: a aprovação é `runs_approve` com um token que tenha `run:approve`.
