# Plano de adoção — decisão `REJECT_TIMELINE_STUDIO_TEST_OPENCUT`

> Não adotamos o Timeline Studio. Este plano descreve (a) o que levar dele como ideia e (b) a próxima fase de teste do OpenCut classic. **Nada foi implementado; `main` não foi alterado.**

## Branches (a criar só após a próxima fase, se OpenCut classic passar)

- `main` — CapIA atual, intocado.
- `legacy/core-from-scratch` — referência congelada do núcleo Rust (M01–M05 e Fases 2–6) como requisitos/arquitetura.
- `integration/opencut-classic` — fork com `origin` = CapIA e `upstream` = OpenCut-app/opencut-classic. Sem migração destrutiva.

## Próxima fase (Fase 1b) — teste funcional do OpenCut classic

1. Auditar licenças (dependências bun/cargo, fontes, sons, modelos).
2. Corrigir/entender as 5 falhas de `bun test` (init do `opencut-wasm`); build do Rust/WASM; `next build`.
3. Rodar a UI e repetir a bateria A–Z (mesmos itens do `timeline-studio-test-results.md`), persistência, export, desempenho 100…10k clips.
4. Medir gap de multi-sequence (PROJECT/cenas → sequências aninhadas).
5. Decisão final: ADOPT_OPENCUT_CLASSIC(_WITH_MAJOR_CHANGES) ou REJECT_CURRENT_CANDIDATES.

## Ideias do Timeline Studio a portar (PORT_IDEA, reimplementação própria; sem copiar código)

- Plano de comandos com `baseRevision`, `operationId` idempotente, validação → preview → diff → apply atômico (o CapIA já tem equivalente: `preview → apply_plan`).
- Superfície WebMCP (tools sobre o estado vivo, `stateToken`/`previewId`/`transactionId`, recibo de undo) e histórico compartilhado humano/IA.
- Rejeição explícita de edição insegura (`BROWSER_EDIT_COMPLEX_TIMING`) como padrão de erro estruturado.

## KEEP / MODIFY / WRAP / ADD / REMOVE (se OpenCut classic for adotado)

- UPSTREAM KEEP: timeline, snapping, ripple, retime/speed, keyframes, masks/effects, renderer, migrações de storage.
- UPSTREAM MODIFY: tempo `MediaTime`→compatível com `Ticks` do CapIA (705 600 000/s); persistência → `.capia` com mídia fora do arquivo; histórico → Command Engine único; scenes → sequências do PROJECT.
- UPSTREAM REMOVE: auth/blog/changelog/feedback/analytics e qualquer serviço remoto não necessário (local-first).
- CAPIA ADD: Command Engine como porta única, sequências aninhadas, `capia-ai`/`capia-intelligence` (Demand Interpreter, Planner, Editor, Critic, memória, gateway), REST/MCP/webhooks, jobs, empacotamento desktop.
- Ordem de integração: licenças → build/teste → camada de comandos única → persistência → multi-sequence → agente (cliente do Engine API) → desktop.
