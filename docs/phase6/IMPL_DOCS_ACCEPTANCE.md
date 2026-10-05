# Fase 6 — Track E: documentação, aceitação e release (implementação)

Registro do que a frente de **documentação / pacote de aceitação / processo de release** entregou, do que foi verificado e dos gates **externos** que continuam abertos. Não declara a Fase 6 completa: o estado final é decidido pelo agregador (`node tools/phase6-acceptance/run-all.mjs`) e pelo `docs/STATUS.md` do integrador.

## O que existe

| Área | Arquivos | Verificação |
|---|---|---|
| Docs do usuário (pt-BR) | `docs/user/README.md`, `01`…`09` | conferidos contra strings da UI (`ptBR.ts`), registry de providers, atalhos (`keymap.ts`), CLI e config do `capia-server`; `pnpm format:check` ignora `*.md` |
| Docs da API | `docs/api/{README,auth-and-scopes,rest-reference,mcp,mcp-tools,webhooks,security,canonical-flow}.md`, `openapi.json` | `rest-reference`, `mcp-tools`, `openapi.json` e a matriz de scopes são **gerados** do catálogo; `node tools/docs/gen-api-docs.mjs --check` |
| Gerador | `tools/docs/gen-api-docs.mjs` + `fixtures/catalog.json` (58 operações, extraído do `catalog.rs` por um programa Rust descartável, não por regex) | 15 testes Node; `--catalog -` aceita a saída de `capia-server catalog` |
| Exemplos | `examples/rest/{curl.sh,powershell.ps1,client.mjs,client.py}`, `examples/mcp/`, `examples/webhook-receiver/{receiver.mjs,receiver.py}` | clientes e MCP contra **servidores falsos**; receptores com vetores HMAC calculados de forma independente (Python `hmac`); `node --test` e `unittest` (via `pnpm test:tools`; Python é pulado com aviso se ausente); `.mjs` no ESLint/Prettier |
| Aceitação | `tools/phase6-acceptance/` (`external-flow`, `installer`, `update`, `security`, `clean-machine`, `beta-feedback`, `steps-runner.mjs`, `evidence-lib.mjs`, `run-all.mjs`) | testes Node para runner, agregador e cada validador (aceita/rejeita/parcial/pendente) |
| Release | `docs/RELEASE.md`, `CHANGELOG.md`, `docs/KNOWN_ISSUES.md`, `docs/MIGRATION_COMPAT.md` | `installer/check-release.mjs --docs|--versions` |
| Projeto de exemplo | `tools/sample-project/` | partes puras testadas; geração real exige ffmpeg |
| ADR (rascunho) | `docs/phase6/adr-drafts/track-e.md` | para o integrador incorporar a `docs/DECISIONS.md` |

## O que foi realmente verificado vs. o que não foi

- **Verificado (CI/`pnpm test:tools`)**: geradores e validadores; assinatura/replay/dedupe dos receptores; lógica dos clientes de exemplo contra servidores falsos; que cada passo externo sem evidência vira `pending_external` e que requisito ausente vira `not_available` (nunca `passed`).
- **Lido no código do servidor** (branch de integração) para escrever a documentação: catálogo, códigos de idempotência (`IDEMPOTENCY_KEY_REUSED` 422, `…_IN_PROGRESS`/`…_INDETERMINATE` 409), `X-Request-Id`, `Idempotent-Replay`, `INSUFFICIENT_SCOPE`, SSE `GET /v1/events/stream`, `POST /mcp`, CLI (`serve`, `stop`, `token`, `catalog`, `openapi`).
- **Não verificado**: nenhum exemplo foi executado contra o `capia-server` real; `capia-server mcp-stdio` respondia “not implemented yet” no snapshot lido; cabeçalhos/timestamp exatos e número de tentativas dos webhooks são **interface prevista**; o script PowerShell do checklist de máquina limpa e os `.ps1`/`.sh` de exemplo não foram executados (sem `pwsh`/Windows aqui; `bash -n` passou).

## Interfaces marcadas “prevista”

`capia-server mcp-stdio` (flags), resources MCP `capia://…`, anotações MCP de efeito e argumento `idempotency_key` das tools; formato exato de `X-CapIA-Timestamp` (segundos × ms — os receptores aceitam ambos), ids de entrega e `data` por tipo de evento; número máximo de tentativas de webhook; instalador, atualizador, pacote de diagnóstico e relatório de falhas opt-in (descritos como Fase 6 nos guias do usuário); passos `steps.json` que apontam para `crates/capia-server/tests/{conformance,security,fuzz}.rs` e `apps/desktop/src-tauri/src/updater.rs` (nomes esperados).

## Gates externos exatos (bloqueiam `PHASE 6 COMPLETE`)

1. **Certificado de assinatura de código real** + verificação Authenticode (instalador, atualizador, executáveis). Harness: `installer/`.
2. **Windows 10 22H2 e Windows 11 físicos e limpos** (sem ferramentas de dev). Harness: `clean-machine/`.
3. **Atualização assinada RC→RC e cenários de rollback** em Windows real. Harness: `update/`.
4. **Beta com usuários reais** (mínimo documentado: 5 externos) e gate sem Blocker/Critical aberto. Harness: `beta-feedback/`.
5. **Decisão de produto/jurídica sobre H.264/AAC** (patentes, OpenH264, qualidade).
6. **Paridade UI×REST×MCP e fluxo canônico contra o servidor real.** Harness: `external-flow/`.
7. **Pentest independente** da API local. Harness: `security/`.
8. Revisão do **pacote de licenças** (FFmpeg LGPL, avisos de terceiros) no instalador final.
9. Documentação seguida **do zero por um testador novo** (PHASE6_BETA §20); acessibilidade/localização revisadas por humano.
10. **Pendências herdadas:** Fase 3 (≥ 3 usuários reais, residual de GPU/P2), Fase 4 (LLM real no DemandSpec, corpus real de cenas, STT real), Fase 5 (≥ 10 demandas reais avaliadas, providers/chaves reais, Critic com visão real). Ver `docs/KNOWN_ISSUES.md`.

## Arquivos compartilhados editados

`package.json` (scripts `test:tools` ampliado, `docs:api`, `check:docs`), `eslint.config.js` (o override de scripts Node passa a cobrir `examples/**/*.mjs`), `.prettierignore` (`tools/docs/fixtures/`). Nenhum arquivo de `crates/`, `apps/`, `packages/`, `.github/`, `docs/STATUS.md`, `docs/ROADMAP.md`, `CLAUDE.md`, `docs/DECISIONS.md` ou `tools/check-architecture.mjs` foi alterado.
