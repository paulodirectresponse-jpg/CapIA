# Orquestrador Claude ⇄ Codex

Laço autônomo **executor (Claude Code)** → **auditor independente (Codex)** → executor corrige → … até o auditor dar `PASS`
ou um *hard stop* parar tudo. Sem dependências (Node ≥ 20), Windows/macOS/Linux.

```
init → doctor → dry-run → selftest → run
```

| Comando | O que faz |
|---|---|
| `init` | cria `.orchestrator/` (STATE.json, logs, runs), valida arquivos, põe `.orchestrator/` no `.gitignore`. Idempotente. |
| `doctor` | Claude CLI + login, Codex CLI + login, git, branch não protegida — cada falha traz **como consertar**. |
| `dry-run` | a máquina de estados **real** com agentes simulados e falhas injetadas (13 cenários). Não chama CLI nenhum. |
| `selftest` | cadeia **real** Claude → Codex → Claude passando um nonce. `--mock-codex` simula só a perna do Codex. |
| `run` | inicia/retoma o laço. 1ª vez pede para você confirmar o `MASTER_GOAL.md` (digite `SIM`, ou `--yes`). |
| `status` / `stop` / `reset` | progresso e contadores · cria `STOP` (para antes do próximo passo) · arquiva o estado. |

**Um comando para iniciar:** `node tools/orchestrator/orchestrate.mjs run` (ou `pnpm orchestrate run`).

## Arquivos
- `MASTER_GOAL.md` — objetivo e critérios de aceitação (**edite**). Mudou o texto ⇒ nova confirmação.
- `prompts/executor.md`, `prompts/auditor.md` — papéis. O auditor é somente leitura e tenta *refutar*; só dá `PASS` com evidência.
- `orchestrator.config.json` — limites, retry, agentes por papel, ferramentas permitidas.
- `.orchestrator/` (local, ignorado pelo git): `STATE.json`, `logs/orchestrator.jsonl`, `runs/iter-NN/*` (prompt + saída de cada chamada, já com segredos redigidos).

## Retry e recuperação
- Falha transitória (429, 5xx, timeout, rede) → backoff exponencial com jitter, até `retry.maxAttempts`.
- Login/crédito ausente → **não** repete: pausa (`paused_auth`) e diz o que rodar.
- Saída inválida do auditor → uma nova tentativa com aviso de reparo no prompt.
- Crash/`kill`/Ctrl-C: `STATE.json` é gravado de forma atômica; rodar `run` de novo retoma de onde parou **sem refazer** passos concluídos.
- Timeout mata a **árvore** de processos (o `codex` é um wrapper que lança um binário filho).

## Hard stops (qualquer um para o laço)
`maxIterations` · `maxWallClockMinutes` · `maxCostUsd` (custo real reportado pelo Claude) · `maxClaudeCalls` · `maxCodexCalls` ·
`maxConsecutiveFailures` · **sem progresso** (mesmos achados `maxNoProgress` vezes) · **branch protegida** (`main`/`master`) ·
arquivo `STOP` · **gate humano**: se o executor escrever `HUMAN_GATE: <comando>`, o laço pausa (nunca se fabrica resultado que
exige humano/máquina física). Depois de fazer: escreva o resultado em `.orchestrator/human-response.md` e rode `run --human-done`.

Códigos de saída: `0` done · `10` gate humano · `11` auth/blocked · `12` stopped · `20` hard stop · `30` failed.

## Segurança
Sem `--dangerously-skip-permissions`: o executor roda com `permissionMode` + `allowedTools` da config (sem `git push`, sem `rm`).
Claude como auditor recebe só ferramentas de leitura. Segredos são redigidos de logs, artefatos e STATE.json.

## Testes
`node --test tools/orchestrator/orchestrator.test.mjs`
