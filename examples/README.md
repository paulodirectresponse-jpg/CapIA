# Exemplos de integração

**Somente código de exemplo.** Nada aqui é código de produto, nenhum crate/pacote depende destes arquivos, e nenhum token real deve entrar neles (use `capia_REDACTED` e variáveis de ambiente).

| Pasta | O que é | Como testar |
|---|---|---|
| `rest/` | fluxo canônico por REST: `curl.sh`, `powershell.ps1`, `client.mjs` (Node ≥ 22), `client.py` (Python ≥ 3.9) | `client.mjs` e `client.py` têm testes contra um servidor **falso** em memória (`client.test.mjs`, `test_client.py`) |
| `mcp/` | configuração de cliente MCP (stdio e HTTP) e um cliente JSON-RPC mínimo em Node | `stdio-client.test.mjs` usa um servidor MCP falso (`fake-mcp-server.mjs`) |
| `webhook-receiver/` | receptores locais que **verificam a assinatura** (HMAC-SHA256 sobre `timestamp.corpo`, tempo constante, janela de 5 min, de-duplicação por event id): `receiver.mjs`, `receiver.py` | vetores calculados de forma independente; `node --test` e `unittest` |

Os testes rodam em `pnpm test:tools` (os de Python são **pulados com aviso** se `python3` não existir). Lint/format: os `.mjs` entram no ESLint/Prettier do repositório (override de scripts Node em `eslint.config.js`); `.py`, `.sh` e `.ps1` não são cobertos por essas ferramentas.

**Honestidade sobre o que foi verificado:** estes exemplos foram exercitados **apenas contra servidores falsos** escritos para os testes. Eles seguem o catálogo de operações (`docs/api/rest-reference.md`) e a forma de resposta observada nos serviços da Engine API/IA, mas **não foram executados contra o `capia-server` real** (o servidor é implementado por outra frente da Fase 6). O teste de integração real é o item `external-flow` de `tools/phase6-acceptance/`.
