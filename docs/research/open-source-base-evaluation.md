# Avaliação de base open-source para o CapIA — Fase 1

> Triagem técnica (não é parecer jurídico). Evidência > código > docs > README.
> Branch: `research/open-source-base-evaluation` (a partir de `main` 65b2b87). Nenhum código de terceiros foi importado; clones separados fora do repositório.
> Resultados detalhados do candidato principal: [`timeline-studio-test-results.md`](timeline-studio-test-results.md). Plano: [`capia-adoption-plan.md`](capia-adoption-plan.md).

## 1. Candidatos e versões

| Candidato | URL | Branch | SHA | Licença do código |
|---|---|---|---|---|
| Timeline Studio (principal) | github.com/MartinDelophy/ai-video-editor | main | `36cf7b9` | MIT |
| OpenCut classic (secundário) | github.com/OpenCut-app/opencut-classic | main | `cf5e79e` | MIT |
| OpenCut novo (referência) | github.com/OpenCut-app/OpenCut | main | `e668010` | MIT |

Runtime: Node 22 + npm (Timeline Studio), bun + cargo (OpenCut). Data: 2026-10-13.

## 2. Veredito de licença

- **Timeline Studio — LEGAL_YELLOW.** Código MIT; porém `@ffmpeg/core` (GPL, com x264), render headless via `libx264` do ffmpeg do sistema e modelos de IA locais com licenças variadas (`MODEL_LICENSES.md`, incluindo termos não comerciais) conflitam com ADR-032 e com uso comercial. Remediável removendo/substituindo, mas exige auditoria por modelo.
- **OpenCut classic — LEGAL_GREEN provisório.** LICENSE MIT na raiz (único arquivo LICENSE do repositório). Auditoria das 1 951 dependências bun/cargo, fontes, sons e modelos **não foi feita** (UNVERIFIED).
- **OpenCut novo — LEGAL_GREEN provisório** (MIT), mesma ressalva.

## 3. Execução real vs. análise estática

| Candidato | Real (executado) | Estático |
|---|---|---|
| Timeline Studio | install, lint, typecheck, build, UI Playwright, WebMCP, CLI, MCP stdio, persistência, render, perf motor+UI | arquitetura de módulos, multi-sequence (grep), skill |
| OpenCut classic | `bun install` (1 951 pacotes, 10 s), `bun test`: **150 testes em 30 arquivos, 145 pass / 5 fail** (as 5 falhas: `wasm.__wbindgen_start is not a function` ao carregar `opencut-wasm` sob bun — dependência WASM não inicializada no ambiente de teste; **não** foram investigadas como defeito do upstream) | estrutura (`commands/`, `timeline/`, `retime`, `ripple`, `masks`, `effects`, `animation`, migrações v1→v21 em `services/storage/migrations`), GPU renderer, `apps/desktop` (GPUI/Rust) |
| OpenCut novo | leitura do README/estrutura | tudo; **quase nada implementado** (reescrita em andamento) |

Não foi executado: `tsc` limpo no OpenCut classic (erros de tipo só por `bun:test`), build Next, UI do OpenCut classic em navegador, export. **O teste funcional A–Z do OpenCut classic é UNVERIFIED_ON_CURRENT_ENVIRONMENT** — é o primeiro item da próxima fase.

## 4. Mapa de arquitetura

**Timeline Studio:** UI React → estado em `App.jsx` (coleções fixas de segmentos por faixa) → `editorHistoryCore` (snapshot do estado inteiro) → `projectCommandEngine` (puro, usado por CLI/MCP/WebMCP) → mídia (WebCodecs/mediabunny/libav/ffmpeg.wasm) → preview (canvas/DOM) → export (determinístico offline / MediaRecorder / ffmpeg headless) → persistência (zip `.timeline`). Verdade do estado: React (+ cópia no engine). Tempo: **float de segundos**. Undo: **snapshot**. Asset id: UUID por importação. Paridade preview/export: UNVERIFIED (caminhos distintos: DOM/canvas × ffmpeg/WebCodecs).

**OpenCut classic:** Next.js (`apps/web`) + Rust/WASM (`opencut-wasm`) + GPU renderer (`services/renderer`, `gpu-renderer.ts`); `commands/` (classes `Command` com undo, `batch-command`), `timeline/` (placement, snapping, group-move/resize, ripple, retime, scenes), tempo **`MediaTime` inteiro (ticks)**, trilhas tipadas (`video|text|audio|graphic|effect`) em lista livre, cenas (várias, **sem aninhamento**), storage com migrações versionadas e testadas (v1→v21), `apps/desktop` GPUI. **Sem camada de agente/MCP/CLI.**

## 5. Gap de multi-sequence

- Timeline Studio: **ARQUITETURAL** (modelo de documento = um conjunto fixo de faixas; reducers, UI, arquivo, histórico e agente assumem isso).
- OpenCut classic: **MEDIUM** para PROJECT com várias sequências (já há `scenes`), **LARGE** para sequências aninhadas (clip referenciando sequência, render recursivo, invalidação de cache).

## 6. Edição por referência

Timeline Studio: só o script de diagnóstico é código; o restante é instrução de agente/SKILL. OpenCut: nada. O Reference Analyzer do CapIA (Fase 4) já supera ambos.

## 7. Desktop

Timeline Studio: navegador/PWA (opção A viável; B = Tauri/Electron futuro). OpenCut classic: web + `apps/desktop` GPUI nativo (caminho diferente de Tauri; avaliar fora desta fase).

## 8. Matriz de requisitos CapIA

Ações: KEEP, WRAP, MODIFY, PORT_IDEA, ADD, REPLACE, NOT_NEEDED. TS = Timeline Studio, OC = OpenCut classic (OC marcado "est." = só estático/unit tests).

| Requisito CapIA | TS | OC | Ação |
|---|---|---|---|
| Timeline | faixas fixas, float | faixas livres tipadas, ticks (est.) | REPLACE (TS) / KEEP (OC) |
| Tracks | fixas por tipo | lista livre (est.) | REPLACE / MODIFY |
| Clips | ok, float | ok, ticks | MODIFY |
| Preview | DOM/canvas | GPU renderer (est.) | WRAP |
| Export | WebM ok; MP4 UNVERIFIED; libx264 | est. | MODIFY (só encoders aprovados) |
| Project | `.timeline` zip c/ mídia | storage local versionado (est.) | REPLACE (TS) / MODIFY (OC) |
| Persistência | sem migrações testadas | migrações v1→v21 + testes | PORT_IDEA do OC |
| Undo/redo | snapshot | Command classes | MODIFY |
| Captions | ok (UI parcial, agente ok) | subtitles/transcription (est.) | WRAP |
| Text | painel existe | text (est.) | WRAP |
| Audio | ok básico | audio-separation (est.) | MODIFY |
| Keyframes | ok | animation/keyframes (est.) | KEEP |
| Transitions | modelo existe | não confirmado | ADD |
| Effects | painel/filtros | effects + masks (est.) | WRAP |
| Speed | curva | speed/retime (est.) | KEEP |
| Snapping / Ripple | parcial | snapping/ripple (est.) | KEEP (OC) |
| Multiple sequences | ausente | scenes | ADD |
| Nested sequences | ausente | ausente | ADD (LARGE) |
| Command API | **ótimo** (`applyCommandPlan`) | ausente | PORT_IDEA (TS→OC) |
| Agent editing | WebMCP 21 tools, histórico compartilhado | ausente | PORT_IDEA |
| MCP | stdio 8 tools | ausente | PORT_IDEA |
| Headless | CLI + ffmpeg | ausente | ADD |
| Reference analysis | script diagnóstico | ausente | NOT_NEEDED (CapIA tem) |
| B-roll / Asset acquisition / Generation / Providers / Memory / Demand Interpreter / Planner / Editor Agent / Critic | skill textual | ausente | NOT_NEEDED (CapIA tem, Fases 4–5) → ADD no novo host |
| REST / webhooks / job queue | ausente | ausente | NOT_NEEDED (CapIA Fase 6) |
| Desktop packaging | PWA | GPUI | ADD |

## 9. Pontuação ponderada (0–10; peso soma 100; total = Σ nota×peso/10)

Pesos coerentes com os princípios do CapIA (tempo inteiro, timeline livre, comando único, multi-sequence). Notas de OC são **provisórias** (sem teste funcional).

| Critério | Peso | TS | OC (prov.) |
|---|---|---|---|
| Licença / segurança comercial | 8 | 5 | 8 |
| Completude do editor | 7 | 7 | 6 |
| UX estilo CapCut | 6 | 8 | 7 |
| Maturidade da timeline | 8 | 4 | 7 |
| Preview | 6 | 6 | 6 |
| Export | 6 | 5 | 5 |
| Persistência | 6 | 6 | 6 |
| Desempenho | 6 | 3 | 4 |
| Commandability | 8 | 7 | 6 |
| Prontidão IA/agente | 6 | 7 | 3 |
| Arquitetura | 8 | 4 | 7 |
| Manutenibilidade | 5 | 4 | 7 |
| Qualidade de testes | 5 | 2 | 7 |
| Aderência multi-sequence | 7 | 1 | 5 |
| Caminho desktop | 3 | 3 | 5 |
| Custo de adaptação ao CapIA | 5 | 3 | 6 |
| **Total /100** | 100 | **48,0** | **60,2 (provisório)** |

## 10. Lacunas críticas do Timeline Studio

FUNDAMENTAL (estrutural): faixas fixas/sem timeline livre; sem multi-sequence/aninhamento; tempo em float; undo por snapshot; mídia embutida no projeto; ausência total de testes unitários.
FIXÁVEL: UI sem virtualização (>500 clips inutilizável); GPL/x264/modelos; copiar/colar ausente; agente recusa clips com tempo complexo; render headless sem legendas.

## 11. Decisão

**REJECT_TIMELINE_STUDIO_TEST_OPENCUT.**

Justificativa: a única forma de atender aos princípios 3 e 4 do CapIA (timeline livre, tempo inteiro, multi-sequence) em Timeline Studio é reescrever o modelo de documento, os reducers, o histórico, o arquivo e a timeline de UI — ou seja, reconstruir o núcleo, o que a estratégia pretende evitar, sobre uma base sem testes. Seus pontos fortes (camada de comandos com revisão/idempotência/atomicidade/diff, WebMCP com histórico compartilhado, MCP/CLI) são **ideias portáveis** (PORT_IDEA), não motivo para adotar a base. A decisão **não** é voltar automaticamente ao editor próprio: OpenCut classic tem, por evidência estática e 145/150 testes passando, o modelo mais próximo (ticks inteiros, faixas livres, comandos com undo, cenas, migrações). Falta a ele prova funcional (a fazer na próxima fase) e toda a camada de agente — que o CapIA já possui (Fases 4–6) e pode ser re-hospedada. Se o teste funcional do OpenCut classic falhar, o fallback é a opção `REJECT_CURRENT_CANDIDATES` (reavaliar contra continuar o editor próprio), não adoção forçada.

## 12. Limitações desta avaliação

Chromium sem H.264 (MP4 UNVERIFIED); sem GPU real; sem vídeo longo/VFR; sem medição de snapping; OpenCut classic sem teste de UI; auditoria de licenças de dependências transitivas não feita.
