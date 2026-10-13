# Evidência — validação do OpenCut classic (Fase 1B)

Código de terceiros: apenas o **patch de 136 linhas** (`patches/`) com 5 correções em arquivos do OpenCut classic `cf5e79e9` (**MIT**, © OpenCut) — mantido aqui como evidência, **não** como código do produto. O resto é autoria CapIA (spike, testes, harness).

| Arquivo | O que é |
|---|---|
| `functional-results.json` | 61 verificações de UI/persistência/export/cenas/paridade/contrato (PASS/PARTIAL/FAIL) |
| `perf-ui.json` | desempenho de UI em 100/500/1k/5k/10k clips (SwiftShader, sem GPU) |
| `perf-headless-domain.json` | desempenho do domínio em Bun sem UI (100…50k) |
| `spike/capia-spike-index.ts` | `AgentSurface` mínimo sobre `EditorCore`/`Command` do upstream (instalado em `window.__capia` só no clone descartável; hook em `app/editor/[project_id]/page.tsx`) |
| `spike/headless*.test.ts` | domínio headless em `bun test` (insert/split/undo; perf) |
| `patches/upstream-validation-fixes.patch` | `isShortcutKey`, `isActionWithOptionalArgs`, assinaturas do `IndexedDBAdapter`, `adapter.set`, `stickersRegistry.register` |
| `harness/` | preload WASM para Bun, libs Playwright, teste de paridade humano↔programa, benchmark de UI |
| `screenshots/` | 4 capturas (editor, paridade, cenas reabertas, efeito/máscara/áudio) |

Reproduzir (resumo): clonar `opencut-classic@cf5e79e9`; `bun install`; `bun run build:wasm`; aplicar o patch; copiar `spike/capia-spike-index.ts` para `apps/web/src/capia-spike/` e chamar `installCapiaSpike()` no `useEffect` da página do editor; `next build && next start -p 3100`; Chromium com `--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader` (**sem** `--enable-unsafe-webgpu`); `node harness/parity.mjs`. Mídias de teste geradas com `ffmpeg lavfi` (não versionadas).
