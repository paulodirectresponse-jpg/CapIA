# OPEN SOURCE AUDIT — Missão M02 (due diligence)

> Auditoria feita sobre o código clonado (rasos, via proxy) em 2026-10-02. Nada foi copiado, instalado, executado ou forkado. **Os documentos de arquitetura (M01) não foram alterados**; mudanças sugeridas estão em §9 como *propostas* e dependem de decisão.

## 0. Método e limites da evidência

- Clonados e inspecionados por arquivo: modelo de dados, command layer, persistência, export/render, timeline UI, MCP, testes, dependências, licenças.
- **Não executei** nenhum app (sem benchmark de desempenho real, sem teste de UX ao vivo). Afirmações de desempenho vêm de leitura de código (ex.: DOM vs canvas, virtualização), não de medição.
- Contagem de commits/atividade vem do histórico clonado (raso, aprofundado quando possível). LOC = linhas contadas por `wc` nos diretórios citados (ordem de grandeza).
- Licenças: lidas nos arquivos `LICENSE`/`NOTICE` dos repositórios; Remotion, mediabunny e OpenTimelineIO verificados nos textos upstream. **Não é parecer jurídico.**
- Revisões inspecionadas:

| Repo | Commit | Data do último commit |
|---|---|---|
| MartinDelophy/ai-video-editor | `36cf7b9` | 2026-10-02 |
| OpenCut-app/opencut-classic | `cf5e79e` | 2026-05-17 (arquivado) |
| OpenCut-app/OpenCut | `e668010` | 2026-09-24 |
| tjameswilliams/ai-video-editor | `93f79bb` (v0.1.2) | 2026-08-21 |
| itsjwill/vanta | `350b053` | 2026-07-25 |
| abekyo/abekyo-editor | `d442894` | 2026-08-02 |

---

## 1. Resumo por candidato

### 1.1 MartinDelophy/ai-video-editor ("Timeline Studio") — o candidato que você pediu para verificar a fundo

**O que é:** editor de vídeo *de navegador* (Vite + React 19, **JavaScript**, só 2 arquivos `.ts`), com muitos recursos de IA local no browser (TTS, ASR, reparo/restauração, face swap, lip-sync, música via ONNX/WebGPU). 209 commits em ~12 semanas (início 2026-07-09), vários autores (3 respondem pela maior parte dos commits), ~40k linhas em `src/lib|hooks|workers` + ~16k em componentes. **Zero arquivos de teste.** CI só faz lint/typecheck/build.

**Licença:** MIT para o código original (`LICENSE`). `MODEL_LICENSES.md` declara que **pesos/modelos/mídia não estão cobertos** pela MIT; face swap usa pesos de pesquisa **sem direito comercial**. O libav.js embutido é LGPL-2.1+ (WASM).

**Verificação das alegações:**

| Alegação | Veredito | Evidência |
|---|---|---|
| Timeline estilo CapCut | **Parcial.** Funcional, mas DOM (sem `<canvas>`), sem virtualização, 4,6k linhas de componentes | `src/components/Timeline*.jsx` |
| Projeto editável | **Sim**, mas é um blob de ~40 campos de estado do React serializado em arquivo `.timeline` (zip via fflate); mídia como blobs | `useProjectFiles.js`, `projectArchive.js` |
| Agentes editando a timeline real | **Sim, via comandos** — mas agentes são *externos* (MCP/WebMCP); **não há orquestrador, Brain, providers nem pipeline de IA no app** (nenhum código de LLM) | grep sem hits de `chat/completions`/SDKs |
| Command registry | **Sim, sólido no desenho:** 34 operações em `reducers` (`visual.split`, `timed.move`, `caption.add`, `transition.set`, `marker.*`…), com validação de plano | `src/lib/projectCommandEngine.js` (1.143 linhas) |
| Transações/revisions | **Sim, boa ideia, escopo limitado:** `baseRevision` → `REVISION_CONFLICT`; aplica em `structuredClone` e só então devolve (atomicidade por clone); `operationId` idempotente (replay seguro); `diffProjects` | `applyCommandPlan` |
| Diff antes de apply | **Sim:** `preview` → `previewId` → `apply`, `stateToken` anti-estado-velho, `before/after/changes` | `docs/webmcp.md` |
| MCP | **Sim:** STDIO local com 8 tools (`inspect`, `diff`, `apply`, `render`…) + WebMCP no navegador com 21 tools (jobs de IA, export, amostragem de frames) | `skills/.../mcp/server.mjs`, `docs/webmcp.md` |
| Export offline | **Sim, no navegador:** canvas + WebCodecs via `mediabunny` (MPL-2.0), MP4/MOV/WebM. **Headless só um subconjunto** (Visuals + Voiceover + Music) | `offlineVideoExport.js`, README |
| VFR handling | **Não no sentido geral.** "VFR" aparece só no reparo de marca d'água (usa timestamps originais). Export usa grade fixa `index/fps` com **fps arredondado para inteiro (24–60)** — 29,97 vira 30, 23,976 vira 24. Seek de origem via `video.currentTime` (não frame-exato). Sem frame index | `createOfflineFramePlan`, `seekVideoFrame` |

**Limitações que impedem usá-lo como base do CapIA:**
1. **Sem sequences/timelines múltiplas e sem nested.** Um projeto = uma timeline.
2. **Sem tracks genéricas.** Coleções fixas (`visualSegments`, `visualOverlaySegments`, `audioSegments`, `musicSegments`, `stickerSegments`, `captionSegments`). A faixa visual principal é sequencial (só `duration`, sem `start`) — equivalente a *uma* main track magnética. "Layers" de overlay e "lanes" de áudio são números 1–1000 dentro de coleções.
3. **Tempo em float de segundos** (`start`, `duration`, `Date.now()+Math.random()` como ID). Contradiz ADR-007; `MIN 0.2s` e `Number.EPSILON` aparecem como remendos de tolerância no código.
4. **Undo = snapshots do estado inteiro do editor** (50 níveis, debounce 180 ms, num hook React), desacoplado do command engine — a "undo transaction" de agentes é um caminho paralelo.
5. **Arquitetura 100% browser** (blobs, `HTMLVideoElement`, canvas, workers). Nenhum core headless/Rust; migrar para Tauri exige reescrever mídia, export e persistência. Contradiz C1/C2.
6. **Sem testes** — nenhuma rede de segurança para alterar o núcleo.
7. **Escopo de IA divergente:** o grosso do código (face swap, lip-sync, música, restauração) é fora de escopo para DR/UGC/Ads V1 e carrega risco de licença de pesos.
8. Mantenedor(es) pequeno(s), velocidade alta e mudanças frequentes: um fork divergiria rápido.

**O que é aproveitável (ideias, não código):** formato de plano de comandos (`baseRevision`, ids de operação idempotentes, `diff` estruturado), fluxo `inspect → preview → apply` com tokens opacos e `requestId` para ações caras, paginação limitada em leituras, limites de ops por plano (500), erros com `code` + `operationId`, documentação `command-contract.md`. Algoritmos de remoção de silêncio e agrupamento de legendas (sem testes — validar antes de portar).

**Classificação:** código original **SAFE_WITH_OBLIGATIONS** (MIT, manter aviso) · **modelos/pesos/mídia: DO_NOT_USE** sem verificação individual · valor prático: **REFERENCE_ONLY**.

---

### 1.2 OpenCut-app/opencut-classic — experiência de timeline/UX

**O que é:** editor web (Next.js 16, Bun/Turbo, TypeScript) **arquivado** ("no longer maintained"; reescrita em outro repo). 1.567 commits (2025-06 → 2026-05), ~91k linhas de TS + ~3,7k de Rust (`rust/crates`: `time`, `gpu`, `compositor`, `effects`, `masks`, `bridge`) compilado para WASM (wgpu 29, shaders WGSL). 30 arquivos de teste TS + testes Rust nos crates de tempo. **MIT** (Copyright OpenCut).

**Modelo de dados (inspecionado):**
- `TScene` = **múltiplas timelines por projeto** (uma `isMain`), cada uma com `tracks: { overlay[], main, audio[] }`. **Não há nested**: nenhum tipo de elemento referencia uma cena.
- Tracks **tipadas** (`video|text|audio|graphic|effect`) com `main` especial — compatível com a nossa decisão de famílias + main magnética, porém mais rígidas.
- Elementos: `startTime/duration/trimStart/trimEnd` em `MediaTime`, animações (keyframes + graph editor), efeitos, máscaras, retime (`rate` + pitch), params.
- **Tempo:** crate Rust `time` com `MediaTime(i64)`, **120.000 ticks/s**, `FrameRate` racional (23,976/24/25/29,97/30/48/50/59,94/60/120 exatos), conversões frame↔tick, timecode, testes. Mesma filosofia da ADR-007. **Limite:** 120.000 não divide 44.100 Hz (amostras de áudio não são exatas) — nosso timebase de 705.600.000 resolve; é um argumento a favor de manter o nosso.

**Command/undo:** padrão `Command` OO com `execute()/undo()/redo()` e `BatchCommand`, operando imperativamente sobre managers (singletons). **Não serializável, sem validação estruturada, sem transações com rollback, sem API para IA.** Selection e preview-tracker acoplados.

**Persistência:** IndexedDB + OPFS, **31 migrations testadas** (maturidade real aqui). Projeto não é um formato editável portátil.

**Render/export:** `scene-builder` → render graph em nós → `CanvasRenderer`/GPU renderer (wgpu via WASM) → `mediabunny` (WebCodecs). Sem frame index/VFR geral (não verificado a fundo; sem evidência de tratamento).

**Timeline UX (o ponto forte):** snapping (`snapping/`, `playhead-snap-source`), group-move/group-resize, ripple (`ripple/`, 377 linhas, **sem React**), placement/overlap/insert-index, retime com split, bookmarks, graph editor de keyframes, ruler/zoom/waveforms. A lógica de `placement`, `ripple`, `retime`, `snapping` **não importa React** (verificado) → portável. A UI em si é **DOM** (`timeline-element.tsx`, `timeline-track.tsx`), sem virtualização identificada.

**Sem:** MCP, API de comandos para IA, headless, nested, multi-seq com referência, provider abstraction, mídia gerada. App desktop (GPUI) praticamente vazio (41 linhas).

**Classificação:** **SAFE_WITH_OBLIGATIONS** (MIT; manter copyright; não usar marca "OpenCut").
**Valor:** o melhor *spec executável* de comportamento de timeline estilo CapCut e uma semente de compositor wgpu; **não** serve como base de produto (stack web/Next, command layer não serializável, arquivado).

---

### 1.3 OpenCut-app/OpenCut (rewrite)

**O que é, de fato:** **scaffold**. 1.599 commits (herdados), mas o código novo: ~7k linhas de TS (shadcn/UI + rota de editor *stub*), um app desktop GPUI com primitivas de UI, `crates/media` contendo **apenas** scripts de setup/pin de FFmpeg. **Nenhum crate Rust de core, nenhum Editor API, nenhum MCP, nenhum headless, 0 testes Rust.** O README lista Editor API, plugins, core Rust, MCP e headless como **"coming"**.

**Aproveitável:** o *pipeline de build de FFmpeg LGPL compartilhado* (`media-deps.yml`, `ffmpeg.json` fixando builds BtbN `*-lgpl-shared` por SHA-256, flags `--disable-programs --disable-autodetect --disable-network`, LICENSE/receita junto da distribuição) é exatamente o padrão para resolver OD-2 de forma conforme à LGPL. MIT.

**Importante:** a direção declarada (core Rust + Editor API + MCP + headless) é a **mesma** da nossa arquitetura — valida a tese, mas **não entrega código**. Decisão de arquitetura dependente dele seria dependência de promessa.

**Classificação:** **REFERENCE_ONLY** (visão) + **SAFE_WITH_OBLIGATIONS** para os scripts de FFmpeg (MIT). Reavaliar em ~6 meses.

---

### 1.4 tjameswilliams/ai-video-editor

**O que é:** o repositório **mais avançado em IA de edição**: Electron + Bun + Hono + SQLite (Drizzle) + React; ~66k linhas de TS; **84 arquivos de teste** (incluindo paridade de export e overlap/sync); 50+ tools de edição, planos multi-etapa, sub-agentes, `agentRuns`, memória de delegação, proveniência de assets, cliente LLM compatível com OpenAI/Anthropic/Ollama, servidor MCP e cliente MCP, Remotion para motion graphics, `undoManager`. 29 commits (2026-03 → 2026-08), v0.1.2.

**Licença:** **PolyForm Noncommercial 1.0.0** — uso comercial proibido; o autor vende a versão comercial (app Mac). Além disso: depende do **Remotion** (licença de empresa obrigatória acima de 3 funcionários), FFmpeg GPL/LGPL via binários estáticos, Chrome Headless Shell.

**Arquitetura (observada):** tempo em `real` (float) no schema; app é um único projeto/timeline por vez (sem sequences múltiplas/nested identificadas); render via FFmpeg + Remotion + Chrome headless (pesado, Mac-first).

**Classificação:** **DO_NOT_USE** (código). Não copiar, não "traduzir" arquivos. Pode servir como evidência de que *o padrão* (tools + planos + provider abstrato + MCP) funciona; as ideias em nível de produto são livres, mas **regra de limpeza:** ninguém que implemente o Brain/Tool System deve basear-se em leitura linha a linha deste código.

---

### 1.5 itsjwill/vanta

**O que é:** pacote Remotion com ~12 módulos "integration" de 40–315 linhas (≈2.000 linhas no total) que são wrappers/interfaces e documentação; 8 commits; README promete substituir Adobe CC, Synthesia, Runway, etc. O "timeline" é um objeto TS de ~190 linhas que referencia bibliotecas de terceiros em comentário. Marketing e funil (comunidade paga) em todo README. `ENHANCEMENTS.md` traz uma tabela de "armadilhas de licença" (útil como pista, **não verificada por mim**).

**Licença:** MIT, mas sem substância reutilizável; Remotion como base.
**Classificação:** **DO_NOT_USE** (sem valor técnico; risco reputacional/licença transitiva). Usar a tabela de licenças apenas como lista de pistas a verificar na fonte.

---

### 1.6 abekyo/abekyo-editor

**O que é:** editor Next.js + Remotion de "cenas" (lista linear de clips com imagem/vídeo, narração por clip, uma BGM, legendas, transições), ~16k linhas TS, 22 arquivos de teste, 16 commits (2026-04 → 08), MIT (Opportunity Inc.). Projeto = **um JSON com JSON Schema publicado**; `POST /api/project/validate` (dry-run), `POST /api/render` (NDJSON de progresso, cancel por disconnect, rate limit), MCP zero-dependência (4 tools), validação de upload por magic bytes.

**Limites:** timeline plana (`clips[]`), sem tracks, sem sequences, tempo em segundos float, render Remotion (licença de empresa; Chrome headless).
**Aproveitável:** padrão *schema publicado + validate + render com progresso NDJSON + MCP fino* como referência para a Fase 6; validação por magic bytes (já prevista em SECURITY §8).
**Classificação:** **REFERENCE_ONLY** (código MIT, mas dependência Remotion e escopo incompatíveis).

---

### 1.7 Outras fontes consideradas (sem auditoria de código — só licença/papel)

Apenas para fechar o escopo; **não inspecionei o código** e as licenças de itens marcados † vêm de conhecimento geral, não de verificação nesta sessão:

| Fonte | Papel potencial | Licença | Classificação provisória |
|---|---|---|---|
| OpenTimelineIO | Interoperabilidade (exportar/importar EDL/timelines), vocabulário de modelo | Apache-2.0 (texto verificado) | **SAFE_WITH_OBLIGATIONS** — candidato a formato de intercâmbio na Fase 6, não como modelo interno |
| mediabunny | Mux/encode WebCodecs no browser | MPL-2.0 (verificado; copyleft por arquivo) | Não aplicável ao pipeline nativo; ok como dependência se um dia houver fallback web |
| Remotion | Render React → vídeo | Licença própria: grátis só ≤3 funcionários/sem fins lucrativos; empresa maior exige licença paga; proíbe derivados para revenda (verificado; **exige confirmação por escrito para embutir em app distribuído**) | **DO_NOT_USE** como dependência do CapIA |
| MLT/Kdenlive, Shotcut, Olive, OpenShot, GES† | Engines/editores desktop completos | GPL/LGPL (família copyleft)† | **DO_NOT_USE** como base de produto comercial fechado, salvo bibliotecas LGPL por linkagem dinâmica após verificação jurídica; **não auditados** |

---

## 2. Matriz de avaliação (critérios solicitados)

Legenda: ✅ sólido · 🟡 parcial · ❌ ausente · — não aplicável. "MD" = MartinDelophy; "OCc" = opencut-classic; "OCn" = OpenCut (rewrite); "TJW" = tjameswilliams; "VAN" = vanta; "ABK" = abekyo.

| Critério | MD | OCc | OCn | TJW | VAN | ABK |
|---|---|---|---|---|---|---|
| Licença p/ uso comercial | ✅ código / ❌ pesos | ✅ MIT | ✅ MIT | ❌ NC | ✅ MIT (vazio) | ✅ MIT (+Remotion ❌) |
| Maturidade | 🟡 (12 sem, 0 testes) | ✅ (1,5k commits) | ❌ scaffold | 🟡 (84 testes, v0.1) | ❌ | 🟡 |
| Atividade recente | ✅ diária | ❌ arquivado | 🟡 esparsa | 🟡 | ❌ | 🟡 |
| Arquitetura | 🟡 browser-only JS | 🟡 web + core Rust parcial | ❌ | 🟡 Electron/Bun | ❌ | 🟡 |
| Data model da timeline | 🟡 coleções fixas, float | ✅ cenas/tracks, ticks | ❌ | 🟡 DB, float | ❌ | 🟡 flat |
| Multi-track | 🟡 camadas/lanes numéricas | ✅ tipadas | ❌ | ✅ | 🟡 stub | ❌ |
| Múltiplas sequences | ❌ | ✅ (`TScene`) | ❌ | ❌ | ❌ | ❌ |
| Nested sequences | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Timeline UX (CapCut-like) | 🟡 | ✅ (melhor) | ❌ | 🟡 | ❌ | 🟡 básica |
| Performance (leitura de código) | ❌ DOM sem virtualização | 🟡 DOM, wgpu p/ render | — | 🟡 | — | — |
| Preview | 🟡 canvas/video element | 🟡 wgpu WASM | ❌ | 🟡 | — | 🟡 Remotion Player |
| Render/export | 🟡 browser WebCodecs | 🟡 browser WebCodecs | ❌ | ✅ FFmpeg+Remotion (Mac) | ❌ | 🟡 Remotion |
| Proxy/media handling | ❌ | 🟡 | ❌ | 🟡 | ❌ | ❌ |
| VFR/frame timing | ❌ (fps inteiro) | 🟡 ticks/FrameRate; sem frame index | ❌ | ❌ | ❌ | ❌ |
| Captions | ✅ (bem rico) | ✅ | ❌ | ✅ | 🟡 | ✅ |
| Transitions | 🟡 | 🟡 | ❌ | ✅ | 🟡 | ✅ |
| Keyframes | 🟡 | ✅ + graph editor | ❌ | 🟡 | ❌ | ❌ |
| Undo/redo | 🟡 snapshots | 🟡 Command OO | ❌ | 🟡 | ❌ | ❌ |
| Persistência / formato editável | 🟡 `.timeline` zip | 🟡 IndexedDB/OPFS + 31 migrations | ❌ | 🟡 SQLite | ❌ | ✅ JSON + schema |
| Command API | ✅ (revisões, idempotência) | ❌ | ❌ | 🟡 tools | ❌ | 🟡 JSON |
| Batch/transactions | ✅ atomic por clone | 🟡 BatchCommand sem rollback | ❌ | 🟡 | ❌ | 🟡 |
| MCP | ✅ (8 + 21 tools) | ❌ | ❌ (prometido) | ✅ | ❌ | ✅ (4) |
| Headless | 🟡 subconjunto | ❌ | ❌ (prometido) | 🟡 server | ❌ | ✅ |
| AI editing | 🟡 externa (via MCP) | ❌ | ❌ | ✅ | ❌ | 🟡 externa |
| Provider abstraction | ❌ | ❌ | ❌ | ✅ (OpenAI-compat/Anthropic) | ❌ | ❌ |
| Media generation | 🟡 plugins (Comfy/WebUI/Puter) | ❌ | ❌ | ✅ | 🟡 wrappers | ❌ |
| Extensibilidade | 🟡 plugins de geração | 🟡 | ❌ | 🟡 | ❌ | ❌ |
| Adequação a desktop/Tauri | ❌ | 🟡 (WASM/webview) | ❌ (GPUI) | ❌ (Electron) | ❌ | ❌ |
| Qualidade de testes | ❌ 0 | 🟡 30 + migrations | ❌ | ✅ 84 | ❌ | 🟡 22 |
| Qualidade de código | 🟡 JS denso, funções longas | ✅ TS organizado por domínio | — | 🟡 | ❌ | 🟡 |
| Dependências/risco de licença | ❌ pesos, libav LGPL | ✅ baixo | ✅ baixo | ❌ Remotion, NC | ❌ Remotion | ❌ Remotion |

---

## 3. Mapa de requisitos do CapIA × cobertura existente

Prioridades máximas informadas por você:

| # | Requisito | Melhor cobertura existente | Lacuna para o CapIA |
|---|---|---|---|
| 1 | Timeline manual ≈ CapCut | OCc (lógica) | UI em canvas virtualizada; nested; N tracks |
| 2 | Tudo editável após a IA | MD (planos viram clips comuns) | OK conceitualmente; modelo ainda precisa de nested/tracks |
| 3 | Humano e IA na mesma timeline | MD (shared command engine) | Rebase/conflito, ops primitivas com inversa (nosso design já supera) |
| 4 | Múltiplas sequences | OCc (`TScene`) | Referência/nested/propagação/ciclos |
| 5 | Múltiplas tracks | OCc | Famílias + role + sem limite |
| 6 | Velocidade | nenhuma (todas DOM/browser) | Canvas + core Rust + WASM |
| 7 | Command/API layer | MD | Ops primitivas + undo por inversa + transações multi-sequence |
| 8 | Planejamento + batch | MD (plano de ≤500 ops), TJW (planos) | Pipeline com gate e dry-run |
| 9 | MCP/API futura | MD, ABK | Mesma Engine API (já prevista) |
| 10 | Brain selecionável | TJW (NC) | Construir do zero (Fase 4) |
| 11 | Local-first | MD, TJW | OK por arquitetura |
| 12 | Comercial futuro | OCc, OCn, MD(código) | Sem GPL/NC/Remotion no caminho crítico |

**Nenhum candidato cobre sequer 3 dos 6 requisitos de modelo (sequences + nested + N tracks + Ticks exatos + VFR frame-exato + core headless).**

---

## 4. Conflitos com a arquitetura de M01

| Conflito | MD | OCc | TJW | ABK |
|---|---|---|---|---|
| C1 core Rust headless | ✗ (tudo JS no browser) | ✗/parcial (lógica em TS; só tempo/GPU em Rust) | ✗ (Bun) | ✗ |
| C2 preview+export nativos, compositor único | ✗ (browser canvas/WebCodecs) | ✗/parcial (wgpu via WASM; export WebCodecs) | ✗ (FFmpeg+Remotion) | ✗ |
| C3 timeline em canvas | ✗ (DOM) | ✗ (DOM) | ✗ | ✗ |
| C4 IA/segredos no Rust | ✗ | — | ✗ (Bun) | — |
| C5 core em WASM | ✗ | ✓ parcial (crate `time` com feature `wasm`) | ✗ | ✗ |
| ADR-007 Ticks 705,6M | ✗ float | ≈ (120k; sem 44,1 kHz exato) | ✗ float | ✗ float |
| ADR-008 tracks Visual/Audio+role | ✗ coleções | ≈ tipadas rígidas | ≈ | ✗ |
| ADR-010 nested | ✗ | ✗ | ✗ | ✗ |
| ADR-013 ops primitivas + inversas | ✗ (snapshots) | ✗ (undo OO) | ✗ | ✗ |
| ADR-015 `.capia` SQLite | ✗ zip de blobs | ✗ IndexedDB | ≈ SQLite (outro schema) | ✗ JSON |

Um fork de qualquer um exigiria **reverter ou reescrever** ao menos 4–5 dessas decisões — não apenas "adicionar".

---

## 5. Comparação das estratégias

Critério econômico: `tempo de adaptação + dívida técnica + risco de licença + dificuldade futura` **vs** `tempo de construir corretamente`.

| | A — Do zero | B — Fork de uma base | C — Base própria + componentes selecionados | D — Fork + substituição gradual do core |
|---|---|---|---|---|
| Aderência a M01 | Total | Baixa (reverte C1–C5 e 5 ADRs) | Total | Média no fim, baixa no meio |
| Tempo até algo "visível" | Mais lento (sem UI até Fase 3) | **Mais rápido** (editor web já roda) | Igual a A | Rápido no início |
| Tempo até o produto *desejado* | Linha de base | **Maior ou igual a A** (você paga a base e depois substitui quase tudo) | **Ligeiramente menor que A** | Maior que A (dois sistemas coexistindo durante a troca) |
| Dívida técnica | Mínima | Alta (JS sem testes / Next+IndexedDB) | Mínima | Alta e prolongada |
| Risco de licença | Mínimo | MD: pesos/libav; OCc: baixo | **Baixo** (só MIT/Apache com avisos; nada de NC/Remotion/GPL) | Igual a B |
| Dificuldade futura | Baixa | Alta (upstream arquivado ou divergente) | Baixa | Média-alta |
| Reaproveita aprendizado alheio | Não | Sim (código) | **Sim (comportamento, testes e design)** | Sim |
| Risco de execução | Escopo grande | Reescritas "silenciosas" | Moderado | Alto (difícil de terminar a substituição) |

**Por que B/D perdem aqui (e não em geral):** o valor de um fork vem de herdar o *núcleo*. Nos candidatos, o núcleo é justamente o que não serve: tempo float, coleções fixas, command layer não serializável/ snapshots de undo, persistência de browser, render de browser. O que sobra aproveitável (UI DOM, estilos, i18n) é exatamente o que o requisito C3 (canvas, virtualização, 60 fps) manda refazer, e a UI do CapIA precisa de design system próprio. O custo de entender e expurgar 40–90k linhas de código alheio, sem testes (MD) ou com stack web (OCc), supera o benefício.

**Por que A puro perde para C:** há três ativos MIT com valor real e baixo custo de integração (tempo/geometria de timeline testados, semente de compositor wgpu, padrão de build FFmpeg LGPL) e um corpo de design de API (MD) que reduz erros de contrato. Ignorá-los é desperdício; adotá-los *como referência/semente* não compromete a arquitetura.

---

## 6. Estratégia recomendada: **C — base própria + componentes selecionados (reuso como spec e semente, nunca como fundação)**

> Recomendação com confiança **alta** para "não forkar" e **média** para o tamanho do ganho (ver §8). A arquitetura de M01 permanece válida; as mudanças sugeridas em §9 são aditivas.

### 6.1 Mapa `subsistema CapIA → fonte → estratégia`

| Subsistema CapIA | Fonte | Estratégia de reuso |
|---|---|---|
| `capia-time` (Ticks/Rational/FrameRate) | OCc `rust/crates/time` | **Reimplementar com nosso timebase (705,6M)**; usar API (`from_frame`, `round_to_frame`, `last_frame_time`, timecode) e **casos de teste** como oráculo. MIT, atribuir se copiar trechos |
| `capia-model` / invariantes | OCc `timeline/types.ts`, `scenes.ts`; MD `projectCommandEngine` (inspect/diff) | **Referência de vocabulário.** Modelo próprio (famílias+role, nested, clips com `AssetRef`) |
| `capia-commands` (comandos, transações, undo) | MD (formato de plano, idempotência, diff, erros), OCc (BatchCommand só como contraexemplo) | **Reimplementar.** Adotar ideias: `operationId` idempotente e `describe/diff` (ver §9) |
| Regras de edição (placement, overlap, ripple, retime, split, snapping, group-move) | OCc `timeline/placement`, `ripple`, `retime`, `snapping`, `group-*` (+ testes) | **Portar comportamento**: traduzir para o core Rust (regras) e para `ui-timeline` (ghost/snap via WASM). Usar os testes como casos de aceitação. MIT com atribuição |
| Keyframes/interpolação | OCc `animation/` (3,1k linhas, testes) | **Referência + oráculo de teste**; avaliação em Rust no core |
| `ui-timeline` (canvas) | OCc `timeline/components` (DOM) | **Referência de UX e hit-testing**; implementação nova em canvas |
| `capia-render` compositor (wgpu) | OCc `rust/crates/{gpu,compositor,effects,masks}` (~3,7k linhas, WGSL, wgpu 29) | **Semente candidata**, condicionada a spike (S6): rodar nativo (não só WebGL), encaixar no nosso Render Graph, 16-bit float linear, text engine separado. Se não encaixar: usar como referência de shaders de blend/máscara/feather |
| FFmpeg build/licença (OD-2) | OCn `crates/media/setup` + `media-deps.yml` + `ffmpeg.json` | **Adotar o padrão**: builds LGPL *shared* fixadas por SHA-256, flags minimalistas, LICENSE+receita no pacote. Verificar encoders HW nas builds BtbN no spike S3 |
| `capia-store` / migrations | OCc `services/storage/migrations` (31 + testes) | **Referência de disciplina** (um arquivo por versão, teste por migration, fixtures); implementação em Rust/SQLite |
| Tool system / MCP / API (Fase 6) | MD `docs/webmcp.md`, `references/command-contract.md`, `mcp/server.mjs`; ABK `docs/headless.md` + schema | **Referência de contrato**: `inspect → preview(dry_run) → apply`, `stateToken`/`previewId` opacos, `requestId` em ações caras, paginação, JSON Schema publicado, progresso NDJSON. MIT |
| Legendas (agrupamento por palavra, estilo), remoção de silêncio | MD (`captionEditingActions`, `docs/silence-removal.md`), ABK | **Referência de requisitos**; reimplementar com testes. Validar qualidade antes (MD sem testes) |
| Reference Analyzer | MD `analyze_replication.py` (289 linhas, OpenCV: limiar robusto por mediana/MAD), TJW `sceneDetection` (NC, não ler) | **Referência de técnica** (limiar robusto); implementar nosso pipeline (Fase 4) |
| AI Orchestrator / Brain / Providers / Router | TJW (padrão; **NC — não usar**) | **Construir do zero.** Avaliar crates Rust de cliente LLM na Fase 4 (**não auditados aqui**) |
| Asset system / dedup / relink / proveniência | nenhum | Construir (TJW tem `assetProvenance`, NC) |
| UI shell / design system | — | Próprio; nada a reaproveitar |
| Interchange (EDL/timelines externas) | OpenTimelineIO (Apache-2.0) | **Opcional na Fase 6** como import/export; não como modelo interno |

### 6.2 Módulos: manter / remover / reescrever (da perspectiva "se alguém ainda considerar um fork")

- **Se a base fosse OCc (a melhor):** *manter* `rust/crates/{time*,gpu,compositor,effects,masks}`, lógica pura de timeline e testes; *remover* Next/better-auth/DB/Redis/Cloudflare/site/blog, DOM da timeline, managers com singletons; *reescrever* command layer, persistência, export, modelo (nested, ticks, tracks), UI em canvas. **Sobra ≈ 10–15% do repositório** — isso é uma "extração de componentes" (estratégia C), não um fork.
- **Se a base fosse MD:** *manter* ideias do command engine e MCP; o resto (JS, browser, blobs) **reescrever**. Sobra < 5% como código.

### 6.3 O que ainda falta do CapIA depois do reuso (tudo)

Core Rust completo (modelo, comandos, store SQLite, jobs), nested/ciclos, frame index/VFR, decode/seek exatos, mixer de áudio, proxies, presenter nativo, export H.264/HEVC nativo, Asset System/Gateway, AI Brain/Providers/Router/Tools/agentes, Reference Analyzer, memória, segurança de credenciais, MCP/REST. O reuso reduz *descoberta e risco* em timeline/render; **não reduz** o grosso do trabalho.

### 6.4 Impacto no roadmap

Nenhuma fase muda; ajustes **aditivos** (não aplicados a `ROADMAP.md`):
- **Fase 1 / M03 (spikes):** acrescentar **S6 — avaliar semente de compositor** (`opencut-classic` `gpu/compositor/effects/masks` em wgpu nativo) e **S7 — oráculo de comportamento**: extrair casos de teste de `placement/ripple/retime/snapping` como suíte de aceitação do `capia-commands` (execução em Rust/TS, sem copiar código de produção).
- **S3 (FFmpeg):** incluir verificação de encoders de hardware nas builds LGPL pinadas (padrão OCn) como parte de OD-2.
- **Fase 2:** critério adicional — suíte de aceitação derivada de OCc passa no `capia-commands`.
- **Fase 6:** usar `docs/webmcp.md`/`command-contract.md` (MD) e `headless.md` (ABK) como checklist de contrato.

---

## 7. Licenças — achados e regras

| Fonte | Licença | Classificação | Obrigações / riscos |
|---|---|---|---|
| MD (código) | MIT | SAFE_WITH_OBLIGATIONS | Manter copyright/aviso se copiar trechos |
| MD (modelos, pesos, mídia, fontes) | Variadas; **pesos de face swap = pesquisa, sem uso comercial** | **DO_NOT_USE** sem verificação individual | `MODEL_LICENSES.md` isenta-os da MIT |
| MD libav.js embutido | LGPL-2.1+ (ISC no loader) | SAFE_WITH_OBLIGATIONS (se um dia) | Linkagem/relink, texto da licença |
| OCc | MIT | SAFE_WITH_OBLIGATIONS | Manter copyright "OpenCut"; **não usar nome/marca**; revisar licenças das deps (fontes, stickers, sons e demais assets em `public/`/`src`; não auditados arquivo a arquivo) antes de copiar qualquer asset |
| OCn | MIT | SAFE_WITH_OBLIGATIONS (scripts FFmpeg) / REFERENCE_ONLY (resto) | FFmpeg LGPL: shared, texto da licença e receita no pacote |
| TJW | **PolyForm Noncommercial 1.0.0** | **DO_NOT_USE** | Proíbe uso comercial; não copiar nem traduzir código; Remotion; Chrome Headless Shell |
| VAN | MIT (sem substância) | DO_NOT_USE | Remotion; claims não verificáveis |
| ABK | MIT | REFERENCE_ONLY | Dependência Remotion (licença de empresa) |
| **Remotion** | Licença própria: grátis ≤3 funcionários; acima, licença paga; veda derivados para revenda | **DO_NOT_USE** no CapIA | Nunca no caminho crítico; embutir exigiria acordo escrito |
| mediabunny | MPL-2.0 | n/a (nativo) | Copyleft por arquivo se usado |
| OpenTimelineIO | Apache-2.0 | SAFE_WITH_OBLIGATIONS | Aviso + NOTICE; patentes concedidas |

**Regras de proveniência para qualquer reuso futuro** (propostas):
1. Arquivo `THIRD_PARTY.md` por componente copiado: origem, commit, licença, modificações.
2. Copiar apenas de fontes SAFE_WITH_OBLIGATIONS, com cabeçalho de atribuição no arquivo.
3. **Clean-room** para qualquer coisa em REFERENCE_ONLY/DO_NOT_USE: especificar comportamento em documento próprio e implementar sem copiar.
4. Nenhum asset (fonte, som, sticker, modelo) vindo desses repos sem licença individual verificada.
5. Verificar licenças de dependências transitivas (`cargo-deny`, `license-checker`) no CI (M03).

---

## 8. Economia estimada de engenharia

Premissas: equipe pequena e sênior; o escopo de Fases 2–3 (motor + editor manual) estimado, grosso modo, em **12–20 pessoa-meses** do zero. **Estimativas por julgamento, não medições**; as faixas pressupõem que os spikes S6/S7 confirmem o encaixe.

| Reuso | Economia estimada | Justificativa |
|---|---|---|
| Tempo (`capia-time`) | 0,5–1 pessoa-semana | Crate pequeno; ganho em casos de teste e API |
| Regras de edição como oráculo (placement/ripple/retime/snapping/group-move) | 3–6 pessoas-semanas | Evita redescobrir casos-limite; testes prontos; **não** economiza a UI em canvas |
| Semente de compositor wgpu | 0–5 pessoas-semanas (**incerta; pode ser 0**) | Depende de S6; shaders de blend/máscara/feather aproveitáveis mesmo se o resto não encaixar |
| Pipeline FFmpeg LGPL (OD-2) | 1–2 pessoas-semanas | Evita desenhar build/pin/licença do zero |
| Contrato de API/MCP (MD/ABK) | 1–2 pessoas-semanas (Fase 6) | Menos retrabalho de contrato |
| Disciplina de migrations (OCc) | < 1 pessoa-semana | Padrão, não código |
| **Total (estratégia C)** | **≈ 6–16 pessoas-semanas ≈ 3–10% de Fases 2–3** | Ganho real, modesto, de baixo risco |

Alternativas (para contraste, mesma incerteza):
- **Fork (B/D)** parece economizar "6–10 meses de UI/editor" no papel, mas o *líquido* após reescrever tempo, modelo, comandos, persistência, render e UI em canvas fica entre **~0% e 20%**, com **risco alto** de acabar com dois sistemas e dívida técnica. Não recomendado.
- **A (zero)**: sem economia, sem risco de licença; perde só os ~6–16 pessoas-semanas acima.

O que o reuso **não** economiza e domina o custo: core Rust, frame-accuracy/VFR, presenter nativo, Asset System, IA/agentes, segurança, testes.

---

## 9. Emendas propostas à arquitetura (NÃO aplicadas — requerem decisão)

Aprendizados desta auditoria que *melhorariam* M01. Cada item seria um ADR novo/atualização:

1. **Idempotência de operações** (de MD): cada transação/comando aceita `operation_id`/`idempotency_key`; reenvio após falha de rede (REST/MCP/AI) não duplica efeitos. M01 só tem `base_revision`. *Proposta de ADR.*
2. **Preview/apply com token** (de MD): `dry_run` retorna `preview_id` + `state_token`; `apply(preview_id)` aplica *exatamente* o plano revisado e falha se o estado mudou. M01 tem `dry_run` mas não o vínculo preview→apply. *Proposta.*
3. **Timebase:** manter 705.600.000 (OCc com 120.000 não representa 44,1 kHz exatamente). *Sem mudança — reforça ADR-007.*
4. **Limites de plano** (de MD): reduzir default de `max_ops` por transação de IA e exigir paginação em `timeline.get_state`; M01 já prevê limites — alinhar números no Fase 2.
5. **Suite de aceitação de comportamento** derivada de OCc (S7), como critério da Fase 2.
6. **Camada de provedores de tempo para áudio**: nada a mudar; apenas confirmar nos spikes.

Nada disso muda a stack ou as condições C1–C5.

---

## 10. O que NÃO reutilizar

- **Código de TJW** (PolyForm NC) e **qualquer coisa Remotion** (licença de empresa).
- **VAN** inteiro.
- **Pesos/modelos/mídia** de MD (face swap, lip-sync, vozes) sem verificação individual; e o conjunto de funções de IA de MD (fora do escopo DR/UGC V1).
- **A timeline em DOM** de MD e OCc (contradiz C3) — apenas a lógica e o comportamento.
- **Command layer e persistência** de OCc/MD (não serializáveis / de browser).
- **Seek por `HTMLVideoElement.currentTime`** e **fps arredondado** de MD (incompatíveis com ADR-007 e frame-accuracy).
- **Dependências de plataforma web** (Next, Cloudflare, better-auth, Redis) de OCc.
- **Marca/nome "OpenCut"**; copiar identidade visual de qualquer fonte.
- Qualquer encoder/biblioteca GPL no caminho do produto fechado (x264 etc.) — decisão OD-2.

---

## 11. Riscos desta auditoria

- Sem execução/benchmark: a qualidade de UX/desempenho de OCc/MD foi inferida do código.
- Repositórios mudam rápido (MD comita diariamente; OCn pode entregar o core prometido). **Reavaliar OCn/MD em ~6 meses** (checkpoint em Fase 3).
- Clones rasos: contagens de commit e autores podem estar incompletas.
- Licenças de ativos (fontes, sons, stickers) em OCc **não** foram auditadas arquivo a arquivo.
- Itens de §1.7 marcados † não foram verificados nesta sessão.

---

## 12. Decisões abertas (para o dono do produto)

1. **Aprovar a estratégia C** (e o abandono de fork como caminho).
2. **Autorizar spikes S6 e S7** em M03/Fase 1 (nada de cópia de código até lá).
3. **Aprovar as emendas §9** (1 e 2 em especial) para virarem ADRs antes da Fase 2.
4. **OD-2 (licença do produto)** continua sendo bloqueante e agora também define se builds LGPL do padrão OCn bastam.
5. **Regra de provenance/clean-room** (§7) como política do repositório.
