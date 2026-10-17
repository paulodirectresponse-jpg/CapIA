# Prompt único de instalação (cole no Claude Code, na raiz do repositório CapIA)

> Instale e valide o orquestrador Claude ⇄ Codex deste repositório. Não inicie o laço autônomo.
>
> 1. Confirme que estou numa branch que NÃO é `main`/`master` (`git branch --show-current`); se estiver, pare e avise.
> 2. Rode `node tools/orchestrator/orchestrate.mjs init`.
> 3. Rode `node tools/orchestrator/orchestrate.mjs doctor`. Se faltar o Codex CLI, mostre o comando de correção (`npm i -g @openai/codex` e `codex login`) e **pare para eu fazer o login** — não invente credenciais.
> 4. Rode `node tools/orchestrator/orchestrate.mjs dry-run` e `node --test tools/orchestrator/orchestrator.test.mjs`; ambos precisam passar 100%.
> 5. Rode `node tools/orchestrator/orchestrate.mjs selftest` (Claude → Codex → Claude). Se ficar `BLOCKED`, explique o bloqueio; só `--mock-codex` NÃO conta como prova do Codex.
> 6. Mostre o `MASTER_GOAL.md` e os limites de `orchestrator.config.json` e me pergunte se estão corretos.
> 7. Entregue UM comando para iniciar: `node tools/orchestrator/orchestrate.mjs run`.
> Responda só com: o que passou, o que está bloqueado e o comando final.
