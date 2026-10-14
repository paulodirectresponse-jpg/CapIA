# Fase 2A — OpenCut Adoption Gate + integração segura

Branch: `integration/opencut-classic` (nunca mesclada em `main`). Referências de pesquisa: Fase 1 `81765826`, Fase 1B `786f4e21`; baseline `OpenCut-app/opencut-classic@cf5e79e9`. Decisão do PO: `ADOPT_OPENCUT_CLASSIC_WITH_MAJOR_CHANGES`.

> Regra de honestidade da fase: nenhum gate humano/externo foi marcado como passado sem evidência. Onde só há ambiente de software (SwiftShader) ou Linux, o veredito diz isso.

## 1. Preservação
- `git fetch --all --prune`; `legacy/core-from-scratch` criada a partir de `claude/rc3-free-timeline-ux` (SHA `ff45503`) e enviada. Tag `legacy-core-rc3` criada **localmente**; o push de tags falha neste ambiente (`unexpected disconnect while reading sideband packet`, anotada e leve) — a branch carrega o mesmo SHA. Nada foi apagado nem sofreu force-push.

## 2. Import do upstream
- `git subtree add` **sem squash** em `apps/editor/` (histórico completo); remote `upstream-opencut`; tag local `upstream-opencut-cf5e79e9`. Workspace bun do editor isolado de pnpm/cargo (`pnpm-workspace.yaml`: `!apps/editor`; `Cargo.toml`: `exclude`).
- SaaS removido (auth/DB/Redis/Marble/Freesound/analytics): build sem `.env`. Next mantido.
- Correções da 1B como commits separados (`03ec6927`), testes conhecidos resolvidos (`64af6bbb`).

## 3. Licenças (gate I — solucionável)
- OpenCut: MIT, mantida (`THIRD_PARTY_NOTICES.md`, `docs/PROVENANCE.md` E1B-1/E2A-1).
- Varredura de `apps/editor/node_modules` (licenças declaradas): sem GPL/AGPL/SSPL. Achados:
  | Item | Licença | Tratamento |
  |---|---|---|
  | `soundtouchjs` (retime de áudio, em `retime/audio-stretch.ts`) | LGPL-2.1 | **Risco real** em bundle JS. Fase 2B: manter como módulo substituível/dinâmico + aviso, ou trocar por implementação própria. Não bloqueia 2A. |
  | `sharp` + `@img/sharp-libvips-*` | LGPL-3.0 (binário nativo, dinâmico, otimização de imagem do Next) | Não entra no desktop/WebView; remover `sharp` no 2B. |
  | `@better-fetch/fetch`, `botid` | sem campo `license` | Resíduos de `node_modules` (não estão em `package.json`/`bun.lock`). |
  | Modelo Whisper e fontes do editor | não auditados | Pendência 2B (registrar em PROVENANCE). |
- Rust: `cargo deny check licenses bans sources` roda no job `rules` do CI.

## 4. Build base (gate A) e testes (gate B)
- `bun install` → `tsc --noEmit` limpo → `bun test`: **245 pass / 0 fail** (218 no início; +27 de contrato/snapping/virtualização) → `next build` OK sem `.env`. Compositor WASM: `wasm-pack build rust/wasm` (job `wasm`). Arquivos novos/alterados passam no lint do editor.
- Dois shims de teste (Bun não importa `.wasm` como ESM; `OffscreenCanvas` determinístico para medir texto): `apps/editor/bunfig.toml`.

## 5. AgentSurface v1 (gate C) e garantias de comando
Código: `apps/editor/apps/web/src/agent-surface/`. Operações: `inspect_project/sequence/tracks/clip`, `insert/move/trim/split/delete`, `undo/redo`, `validate_plan/preview_plan/apply_plan`. Plano: `schemaVersion`, `planId`, `actor`, `baseRevision`, `operations[]` com `operationId`; referências internas do plano (`ref`, `rightRef`, `clipRef`).

| Garantia | Implementação |
|---|---|
| ids injetáveis | `utils/id.ts`: `withIdSeed`, `deterministicUuid`; ids derivados por `planId+operationId` |
| erros estruturados | `CommandError` (código, mensagem, operação) |
| no-op fora do histórico | `CommandManager` strict/no-effect; teste dedicado |
| batch atômico + rollback | `BatchCommand`: falha ⇒ desfaz tudo |
| revisão/CAS | `RevisionManager` + `document-fingerprint`; `baseRevision` divergente ⇒ `STALE_REVISION` |
| idempotência | mesmo `planId` reaplicado devolve o resultado anterior sem reexecutar |
| metadados no histórico | ator, planId, operationId, revisão (`getHistory`) |
| preview/validate | executa e reverte (sem poluir histórico) |
Transportes: stdio (host Bun), `window.__capiaAgent` (página), HTTP loopback — um despachante RPC único (`bridge/dispatch.ts`).

## 6. Paridade humano ↔ programa (gate D)
`__tests__/contract.test.ts` aplica a mesma edição por (a) comandos da UI e (b) AgentSurface e compara o documento (fingerprint) e o histórico: **PASS**. E2E `apps/editor/e2e/first-flow.mjs` (Playwright, UI real): humano cria projeto/importa/insere; CapIA (EnginePort → CLI Rust) inspeciona, divide, move, apara; Ctrl+Z na UI desfaz a ação do agente — um único documento, um único histórico. Evidência: `docs/adoption/evidence/first-flow/`.

## 7. EnginePort (gate E) — `ENGINEPORT_ADAPTER_FEASIBLE`
`crates/capia-engine-port`: a trait `Engine` real de `capia-intelligence` implementada sobre a AgentSurface (`AgentSurfaceEngine`), com tradutor do vocabulário legado de comandos e conversão de unidades (ver §12). Testes contra o domínio OpenCut **real** via Bun (`tests/real_opencut.rs`, `CAPIA_REQUIRE_BUN=1` no CI). Para isso `capia-intelligence` foi desacoplado da `Session` Rust: adaptador legado atrás da feature `session-engine`; tipo de ator no crate folha `capia-actor`. Pendência 2B: cobrir o restante do vocabulário (transições, legendas auto, etc.).

## 8. H.264/AAC no WebView2 (gate F)
**`H264_WEBVIEW2_PASS`** — medido num WebView2 **real** (runtime 153.0.4234.48) em `windows-latest` (run 38487826620; evidência `docs/adoption/evidence/codec/webview2-verdict.json`). O mesmo caminho do exportador do editor (mediabunny: `CanvasSource avc` + `AudioBufferSource aac`) gerou MP4 verificado com ffprobe: h264 High 640×360@30, AAC 48 kHz estéreo, 2,005 s. Controle: Chromium do Playwright (sem codecs proprietários) reporta `H264_FAIL`, ou seja, o harness distingue falha de sucesso.
Limites honestos: (a) runner sem GPU — só o encoder **software** do Media Foundation foi exercido (`prefer-hardware` = false); encoder por hardware em PCs de usuário não foi medido; (b) o spike valida 640×360/2 s, não um export 1080p longo; (c) `pix_fmt yuvj420p` (full range) — conferir o range de cor no 2B. Plano B (FFmpeg LGPL híbrido) **não foi necessário**, segue documentado (ADR-129). O exportador do editor ainda faz fallback silencioso AAC→Opus: tornar explícito no 2B.

## 9. GPU real (gate H)
Harness `apps/editor/e2e/gpu-parity.mjs` (cena: vídeo + overlay, transform/opacidade, máscara elíptica, blur, texto, keyframes; compara snapshot do compositor × tela × export por PSNR). Neste ambiente só há **SwiftShader** (WebGPU indisponível, fallback WebGL2; export ~11× mais lento que tempo real).
- Veredito: **`GPU_PARITY_PENDING_HUMAN_MACHINE`** — não é PASS e não prova defeito de GPU.
- Medido em software: snapshot × tela PSNR 26–31 dB (passa); **snapshot × export PSNR 14–19 dB (abaixo do limiar)**: nos quadros extraídos o conteúdo do vídeo-fonte aparece em instante diferente do snapshot (texto/máscara coincidem). Causa não isolada (suspeitas: seek do webm sem cues, descarte de quadros do export em renderer lento). Registrado como achado aberto para rodar em máquina Windows com GPU (`GPU_REAL=1`, job manual `gpu-parity`). Evidência: `docs/adoption/evidence/gpu/`.
- Nota: o JSON de evidência foi regenerado a partir dos checks reais da execução aplicando a classificação final (software ⇒ PENDING com `softwareFindings`).

## 10. Virtualização (gate G) — `VIRTUALIZATION_FEASIBLE`
`timeline/virtualization.ts` (janela de tempo + overscan + chunk + ids fixados) sem alterar o domínio. Medição (`docs/adoption/evidence/virtualization/perf.json`, Chromium/SwiftShader):

| Clips | Layout | DOM base → virt. | Heap MB | scroll p95 ms | split ms |
|---|---|---|---|---|---|
| 1 000 | sequencial | 10 623 → 1 723 | 233 → 76 | 417 → 233 | 515 → 161 |
| 5 000 | sequencial | 50 623 → 1 723 | 962 → 78 | 2000 → 217 | 2160 → 195 |
| 10 000 | sequencial | 100 623 → 1 721 | 1842 → 86 | 5317 → 217 | 6203 → 227 |
| 5 000 | denso (muitas faixas) | 52 153 → 13 153 | 964 → 260 | 3917 → 717 | 2229 → 1222 |
Limite conhecido: layout denso exige virtualização vertical de faixas (Fase 2B); não requer reescrita do domínio.

## 11. Snapping
`timeline/__tests__/snapping.test.ts`: borda→playhead, borda→borda, desativado, empate — comportamento fixado.

## 12. Timebase (ADR-130)
OpenCut: 120 000 ticks/s (inteiro). CapIA: 705 600 000/s. Razão 5880 (inteira). 120 000 é exato para 24/25/30/48/50/60 fps e NTSC; **não** para amostras de áudio 44,1/48 kHz. Decisão: manter 120 000 no editor, converter exatamente na fronteira EnginePort; áudio mantém contagem de amostras própria.

## 13. CI
Workflow `.github/workflows/opencut-integration.yml` (push em `integration/**` + manual). Jobs: `editor` (install/tsc/testes/contrato/build), `wasm`, `engine-port`, `rules` (arquitetura core + editor, licenças JS/Rust), `codec-webview2` (Windows), `gpu-parity` (manual). `ci.yml` legado intocado.

### Estado do CI (evidência por job — runs reais no GitHub Actions)
| Job | Resultado | Onde |
|---|---|---|
| `editor` (install, tsc, 245 testes, contrato AgentSurface, `next build`) | verde | run 38487826620 (completo) e run 38488194148 (tsc/testes/contrato verdes; build em andamento ao fechar este relatório) |
| `wasm` (`wasm-pack build rust/wasm`) | verde | run 38487826620 |
| `engine-port` (cargo test EnginePort × OpenCut real com `CAPIA_REQUIRE_BUN=1`) | passo de testes verde no run 38487826620; passo final (`--no-default-features`) não observado até o fim em CI (verificado localmente) | run 38487826620 |
| `codec-webview2` (Windows) | **verde / H264_PASS** | run 38487826620 |
| `rules` (arquitetura core+editor, licenças JS, `cargo deny`) | verde | run 38488194148 (após corrigir 2 achados reais: fixtures de `check-architecture.test.mjs` e dependência sem versão em `capia-engine-port`) |
| `gpu-parity` | manual (não roda no push) | — |
Os runs 1 e 2 foram cancelados por `concurrency` quando um push novo chegou; o run 3 (HEAD de código) estava em andamento ao fechar. Nenhum job do HEAD de código falhou até então.

## 14. Pendências assumidas para a Fase 2B (nenhuma bloqueia o veredito)
1. Paridade export × snapshot em GPU real (achado aberto em software) — rodar `gpu-parity` em PC Windows com GPU.
2. Encoder H.264 por hardware e export 1080p longo no WebView2; range de cor `yuvj420p`; tornar explícito o fallback AAC→Opus.
3. `soundtouchjs` (LGPL-2.1) e `sharp` (LGPL, nativo) — resolver; auditar Whisper/fontes.
4. Virtualização vertical de faixas (layout denso).
5. Vocabulário legado restante no EnginePort; `capia-store` ainda puxa `capia-commands/model` transitivamente.
6. Tag `legacy-core-rc3` no remoto (push de tags falha neste ambiente).
7. Persistência: o OpenCut usa IndexedDB/OPFS; decidir/armazenar projeto local no desktop (Tauri) na 2B.
