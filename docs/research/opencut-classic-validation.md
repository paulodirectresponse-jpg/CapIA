# Fase 1B — Validação funcional e de adaptação do OpenCut classic

> Triagem técnica, não é parecer jurídico. Prioridade da evidência: execução real > teste automatizado > código > docs > README.
> Referências: [`open-source-base-evaluation.md`](open-source-base-evaluation.md) (Fase 1; Timeline Studio **não** foi reconsiderado) e [`opencut-adoption-plan.md`](opencut-adoption-plan.md) (plano da Fase 2, **não executado**).
> Evidência bruta (resultados JSON, spike, patch, harness, 4 capturas): [`evidence/opencut-classic/`](evidence/opencut-classic/README.md).
> Nenhum código do OpenCut foi importado para o produto. Todos os testes rodaram em clone separado (`/home/user/opencut-app/opencut-classic`, branch descartável `validation/capia-spike`).

## 0. Resumo executivo

| Pergunta | Resposta com evidência |
|---|---|
| A. É um editor funcional de verdade? | **Sim.** UI real exercitada em Chromium: importar vídeo/imagem/áudio, multi-track, mover, trim, split, delete, ripple, undo/redo, copiar/colar, duplicar, texto, SRT, transform, opacidade, volume, velocidade, keyframes, efeito (Blur), máscara, separar áudio, salvar/reabrir, exportar WebM. **Faltam:** transições, track reorder, grupos, criação de cena pela UI, H.264 (não verificável aqui). |
| B. O núcleo/timeline dispensa reescrita? | **Sim para o domínio** (41 comandos, tempo inteiro, imutável, O(1)-ish até 50k clips em ~10 ms). **Não para a camada de render da UI** (sem virtualização: 10k clips = 104k nós DOM, 22 s de carga). Corrigível sem reescrever o domínio. |
| C. É comercialmente utilizável? | **LEGAL_YELLOW solucionável** (MIT; 4 itens a remover/substituir/cumprir: `botid`, `sharp-libvips` LGPL-3, `soundtouchjs` LGPL-2.1, modelos de IA por licença). Nenhuma GPL/AGPL/NC distribuída por padrão. |
| D. A IA opera a MESMA timeline sem Computer Use? | **Sim — `ADAPTER_FEASIBLE`.** Spike de ~230 linhas chama os mesmos `Command`/managers da UI; humano↔programa compartilham estado **e** histórico (verificado ao vivo); o domínio roda **headless em Bun** sem DOM. |

**Decisão: `ADOPT_OPENCUT_CLASSIC_WITH_MAJOR_CHANGES`** (seção 24). Score verificado **62,9/100**; confiança de adaptação **66/100**.

## 1. Preservação do CapIA e versão testada

- `git fetch`; `research/open-source-base-evaluation` contém `81765826e96ecb1153090ff1ff4ffd2e1438f39b`. Branch `research/opencut-classic-validation` criada **desse commit**. `main` (65b2b87) intocado; sem merge.
- **OpenCut classic:** `cf5e79e9` (2026-05-17), `git fetch` confirma que **é o HEAD atual do upstream** (nenhum commit posterior ⇒ `BASELINE_RESULT` = `LATEST_UPSTREAM_RESULT`). Repositório **arquivado** pelo autor ("mark repo as archived legacy", 2026-05-16); reescrita em `opencut-app/opencut`.
- Runtime: Bun 1.3.14 (o `packageManager` pede 1.2.18 — também testado), Node 22.22, Rust 1.97, wasm-pack 0.15 / wasm-bindgen 0.2.116, Chromium 1194 (Playwright), SwiftShader (sem GPU real).

## 2. Licenças — auditoria real

Origem dos dados: `package.json` de 1 951 pacotes em `node_modules` + `cargo metadata` (workspace + 760 crates) + inspeção de assets.

**LICENSE raiz:** MIT (© 2025-2026 OpenCut). `opencut-wasm` publicado: MIT.

| Item | Licença | Classe | Escopo | Obrigação / ação |
|---|---|---|---|---|
| ~726 JS + ~500 crates | MIT / Apache-2.0 / ISC / BSD / Zlib / Unicode / BlueOak / 0BSD | PERMISSIVE | runtime/build | atribuição (NOTICE) |
| `mediabunny` 1.41 (demux/mux WebCodecs) | MPL-2.0 | WEAK_COPYLEFT | **runtime, distribuído** | copyleft por arquivo: manter arquivos modificados sob MPL e disponibilizar fonte deles. **Não bloqueante.** |
| `soundtouchjs` 0.3.0 (pitch/tempo) | LGPL-2.1 | WEAK_COPYLEFT | **runtime, distribuído** | permitir substituição/relink (JS ES module separado é suficiente); **YELLOW** — ou trocar por alternativa permissiva |
| `@img/sharp-libvips-*` | LGPL-3.0-or-later | WEAK_COPYLEFT | **só servidor Next (otimização de imagem)** | **REMOVER** (CapIA não roda servidor Next) |
| `lightningcss`, `axe-core` | MPL-2.0 | WEAK_COPYLEFT | build/dev | nenhuma |
| `cbindgen`, `dwrote`, `option-ext` (Rust) | MPL-2.0 | WEAK_COPYLEFT | build / desktop GPUI | nenhuma (desktop GPUI não será usado) |
| `self_cell` | Apache-2.0 **OR** GPL-2.0 | dual | transitivo Rust | escolher Apache-2.0 |
| `botid` 1.5.11 (Vercel) | **sem licença declarada** | **UNKNOWN** | runtime web | **REMOVER** (anti-bot SaaS; não faz parte do editor) |
| `@better-fetch/fetch` | LICENSE presente (MIT) | PERMISSIVE | runtime | — |
| `caniuse-lite` CC-BY-4.0, `language-subtag-registry` CC0 | dados | PERMISSIVE (atribuição) | build | atribuição |
| `@huggingface/transformers` + modelos Whisper | código Apache-2.0; **pesos baixados em runtime** | **modelo ≠ código** | runtime, opt-in | **auditar licença de cada modelo** em `transcription/models.ts` antes de distribuir/habilitar (UNVERIFIED) |
| Fontes | atlas de sprites (`public/fonts/*.avif`) + CDN Google Fonts em `fonts/google-fonts.ts` | UNKNOWN por família | runtime | auditar famílias (OFL/Apache esperadas); preferir empacotar |
| Sons | Freesound via API (`api/sounds/search`), filtro "commercially licensed" | por item | runtime online | **remover** (CapIA tem Asset Gateway próprio) |
| Ícones | `@hugeicons/core-free-icons`, `lucide-react` | MIT/ISC | runtime | — |
| Música/SFX/modelos/binários embutidos | **nenhum encontrado** em `public/` (5,7 MB: favicons, logos, atlas de fontes, preview) | — | — | — |
| FFmpeg / codecs | **nenhum** (WebCodecs do navegador; `mediabunny` em JS) | — | — | H.264 depende do codec da plataforma (decisão `OUTPUT-H264` do CapIA continua aberta) |
| Código vendorizado | nenhum relevante | — | — | — |
| Serviços (Postgres/Drizzle, better-auth, Upstash, Marble CMS, Freesound, Databuddy) | — | — | servidor/rede | **REMOVER** (local-first); `env/web.ts` os exige só para validar `.env` |

**Veredito: `LEGAL_YELLOW` — solucionável.** Nada GPL/AGPL/NC/CC-NC é distribuído por padrão; os itens YELLOW são `soundtouchjs` (LGPL), `mediabunny` (MPL), modelos/fontes a auditar e `botid` a remover. Gate G1 aceitável.

## 3. As falhas do `bun test` — causa raiz

Resultado obrigatório:

```
ROOT_CAUSE        = C (incompatibilidade Bun) + D (bugs reais do upstream, em camadas)
FIX_OR_WORKAROUND = harness: preload que instancia o .wasm explicitamente (sem mudar nenhum teste)
                    + 2 type guards ausentes no upstream (patch de validação)
FINAL_TEST_COUNT  = 218 testes em 31 arquivos: 214 pass, 4 fail
```

| Configuração | Testes | pass | fail | erros |
|---|---|---|---|---|
| Bun 1.3.14 puro (Fase 1) | 150 | 145 | 5 | 5 (arquivos que nem carregam) |
| Bun 1.2.18 (versão pinada) | 191 | 182 | 9 | 3 — erros diferentes (TDZ circular) |
| + preload WASM (harness) | 210 | 205 | 5 | 1 |
| + preload + 2 guards ausentes | **218** | **214** | **4** | 0 |

1. **Causa do `wasm.__wbindgen_start is not a function`:** `opencut-wasm` (e `wasm-pack --target bundler`, o fluxo oficial) emite `import * as wasm from "./x_bg.wasm"` — integração ESM de WebAssembly que o **webpack/Turbopack** resolvem e o **Bun não**: o import vira `{default: "<caminho>"}`. Provado: `bun` ⇒ `{__esModule, default:string}`; `node --experimental-wasm-modules` ⇒ exports reais. Não é artefato não construído (B) nem configuração (A): construí o WASM pelo fluxo oficial.
2. **WASM oficial:** `bun run build:wasm` **compilou** (3 min) e gerou `rust/wasm/pkg` com **exports idênticos** ao pacote publicado; só o passo `wasm-opt` falhou (download do binaryen no GitHub bloqueado pelo proxy). Binário sem `wasm-opt`: 4,39 MB (publicado: 3,04 MB).
3. **Workaround de harness** (`evidence/.../harness/wasm-preload.ts`): `Bun.plugin` que reimplementa `opencut_wasm.js` com `WebAssembly.instantiate`. Não toca em testes.
4. **Bugs reais do upstream (D):** `actions/keybindings/persistence.ts` importa `isShortcutKey` e `isActionWithOptionalArgs` que **nunca foram definidos** ⇒ `tsc` e `next build` **falham no HEAD** (ver seção 4).
5. **As 4 falhas restantes:** (a) `resolve.test.ts` passa `2.5` a `mediaTime()` (API inteira) — teste obsoleto pós-migração para ticks (D); (b) 2 testes de `masks/snap` e `mask point insertion` esperam resultados que a implementação atual não produz — **não investigado** se é teste obsoleto ou regressão (D/E, UNVERIFIED); (c) `text mask` precisa de canvas/DOM — limitação do ambiente `bun test` sem DOM (A). **Nenhum teste foi alterado para passar.**

## 4. Build completo

| Alvo | Resultado | Notas |
|---|---|---|
| `bun install` (lockfile congelado) | **PASS** | 1 951 pacotes, 10 s |
| build WASM (`bun run build:wasm`) | **PARTIAL** | compila; `wasm-opt` bloqueado por rede (env) |
| `tsc --noEmit` | **FAIL no HEAD → PASS com 5 remendos** | 49 erros: 42 só de `bun:test` types (testes); reais: guards ausentes (3), `IndexedDBAdapter` com assinatura antiga em `migrations/runner.ts` e `v1-to-v2.ts` (5), `adapter.set(key,value)` antigo (1), `stickersRegistry.register(id, p)` antigo (1), `*.css`/`content-collections` (gerados no build) |
| `eslint` | **FAIL** | 112 erros / 21 warnings (regras estritas, `no-unsafe-type-assertion`) — dívida de lint, não bloqueia build |
| `next build` | **FAIL no HEAD → PASS com os 5 remendos** (1 min 10 s, Turbopack) | patch: `evidence/.../patches/upstream-validation-fixes.patch` (136 linhas) |
| `bun test` | ver seção 3 | |
| desktop (GPUI) | **UNVERIFIED** | `apps/desktop/src/main.rs` tem **41 linhas** (janela vazia) — não é editor; não compilei |

Os remendos corrigem **bugs reais com impacto de runtime**: o migrador de projetos antigos (`runner.ts`) gravaria com assinatura errada, e o registro de stickers ficaria com chave `undefined`. Classificação: **FIXÁVEL / SMALL**. Build reproduzível **com** o patch (G2 aceitável).

## 5. Bateria funcional A–Z (UI real, Playwright + Chromium; estado verificado pelo próprio `EditorCore`)

Setup de UI: servidor `next start`; gestos por ponteiro real (mouse/teclado). `window.__capia` (spike) só **lê** o estado. WebGPU de SwiftShader entrega canvas vazio ⇒ usado o **fallback WebGL2** (flags sem `--enable-unsafe-webgpu`), que renderiza corretamente.

| # | Item | Resultado | Evidência |
|---|---|---|---|
| A | criar projeto | **PASS** | `/editor/<uuid>` |
| B/C/D | importar vídeo, imagem, áudio | **PASS** | miniaturas, durações, 4 assets |
| E | vídeo na timeline (arrastar) | **PASS** | `main.webm 0.00+20.00` |
| F | múltiplas tracks | **PASS** | drop acima ⇒ nova overlay; abaixo ⇒ audio; 2ª overlay |
| G | reorganizar tracks | **FAIL (não implementado)** | só `removeTrack`; sem reorder |
| H | mover clip | **PASS** | 2.00→2.50 (195 px = 0,5 s) |
| I | trim start | **PASS** | start 2.50→2.70, `trimStart=24000` ticks |
| J | trim end | **PASS** | 2.00→1.70 (paridade) |
| K | split | **PASS** | tecla S: `0+1 / 1+19` |
| L | delete | **PASS** | tecla Delete |
| M | ripple | **PASS** | toggle liga; main fecha o buraco; **overlays não acompanham** |
| N | snapping | **PARTIAL / inconclusivo** | ligado por padrão, limiar 10 px no código; **3 testes de arrasto não observaram snap** (pode ser artefato do harness); sem teste unitário de snapping de timeline |
| O/P | copiar/colar | **PASS** | Ctrl+C/V no playhead |
| Q | duplicar | **PASS** | Ctrl+D (cria na track acima) |
| R/S | undo/redo | **PASS** | Ctrl+Z / Ctrl+Y |
| T | texto | **PASS** | "Default text" arrastado ⇒ elemento `text` |
| U | captions | **PARTIAL** | **import SRT = PASS** (2 elementos); auto-legenda (Whisper no navegador) **UNVERIFIED** (baixa pesos da rede) |
| V | posição/escala/rotação | **PASS** | inspector ⇒ 40/-25/1.5/15° |
| W | opacidade | **PASS** | 0.6 |
| X | volume/mute | **PASS** | volume -6, muted true |
| Y | velocidade | **PASS** | 1.5× ⇒ duração 20→13.33 s |
| Z | keyframes | **PASS** | diamante + edição ⇒ `animations` com chaves, `segmentToNext`, tangentes |
| AA | efeitos | **PARTIAL** | **só 1 efeito (Blur)**, via GPU, arrasto no clip = PASS |
| AB | máscaras | **PASS** | menu: Split, Cinematic Bars, Rectangle, Ellipse, Heart, Diamond, Star, Text, Pen tool; Ellipse aplicada |
| AC | transições | **FAIL (não implementado)** | painel "coming soon"; nenhum código |
| AD | separar áudio da fonte | **PASS** | cria clip em track de áudio |
| AE | clips agrupados | **FAIL (não implementado)** | sem tipo `group`; só multiseleção/`group-move` |
| AF | múltiplas cenas | **PARTIAL** | ver seção 17 |
| AG/AH/AI | salvar/fechar/reabrir | **PASS** | autosave (800 ms) + **reinício completo do navegador** com perfil persistente |
| AJ | continuar editando | **PARTIAL** | após reabrir: export OK e mídia decodifica; o split por UI **não acertou o clip** (geometria do meu script) — split/undo por UI foi provado em sessão nova (K, R) e na paridade, **não** encadeado após reabrir |
| AK | exportar | **PASS (WebM)** / **FAIL no ambiente (MP4)** | ver seção 9 |

## 6. Mini edição real (anúncio)

**`PARTIAL` — honesto:** montei o anúncio de ~20 s no editor **real**: vídeo principal com 3 cortes (`0–4 | 4–9 | 9–14 | 14–20`; os cortes saíram em 4/9/14 s em vez de 5/10/15 por erro de geometria do meu script), B-roll e imagem em tracks superiores, música em track de áudio cortada com cross-track, salvar → **fechar o navegador** → reabrir (7 elementos, mídia ainda decodificável, preview renderiza) → exportar WebM. **Transform, keyframe, velocidade, texto, SRT, efeito, máscara e separação de áudio** foram verificados em scripts de UI separados **na mesma build**, não encadeados na mesma sessão; o ripple-delete dentro da montagem encadeada **não** aplicou (PARTIAL), mas funcionou isoladamente (M). Não houve julgamento humano de qualidade visual.

## 7. Persistência (`CREATE → EDIT → SAVE → CLOSE → REOPEN → EDIT`)

Inspecionado direto no IndexedDB/OPFS do perfil:

- **Projeto:** IndexedDB `video-editor-projects/projects`, registro `{id, metadata, scenes[], currentSceneId, settings, version: 31, timelineViewState}` — JSON puro (63 KB para o projeto de 7 clips). Datas como `Date`.
- **Mídia:** binário em **OPFS** `media-files-<projectId>/<mediaId>`; metadados + miniatura (data-URL) em IndexedDB por projeto `video-editor-media-<projectId>`. Elementos referenciam **`mediaId` (UUID)**, nunca caminho ⇒ sem caminhos absolutos, mas também **sem relink/offline**: a mídia é **copiada** para o armazenamento do navegador (contraria "mídia fica local, referenciada" do CapIA; precisa de camada de arquivos no desktop).
- **Schema/migrações:** `version: 31`, **31 migrações encadeadas** (`v0→v1 … v30→v31`) com política "apenas aditivas" (`migrations/AGENTS.md`) e ≈21 arquivos em `migrations/__tests__`. Dois bugs reais no executor legado (corrigidos no patch).
- **Atomicidade:** `IndexedDBAdapter.set` = uma transação IDB; salvar medido em **17–228 ms** (100–10 000 clips).
- **Corrupção/crash:** autosave debounced (800 ms); **sem journal próprio, sem checksum, sem recuperação de crash além do último autosave**; **o histórico de undo NÃO é persistido** (`canUndo=false` após reabrir).
- **Portabilidade:** **não existe export/import de projeto** (arquivo). IDs de clip são UUIDs gerados no comando (não determinísticos).

**KEEP:** modelo `TProject/TScene/Track/Element` + cadeia de migrações. **MODIFY:** persistir em arquivo `.capia` (SQLite no host Tauri) mantendo as migrações do upstream como camada de leitura; mídia por referência com hash (princípio 8), relink, journal, histórico persistido.

## 8. Renderer

- **Caminho principal:** compositor **wgpu em WASM** (`rust/crates/compositor`, 870 linhas + `gpu`), texturas dos nós 2D → GPU; **preview, snapshot e export usam o mesmo** `CanvasRenderer.render`. Não é código morto.
- **Nós:** video, image, text, sticker, graphic, color, blur-background, effect-layer. **Blend modes** no compositor; transforms/opacity/keyframes resolvidos em TS; máscaras (feather na GPU); efeitos por passes de shader (só Blur registrado). **Sem transições.** Cor: sRGB, sem gestão de cor.
- **Backends:** WebGPU primeiro, **fallback WebGL2** (`try_gl_fallback`). Em SwiftShader o caminho WebGPU devolveu canvas **transparente** (aprovação em GPU real: `UNVERIFIED_ON_CURRENT_ENVIRONMENT`); o WebGL2 renderizou corretamente (cores corretas, camada de texto/imagem por cima).

## 9. Export

- **Formatos oferecidos:** MP4 (H.264) e WebM (VP9), qualidade low→very_high, áudio opcional (`ExportOptions`).
- **WebM (VP9 + Opus): PASS.** 20 s de conteúdo → 2 671 214 bytes, 1280×720, 30 fps, Opus 48 kHz estéreo, Matroska/WebM (ffprobe). **Tempo: 217 s** (≈10,9× mais lento que tempo real) **em SwiftShader, sem GPU** — não representa hardware real.
- **MP4/H.264: FAIL no ambiente** — `VideoEncoder.isConfigSupported(avc1.*)` = `false` e AAC = `false` neste Chromium (Playwright não traz H.264/AAC). Em WebView2/Windows costuma existir; **`UNVERIFIED_ON_CURRENT_ENVIRONMENT`**. Dependência: codec da plataforma (WebCodecs), **sem libx264/FFmpeg** ⇒ licenciamento H.264 é do SO/WebView, não distribuído por nós (a decisão jurídica `OUTPUT-H264` do CapIA continua aberta).
- **Preview × export (4 instantes, 4 pontos cada):** cores equivalentes dentro do erro de codec VP9 (ex.: t=3 s `(191,191,191)` vs `(190,190,190)`; t=7 s `(254,164,0)` vs `(253,163,0)`); em quadros com conteúdo em movimento (t=1, 12 s) houve diferença de ≤1 quadro de fonte (`(15,14,142)` vs `(12,1,195)`). **Nenhuma divergência semântica** (camadas, ordem, transform) detectada.

## 10. Desempenho (sem otimização; clips de vídeo reais em 50 por track)

UI (Chromium/SwiftShader, `performance.memory`, mesmas APIs que a UI usa):

| N | carga (ms) | nós DOM | heap (MB) | scroll p95 (ms) | zoom (ms) | split (ms) | move (ms) | save IDB (ms) |
|---|---|---|---|---|---|---|---|---|
| 100 | 787 | 1 511 | 72 | 83 | 230 | 93 | 139 | 17 |
| 500 | 1 687 | 5 647 | 155 | 217 | 663 | 324 | 513 | 21 |
| 1 000 | 2 585 | 10 817 | 235 | 400 | 865 | 457 | 1 080 | 36 |
| 5 000 | 11 384 | 52 156 | 959 | 4 050 | 4 700 | 2 602 | 4 523 | 224 |
| 10 000 | 22 475 | 103 856 | 1 857 | 6 600 | 11 304 | 4 581 | 9 906 | 228 |

**Domínio headless em Bun (mesmos comandos, sem UI):** split/move/delete/undo ≤ **10 ms até 50 000 clips**; serialização 89 ms @50k.

**Diagnóstico:** o domínio escala; **a UI re-renderiza tudo a cada comando** (custo ∝ N: ≈0,5–1 ms por clip por comando) e **não virtualiza** (1 nó DOM-árvore por clip: ≈10 nós/clip). Degradação perceptível a partir de **~500 clips** (scroll p95 > 200 ms), inutilizável em ≥ 5 000. **Classe: FIXÁVEL** (virtualização por janela de tempo + memoização por track/elemento); não exige reescrever domínio, comandos nem renderer. Gate G7 aceitável **como trabalho de UI**. O CapIA original (ui-timeline em canvas) já resolveu isso — reusar a técnica é `PORT_IDEA`.

## 11. Timeline UX

Classificação: **`NEEDS_POLISH`** (base boa, não `NEEDS_MAJOR_UI_WORK`).
Positivo: densidade e layout "CapCut" (tracks empilhadas, main track fixa, overlay acima, áudio abaixo), zoom (slider/botões), playhead por clique na régua, seleção + multiseleção (Shift/Ctrl), atalhos completos (S/Q/W, Ctrl+C/V/D/A/Z/Y, N snapping, J/K/L, setas, Home/End), menu de contexto, inspector contextual (transform/áudio/velocidade/opacidade/máscara/efeitos), frame-snap em 30 fps, painel de cenas. Negativo: sem reorder de tracks, sem transições, snapping não confirmado, perf ≥ 500 clips, 1 efeito, painéis "coming soon" (Transitions, Adjustment).

## 12. Spike de edição programática (o teste mais importante)

Código: `evidence/.../spike/capia-spike-index.ts` (~230 linhas), registrado só no clone descartável. **Não duplica regra de timeline**: usa `InsertElementCommand`, `SplitElementsCommand`, `MoveElementCommand`, `UpdateElementsCommand`, `DeleteElementsCommand`, `buildElementFromMedia`, `EditorCore.command.execute/undo/redo`.

API: `inspect_project/scene/tracks`, `insert_clip/move_clip/trim_clip/split_clip/delete_clip`, `undo/redo`, `preview_plan`, `apply_plan` (com `baseRevision`, `operationId`, lote atômico).

Contrato verificado ao vivo (todas **PASS**): `baseRevision` obsoleto rejeitado (`STALE_REVISION`); `operationId` repetido é no-op idempotente; plano com 2ª operação inválida **não aplica nada** (`AtomicBatch` com rollback); `preview_plan` devolve diff e não toca documento/histórico (executa, mede, desfaz); um plano = **uma** entrada de undo (Ctrl+Z da UI desfaz o plano inteiro).

**Headless (sem navegador):** `EditorCore.getInstance()` + `setActiveProject` + comandos rodaram em **`bun test` sem DOM** (só o shim WASM): `insert → split → undo` corretos (`0–10` → `0–4 | 4–10` → `0–10`). Logo o domínio pode servir MCP/REST/CLI **sem WebView** (processo Bun/Node sidecar) ou na própria WebView.

## 13. Paridade humano ↔ programa

Fluxo executado ao vivo (15 verificações, todas **PASS**): UI cria projeto e insere clip → **PROGRAMA** `inspect` vê o clip da UI → **PROGRAMA** `split(2 s)` → **PROGRAMA** `move(4 s)` → UI observa (DOM mostra 2 clips, captura `parity-after-programmatic-move.png`) → **UI** trim por arraste (2,00→1,70) → **PROGRAMA** `inspect` vê o trim → **UI Ctrl+Z ×2** desfaz o trim e **depois o `move` programático** → **PROGRAMA** `undo` desfaz o `split` → redo → **PROGRAMA** `delete` → UI observa estado final.

**Resposta: sim, a IA pode ser “apenas mais um cliente do mesmo editor”.** Um estado, um histórico. **`ADAPTER_FEASIBLE`.** Nenhuma segunda timeline, nenhum espelhamento, nenhum mouse/Computer Use, nenhuma reescrita de UI, nenhuma duplicação do motor.

Limites a tratar no adapter definitivo (seção 15): ids gerados dentro do comando (`generateUUID`) ⇒ não determinísticos; `execute` não reporta falha (comandos retornam `undefined` e **são empilhados no histórico mesmo quando não fazem nada**); `BatchCommand` **não é atômico** (sem rollback); histórico em memória; o `EditorCore` é singleton (um editor por processo); preview precisa de execute+undo.

## 14. Comandos e undo

- **41 classes** em `commands/` (`Command` abstrata: `execute/undo/redo`; selection patch no resultado). Famílias: media (add/remove asset), project (settings), scene (create/delete/rename/bookmarks), timeline/element (insert, delete, duplicate, split, move, update, source-audio, keyframes ×5, effects ×5, masks ×5), track (add/remove/mute/visibility), clipboard (paste, paste-keyframes), `TracksSnapshotCommand`, `BatchCommand`.
- **Undo = snapshot imutável** do `SceneTracks` guardado no `execute` (`savedState`) — barato (ponteiro) e O(1); ripple aplicado como reator pós-comando; um reator poda tracks vazias.
- **`CommandManager`**: pilha `history/redo`, `registerReactor`, ripple opcional. `push()` permite entradas sem executar; `pushHistory=false` para arrasto/preview (`previewElements/commitPreview`).
- **Sem:** ids de operação, revisão, transação/rollback, erro estruturado, persistência de histórico, histórico por ator. **ADD no adapter** (provado no spike).

## 15. Desenho do adapter `PLAN → VALIDATE → PREVIEW/DIFF → APPLY → UNDO RECEIPT`

```
Rust capia-intelligence (Planner/Editor/Critic, memória, gateway…)        [REUSE_WITH_ADAPTER]
        │  EnginePort (trait nova, extraída de capia-editor-api::Session)
        ▼
capia-engine-bridge (Rust, no host Tauri/sidecar) ── JSON-RPC/IPC ──►  OpenCut domain (TS, mesma instância da UI)
                                                                        ┌─ AgentSurface (≈ spike) ─────────────────────────┐
  inspect_* (DTO versionado)  ◄────────────────────────────────────────  │ revision (CAS) · operationId (derivado) · cache  │
  validate_plan(plan)                                                    │ validate (regras do domínio + precondições)      │
  preview_plan → diff                                                    │ preview = execute + diff + rollback (fora do hist)│
  apply_plan (Actor::Agent)  ───────────────────────────────────────►   │ AtomicBatch → UMA entrada em CommandManager       │
  undo_receipt {entryId}  ◄──────────────────────────────────────────   │ ator + planId no entry (undo seletivo)           │
                                                                        └──────────────────────────────────────────────────┘
```

Regras: (1) IA/CLI/API/MCP só chamam `AgentSurface` — **única porta** de escrita além da UI; (2) ids derivados (tarefa+passo+índice) injetados no comando (precisa de parâmetro `idFactory` nos comandos que hoje chamam `generateUUID`); (3) `Command.execute` passa a lançar/retornar erro estruturado; (4) histórico ganha metadados `{actor, planId, operationIds}` e opcionalmente persistência; (5) o catálogo de operações fechado do CapIA (`capia-server` OperationCatalog) é reapontado para as operações do `AgentSurface`; (6) validação semântica (limites de duração, tracks compatíveis) reaproveita `resolveTrackPlacement`/`canElementGoOnTrack` do domínio.

Alternativa headless: o mesmo `AgentSurface` em Bun/Node (provado) alimenta MCP/REST quando não houver WebView.

## 16. Mapeamento do CapIA existente → OpenCut

Branches: `claude/m05-core-engine … phase6-finalization, rc2-stabilization-ux, rc3-free-timeline-ux`. Código real em `claude/rc3-free-timeline-ux` (tamanhos por `git ls-tree`):

| Módulo CapIA (KB) | Destino | Justificativa |
|---|---|---|
| `capia-time`, `capia-model`, `capia-commands` (núcleo Rust, 416 KB) | **DISCARD** como motor; **PORT_LOGIC** das regras/cenários (`tests/acceptance`, property tests, semântica `preview→apply_plan`, `operation_id`, undo seletivo) | o domínio passa a ser o do OpenCut |
| `capia-store` (346) | **REUSE_WITH_ADAPTER** (tabelas de IA: runs, memória, proveniência, `ai_records/usage`, orçamento) · **DISCARD** (tabelas de documento) | AutonomyStore schema 5 independe do editor |
| `capia-project`, `capia-editor-api` (383+64) | **DISCARD/REPLACE** por `AgentSurface` + `EnginePort` | `Session` é concreta e presa a `capia-model` |
| `capia-render`, `capia-decode`, `capia-preview` (945+70+30) | **DISCARD** no caminho do editor (compositor wgpu + WebCodecs do OpenCut) | OpenCut faz melhor como editor; `capia-render` vira **oráculo de paridade** em testes |
| `capia-media`, `capia-assets` (280+113) | **REUSE_AS_IS** como serviço de análise (ffprobe, áudio p/ ASR, amostragem de quadros, hash/dedup) no sidecar Rust | IA precisa de análise fora do navegador |
| `capia-jobs` (48) | **REUSE_AS_IS** (jobs de IA/geração/download) | |
| `capia-secrets`, `capia-ai` (21+399) | **REUSE_AS_IS** — providers, router, registry, Brain Profile, Replay, SafeFetcher; **não conhecem projeto** | núcleo da inteligência a preservar |
| `capia-intelligence` (926): Demand Interpreter, Planner, Editor Agent, Critic (visão), Memory, Asset Gateway, geração, Reference Analyzer, AI Run | **REUSE_WITH_ADAPTER** (**maior risco**): depende de `capia-model`/`capia-commands`/`Session`; exige `EnginePort` + DTO de projeção; o Editor (compilador determinístico) é **PORT_LOGIC** para o catálogo de operações do OpenCut | LARGE |
| `capia-server` (581): REST, SSE, MCP, webhooks, escopos | **REUSE_WITH_ADAPTER** (reapontar `OperationCatalog`) | |
| `capia-updater`, `capia-support`, `apps/desktop` (Tauri, NSIS, updater assinado, crash opt-in) | **REUSE_AS_IS** (hospedar o build web do OpenCut — caminho C) | |
| `packages/editor-ui`, `ui-timeline` (571) | **DISCARD** (UI do editor) · **PORT** dos painéis de IA (chat, runs, aprovação, memória, gateway, orçamento) para a stack do OpenCut | |
| `tools/*acceptance*`, `check-architecture`, `check-licenses`, `PROVENANCE` | **REUSE_AS_IS** (governança; reescrever a matriz de fronteiras) | |

**Princípios do CLAUDE.md afetados (exigem ADR):** #4 tempo inteiro ✔ mantido (OpenCut usa `MediaTime` inteiro, **120 000 ticks/s** ≠ 705 600 000/s: 48 kHz = 2,5 ticks/amostra ⇒ edição de áudio **não** é sample-exata — decisão: aumentar a constante no crate Rust `time` + migração aditiva v32, ou aceitar); #5 "core Rust headless" ⇒ passa a "domínio TS headless-capaz + serviços Rust"; #8 local-first ⇒ exige camada de arquivos (mídia copiada hoje para OPFS).

## 17. Multi-sequence (scenes)

Testado de verdade (API + UI + reinício): criar 2 cenas (`AD1`, `AD2`) → renomear → trocar → editar cada uma **independentemente** (cada `TScene` tem seu `SceneTracks`) com **assets compartilhados** (nível projeto) → painel de cenas lista/troca/apaga → **persistem após reinício completo** (cena ativa `AD2` restaurada) → **exportar cena específica** = tornar ativa e exportar (PASS, 83 KB WebM, 72 s em SwiftShader).
**Não existe:** criar cena pela UI (`scenes-view.tsx` só troca/apaga), **duplicar** cena, referência entre cenas, undo da troca de cena (não é comando). O duração do projeto = cena `isMain`.

Transformação `Scene → Sequence` (PROJECT com `AD1, AD2, AD3, HOOK_1, BODY_MASTER, CTA`): **`MEDIUM`** — renomear conceito, UI de criar/duplicar/renomear/ordenar, abas por sequência, `timelineViewState` por sequência, export por sequência/lote, migração aditiva. **Não é arquitetural**: o modelo já separa tracks por cena.

## 18. Nested sequences — viabilidade (design, sem implementar)

Necessário: `AD1 = HOOK_01 + BODY_MASTER + CTA_01`, com `BODY_MASTER` compartilhado.

| Aspecto | Desenho no OpenCut | Esforço |
|---|---|---|
| clip que referencia sequência | novo membro da união `TimelineElement`: `{type:"sequence", sequenceId, trimStart/End, params…}` (migração v32 aditiva) | M |
| duração | derivada de `calculateTotalDuration(cena referenciada)` (+ trim) | S |
| render recursivo | `buildScene` já gera árvore `RootNode`; adicionar `SequenceNode` que constrói a sub-árvore da cena referenciada, mapeia `t_local = t − start + trimStart` e compõe numa textura intermediária (`texture_pool` do compositor) | L |
| áudio | `createTimelineAudioBuffer` (media/audio) precisa achatar recursivamente com offsets | M |
| preview/cache | chave de cache por `sequenceRevision` (revisão por cena; hoje não existe) + invalidação transitiva | M |
| ciclos | DFS no comando de inserir/duplicar (validação do domínio) | S |
| make unique | `DuplicateSceneCommand` + retarget do clip | S |
| flatten | comando que expande o clip em seus elementos com offset | M |
| UI | desenhar clip-sequência, "abrir sequência", breadcrumb | M |

**Classificação: `LARGE` (não `FUNDAMENTAL`).** É **muito trabalho**, mas **não exige reescrever o editor**: o modelo é uma união de elementos + renderer baseado em árvore + cenas imutáveis, que acomoda a extensão. Estimativa 4–8 semanas-engenheiro (hipótese, não medida).

## 19. Desktop

- **A. Web durante o desenvolvimento** — recomendado: o editor roda inteiro no navegador (provado).
- **B. Desktop existente (GPUI)** — **descartar**: `apps/desktop/src/main.rs` = 41 linhas (janela vazia), crates Rust compartilhados só em teoria; não compartilha o editor web; GPUI 0.2 é experimental e exigiria reescrever toda a UI.
- **C. Empacotar a UI web em Tauri** — **recomendado depois**: o CapIA já tem shell Tauri, NSIS, updater assinado e crash opt-in (Fase 6) para hospedar o build estático do editor + sidecar Rust (IA, mídia, `.capia`). Pendências: build `output: "standalone"`/estático sem servidor Next, `H.264` via WebView2 (`UNVERIFIED`), sistema de arquivos no lugar de OPFS.
- **Recomendação: A agora → C na Fase 2 (passo 11).** Não implementado.

## 20. Risco de manutenção do upstream — **MEDIUM**

Arquivado (2026-05-16), último commit 2026-05-17; 1 567 commits, 356 em 2026 (ritmo alto até arquivar); **86 007 linhas TS + 4 337 Rust**; deps modernas (Next 16, React 19, bun 1.2) — risco de segurança a gerenciar (Next/React/transitivos) mas não obsoleto; **HEAD arquivado não compila** (type errors) e tem 4 testes quebrados ⇒ trabalho em progresso interrompido. O rewrite novo (`opencut-app/opencut`) está quase vazio (Fase 1). **Possuir o fork é viável**: código tipado, camadas claras (`core/managers`, `commands`, `timeline`, `services/renderer`), `AGENTS.md` com políticas, migrações testadas. Custo: assumir 86k linhas; remover site, blog, auth, DB e CMS reduz a superfície.

## 21. Gates de adoção (G1–G12)

| Gate | Veredito | Evidência |
|---|---|---|
| G1 licença | **PASS (YELLOW solucionável)** | seção 2 |
| G2 build reproduzível | **PASS com patch** | seção 3–4; 5 remendos + shim de teste |
| G3 bateria principal | **PASS com lacunas** | transições, reorder, grupos ausentes; 1 efeito |
| G4 persistência | **PASS (navegador) / MODIFY p/ `.capia`** | reinício completo OK; sem arquivo portátil |
| G5 export | **PARTIAL** | WebM ✔ e paridade ✔; H.264 `UNVERIFIED`; sem GPU real |
| G6 timeline madura | **PASS** | snapping `PARTIAL` (inconclusivo) |
| G7 performance corrigível | **PASS (UI-only)** | domínio ≤10 ms @50k; UI sem virtualização |
| G8 edição programática | **PASS — `ADAPTER_FEASIBLE`** | spike + headless |
| G9 estado/histórico compartilhado | **PASS** | 15 verificações de paridade |
| G10 multi-sequence | **PASS (MEDIUM)** | cenas independentes, persistentes |
| G11 nested | **PASS (LARGE, não fundamental)** | desenho seção 18 |
| G12 fork sustentável | **PASS (MEDIUM)** | seção 20 |

## 22. Score verificado (mesmos 16 critérios e pesos da Fase 1)

| Critério | Peso | Timeline Studio | OpenCut (prov.) | **OpenCut (verificado)** |
|---|---|---|---|---|
| Licença / segurança comercial | 8 | 5 | 8 | **7** |
| Completude do editor | 7 | 7 | 6 | **7** |
| UX estilo CapCut | 6 | 8 | 7 | **8** |
| Maturidade da timeline | 8 | 4 | 7 | **8** |
| Preview | 6 | 6 | 6 | **7** |
| Export | 6 | 5 | 5 | **5** |
| Persistência | 6 | 6 | 6 | **6** |
| Desempenho | 6 | 3 | 4 | **4** |
| Commandability | 8 | 7 | 6 | **7** |
| Prontidão IA/agente | 6 | 7 | 3 | **6** |
| Arquitetura | 8 | 4 | 7 | **7** |
| Manutenibilidade | 5 | 4 | 7 | **5** |
| Qualidade de testes | 5 | 2 | 7 | **5** |
| Aderência multi-sequence | 7 | 1 | 5 | **6** |
| Caminho desktop | 3 | 3 | 5 | **5** |
| Custo de adaptação | 5 | 3 | 6 | **5** |
| **Total /100** | 100 | **48,0** | 60,2 (prov.) | **62,9** |

Ajustes vs. provisório (com motivo): Licença −1 (`botid`/LGPL/modelos a limpar); Manutenibilidade −2 (arquivado, HEAD quebrado); Testes −2 (4 falhas, sem teste de snapping/UI); Prontidão IA +3 e Preview +1 (spike, headless, compositor verificado); Timeline +1; Multi-sequence +1; Desempenho 4 (UI) com domínio excelente. Pesos inalterados.

**`OPENCUT_CLASSIC_VERIFIED_SCORE = 62,9 / 100`**
**`CAPIA_ADAPTATION_CONFIDENCE = 66 / 100`** (alta: domínio headless, histórico compartilhado, spike funcional; baixa/média: rehospedar `capia-intelligence` (acoplado a `capia-model`), H.264 em WebView2, GPU real, UI ≥ 500 clips, upstream arquivado, mudança de princípio #5).

## 23. Lacunas críticas

1. **Rehospedagem da inteligência** (`capia-intelligence` ↔ `Session`/`capia-model`): LARGE; depende de extrair `EnginePort`.
2. **Sem transições, reorder de tracks, grupos, duplicar/criar cena pela UI, 1 só efeito**: ADD (M–L).
3. **Nested sequences**: LARGE.
4. **UI sem virtualização** (≥ 500 clips): FIXÁVEL (M).
5. **Persistência em arquivo `.capia` + mídia por referência + histórico persistido**: MODIFY (L).
6. **H.264/AAC e GPU real**: `UNVERIFIED` — gate de aceitação da Fase 2 (Windows/WebView2).
7. **Determinismo de comandos** (ids, falha silenciosa, batch não atômico): MODIFY (S–M).
8. **Base arquivada, HEAD quebrado, 4 testes falhando, lint com 112 erros**: assumir dívida.
9. **Tick 120 000/s**: áudio não sample-exato (decidir).
10. **Mudança de princípio** (#5 Rust-first → TS domain): ADR novo obrigatório.

## 24. Decisão final da Fase 1

**`ADOPT_OPENCUT_CLASSIC_WITH_MAJOR_CHANGES`**

Critério do enunciado: *"Se para transformá-lo no CapIA precisarmos substituir timeline + state model + renderer + persistence + UI, ele não está economizando o suficiente."* Aqui **mantemos** timeline, modelo de estado, comandos/undo, compositor, UI e cadeia de migrações; **modificamos** persistência (arquivo `.capia`, mídia por referência) e a porta de escrita (`AgentSurface`); **adicionamos** sequências aninhadas, transições, virtualização e a ponte para a inteligência. O que é descartado do CapIA atual é justamente o que o OpenCut faz melhor como **editor** (UI da timeline, compositor, decode, modelo de documento) — a inteligência (≈ 1,9 MB de Rust: `capia-ai`, `capia-intelligence`, `capia-secrets`, `capia-server`, mais jobs e mídia de análise) é preservada.

Por que não `ADOPT_OPENCUT_CLASSIC` simples: nenhuma das lacunas 1–10 é trivial. Por que não `REJECT_CURRENT_CANDIDATES`: nenhum problema encontrado é **estrutural/fundamental** (não exige substituir o domínio), e o benefício é grande (editor CapCut-like funcional hoje, com paridade humano↔IA provada). **Condições de saída (kill criteria da Fase 2):** (i) H.264 exportável em WebView2/Windows real; (ii) compositor em GPU real sem divergência; (iii) timeline virtualizada com 5 000 clips ≤ 100 ms/comando; (iv) `EnginePort` extraído de `capia-intelligence` sem reescrever os agentes — se (i)/(ii) falharem de forma não corrigível, reavaliar.

## 25. Limitações desta validação

SwiftShader (sem GPU; export 10,9× tempo real; WebGPU não verificado); sem H.264/AAC; Whisper/Freesound/Google Fonts (rede bloqueada) não exercitados; snapping de timeline inconclusivo; mini edição encadeada parcial; 2 falhas de teste de máscara não investigadas; desktop GPUI não compilado; auditoria de licenças por **metadados** (não por leitura de cada arquivo de licença); estimativas de esforço são hipóteses.
