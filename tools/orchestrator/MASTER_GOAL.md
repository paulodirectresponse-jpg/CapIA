# MASTER GOAL — CapIA (edite antes de rodar; o orquestrador pede confirmação na 1ª execução)

## Objetivo
Concluir, em ordem e **um item por iteração**, os itens da Fase 2B que **não dependem da sua máquina física**, listados em `docs/adoption/PHASE_2B.md` (seção "O que falta"):
1. UI do opt-in de fontes remotas e do aviso/opt-in de fallback AAC→Opus no export.
2. Comandos Tauri `project_open`/`project_save` no host Rust usando o formato `capia.project` (ADR-132), com escrita atômica (temp + fsync + rename) e testes.
3. Hospedar o editor (`apps/editor`) no shell desktop, sem criar uma segunda timeline.

## Critérios de aceitação (o auditor só dá PASS se todos forem comprovados)
- Cada item entregue com testes automatizados que falham sem a mudança.
- `bun test` (em `apps/editor`) e `tsc --noEmit` limpos; `cargo test -p <crates tocados>` e `cargo clippy ... -D warnings` limpos quando houver Rust.
- `node tools/check-architecture.mjs` e `node tools/check-editor-architecture.mjs` passando.
- Documentação atualizada (`docs/adoption/PHASE_2B.md`) com o que foi feito **e o que não foi verificado**.

## Fora do escopo / proibido
- Alterar `main`, abrir PR, fazer push, mexer em CI de outros workflows.
- Declarar resultado de GPU física ou de aceitação humana: isso exige o usuário (`HUMAN_GATE`).
- Multi-sequence, transições, REST/MCP, branding.

## Princípios do projeto (de `CLAUDE.md`)
Timeline é a fonte de verdade; uma única porta de escrita (AgentSurface/Command Engine); tempo é inteiro; segredos nunca saem do core; local-first; nunca fabricar resultados.
