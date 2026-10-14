# Fase 2B — fechamento dos riscos abertos da 2A (andamento)

Base: `integration/opencut-classic`; fase 2A fechada com `GO_PHASE_2B` e CI verde no run 38488194148 (commit de código `e5a6a307`; `ed5ded27` é só documentação, `[skip ci]`). `main` intocada. Estado preservado em `snapshot/phase-2a-complete` (branch, remoto) e na tag local `phase-2a-complete`.

| # | Item | Estado | Evidência |
|---|---|---|---|
| 1 | GPU parity em Windows com GPU física | **PRONTO PARA RODAR NA SUA MÁQUINA** — não executado; veredito continua `GPU_PARITY_PENDING_HUMAN_MACHINE` | `tools/gpu-parity/run-windows.ps1`; harness agora usa Chrome/Edge instalado, janela visível, `--enable-unsafe-webgpu` |
| 2 | `soundtouchjs` e `sharp` | **Resolvido** | ADR-131; `retime/wsola.ts` + 7 testes; `bun.lock` sem sharp/soundtouch; build e 271 testes verdes |
| 3 | Whisper e fontes | **Parcial (honesto)** | `AUDIT_ASSETS.md`: fontes remotas agora opt-in (corrigido e testado); licenças dos cards Whisper e por família de fonte **não verificáveis aqui** (egress bloqueado) — marcadas VERIFICAR |
| 4 | AAC→Opus explícito + export 1080p longo | **AAC explícito: resolvido e testado. 1080p longo: `H264_PASS` no WebView2 real (CI Windows)** | ADR-133; `export/audio-codec.ts`; run 38497167119, job `codec-webview2`: 1920×1080 @30, 20,01 s, 600/600 quadros, H.264 High + AAC 48 kHz estéreo, deriva A/V dentro do limite, encode em 4,8 s (encoder **software** do runner sem GPU; hardware segue sem medição) |
| 5 | Virtualização vertical | **Resolvido** | denso 5000 clips: DOM 13 153→3 686, heap 260→90 MB, scroll p95 717→283 ms, split 1 222→208 ms, move 2 537→366 ms (`evidence/virtualization/perf-vertical.json`) |
| 6 | Ampliar EnginePort | **Resolvido para o que o Editor Agent emite e o OpenCut suporta** | ADR-134; teste de integração real (texto+legenda+enquadramento, 1 plano = 1 histórico); transições e multi-sequence ficam **explicitamente** sem suporte |
| 7 | Persistência desktop | **Decidido + formato implementado/testado**; integração no shell Tauri pendente | ADR-132; `project-file/format.ts` (8 testes) |

## Como rodar o item 1 (precisa da sua máquina)
Windows com GPU física, Git, Node 22+, Bun, FFmpeg no PATH, Chrome ou Edge:
```
git fetch origin integration/opencut-classic && git checkout integration/opencut-classic && git pull
.\tools\gpu-parity\run-windows.ps1 -Channel chrome      # ou -Channel msedge
```
Saída: `apps\editor\gpu-parity-out\gpu-parity-verdict.json` + PNGs. Se o renderer for software o próprio harness reporta `PENDING_HUMAN_MACHINE` (nunca PASS). Envie a pasta `gpu-parity-out` (zip).

## O que falta (não bloqueado pela sua máquina)
- Hospedar o editor no shell desktop e implementar `project_open/save` no host Rust (ADR-132).
- UI do opt-in de fontes remotas e do fallback de áudio (ADR-131/133).
- Fixar `revision` e confirmar licença dos modelos Whisper; trocar logos OpenCut pela marca CapIA.
- Decisão de produto: transições e multi-sequence (ADR-134).
- Investigar a divergência export × snapshot achada em software (pode ser do pipeline de export, independente de GPU).

## CI
Runs `ceb474f8` (38497167119) e `b654de63` (38497486640): todos os jobs verdes (`editor`, `wasm`, `engine-port` completo, `rules`, `codec-webview2`). Os commits seguintes (`5c95fe9c` EnginePort v1.1, `1db0b024` formato de projeto/ADRs) foram enviados; o resultado desses runs deve ser conferido no GitHub Actions (em andamento ao escrever).

---

# Fase 2B.1 — bug de paridade preview / snapshot / export (MUTABLE VIDEO TEXTURE CACHE)

## GPU física testada (rerun do usuário)
Máquina Windows física, GPU AMD (WebGL2 `ANGLE / AMD Radeon / Direct3D11`; WebGPU vendor AMD, arquitetura RDNA2), `softwareRenderer=false`, H.264=true. Cena/transform/máscara/blur/texto/keyframes/export MP4 = PASS. Resultado anterior: **`GPU_PARITY_FAIL`** — preview×snapshot passava (PSNR 29–40 dB), export×snapshot falhava em todas as amostras (PSNR 14–20 dB). Os PSNR de 3,5 s e 5,5 s do preview/snapshot eram **idênticos** (29,551182), o que já indicava quadro congelado.

## Causa raiz (provada antes de corrigir)
`ROOT_CAUSE = PREVIEW_SNAPSHOT_MUTABLE_TEXTURE_CACHE` (`MUTABLE_CANVAS_IDENTITY_CACHE_BUG`).

Cadeia: `VideoCache` → mediabunny `CanvasSink(poolSize: 3)` → `WrappedCanvas.canvas` → `resolveVideoNode` → `FrameDescriptor` (textura externa) → `WasmCompositor.syncExternalTexture`. O cache de textura externa dava HIT quando `previous.source === texture.source` + largura + altura. O `CanvasSink` **reaproveita o mesmo objeto Canvas** entre quadros e só reescreve os pixels; identidade do objeto ≠ identidade do conteúdo temporal.

Evidência (instrumentação `diagnostics/render-trace.ts` + `e2e/cache-diagnose.mjs`, seeks 0.5, 2.5, 3.5, 5.5, 0.5, 3.5, 2.5; `evidence/gpu-2b1/cache-diagnose-PRE-FIX.json`), reproduzida também no renderer de software do Cloud:

| seek | timestamp do quadro | Canvas | cache | snapshot |
|---|---|---|---|---|
| 0.5 | 0.5 | #1 | hit | `b05e…` |
| 2.5 | 2.5 | #1 | **hit, sem upload** | `b05e…` (igual) |
| 3.5 | 3.5 | #1 | **hit, sem upload** | `b05e…` (igual) |
| 5.5 | 5.5 | #1 | **hit, sem upload** | `b05e…` (igual) |

Condição exata encontrada 4 vezes: **mesmo Canvas + timestamp diferente + cache HIT** (veredito do script: `MUTABLE_CANVAS_IDENTITY_CACHE_BUG_CONFIRMED`; 3 snapshots distintos para 4 tempos distintos). O export não era o culpado: ele pede os quadros em sequência e o Canvas muda entre eles, então avançava no tempo enquanto preview/snapshot ficavam congelados.

Nada de timebase, PTS do exporter, fps, limiar de PSNR, efeitos do teste ou cache desligado foi alterado.

## Correção
Identidade de **conteúdo** além da identidade do objeto, propagada por todo o caminho:

`videoFrameRevision({timestamp,duration})` (`services/video-cache/revision.ts`) → `ResolvedVisualSourceNodeState.sourceRevision` / `BackdropSource.revision` → `ExternalTextureDescriptor.revision` (obrigatório) → `decideExternalTexture` (`compositor/texture-cache-policy.ts`) → cache do `WasmCompositor`.

HIT só se **mesmo objeto + mesma revisão + mesma largura + mesma altura**. Fontes estáticas (imagem, sticker, gráfico) usam a revisão constante `static` e continuam com cache normal. O hash do `BlurBackgroundNode` agora inclui a revisão do quadro (antes: só a identidade do Canvas, com um comentário afirmando erradamente que ela mudava com o quadro) — cobre vídeo como camada e vídeo como blur background.

## Pós-correção (Cloud, SwiftShader — evidência de lógica, não de GPU)
`evidence/gpu-2b1/cache-diagnose-POST-FIX.json`: mesmos 7 seeks → cada quadro novo faz upload (`miss/revision`), 4 tempos distintos → **4 snapshots distintos**, e os tempos revisitados reproduzem os **mesmos** hashes (0.5 → `ba0f…` ×2, 3.5 → `fe4c…` ×2, 2.5 → `a6d1…` ×2). `NOT_REPRODUCED`.

### Harness completo no Cloud após a correção (SwiftShader, `GPU_PARITY_PENDING_HUMAN_MACHINE` — software nunca dá PASS)
`evidence/gpu-2b1/gpu-parity-verdict-POST-FIX-cloud-swiftshader.json`:

| amostra | preview↔snapshot (dB) | **export↔snapshot (dB)** | antes (AMD real) |
|---|---|---|---|
| 0,5 s | 27,4 | **46,2** | 19,9 |
| 2,5 s | 29,5 | **43,0** | 19,7 |
| 3,5 s | 26,5 | **44,7** | 14,3 |
| 5,5 s | 25,5 | **44,8** | 18,7 |

`VIDEO_TEMPORAL_ADVANCE` = PASS: 7 seeks fora de ordem; tempos iguais revisitados dão PSNR 99 (pixels idênticos); o menor PSNR entre tempos distintos é 13,0 dB (limite 35); cada quadro novo fez upload. Sanidade do próprio teste: com a correção **desligada por mutação** (política ignora a revisão), o harness reporta `GPU_PARITY_FAIL` (`temporal-advance-MUTATION-revision-ignored-FAILS.json`): "vídeo congelado" (PSNR 99 entre tempos distintos) e "mesmo tempo 0,5 s com PSNR 18,9 < 45: resultado depende da ordem dos seeks". Ressalva: o PSNR preview↔snapshot (25–29 dB) é o mesmo patamar de sempre em software (captura de tela × render offscreen, passa o limiar 24); a medição em GPU física é a do rerun.

## Testes novos
`services/renderer/__tests__/texture-revision.test.ts` (12 testes; mutação confirmada: sem a revisão na política falham B/E/F; sem a revisão no hash do blur falha G):
A mesmo Canvas+mesma revisão ⇒ HIT · B mesmo Canvas+revisão diferente ⇒ upload · C Canvas diferente ⇒ upload · D imagem estática repetida ⇒ HIT · E seek aleatório: mesmo Canvas, timestamps diferentes ⇒ upload a cada quadro novo · F seek para trás reenvia o quadro correto (e repetição do mesmo quadro não reenvia) · G blur com vídeo: hash muda entre quadros com o Canvas reaproveitado, e é estável para imagem. Além disso, o descritor de frame propaga a revisão (vídeo) e usa `static` para imagem.

## Harness (`apps/editor/e2e/gpu-parity.mjs`)
- **`videoTemporalAdvance`** (independe do export; vídeo sozinho na timeline; seeks `0.5,2.5,3.5,5.5,0.5,3.5,2.5`): (a) tempos diferentes não podem gerar imagens quase iguais (PSNR < 35 ⇒ senão "vídeo congelado"); (b) o mesmo tempo revisitado precisa reproduzir os mesmos pixels (PSNR ≥ 45 ⇒ resultado independe da ordem); (c) quadro novo do decoder nunca pode ter cache HIT sem upload. Falha aqui é `GPU_PARITY_FAIL` **mesmo em renderer de software**.
- Cada amostra agora grava `source` = timestamp/revisão/Canvas do quadro de origem + decisões do cache de textura; o JSON traz `videoTemporalAdvance: { ok, reasons, samples[], pairs[] }`.
- Limiares ajustáveis por env: `TEMPORAL_SEEKS`, `TEMPORAL_DISTINCT_MAX`, `TEMPORAL_SAME_MIN`, e os de sempre (`PSNR_EXPORT_MIN`, `PSNR_PREVIEW_MIN`).

## Reexecução na máquina Windows física (mesma máquina AMD)
```powershell
git fetch origin integration/opencut-classic; git checkout integration/opencut-classic; git pull; .\tools\gpu-parity\run-windows.ps1 -Channel chrome
```
Saída: `apps\editor\gpu-parity-out\gpu-parity-verdict.json` (+ PNGs `temporal-*`, `snapshot-*`, `preview-*`, `export-*`). **O PASS final só vale depois desse rerun em GPU física; o Cloud não declara `GPU_PARITY_PASS`.**

## Interpretação
Se, no rerun, `videoTemporalAdvance.ok = true` **e** o export×snapshot passar: `ROOT_CAUSE = PREVIEW_SNAPSHOT_MUTABLE_TEXTURE_CACHE`, `GPU_RENDERER = PASS`, `EXPORT_PIPELINE = PASS`. Se o vídeo parar de congelar mas export×snapshot continuar falhando, abre-se uma segunda investigação (EXPORT_TIMING, SOURCE_TIME_MAPPING, FRAME_ROUNDING, COLOR_PIPELINE) — só depois.

---

# Fase 2B.1B — bootstrap do harness no Windows (HARNESS_BOOTSTRAP_FAIL)

## O que aconteceu
O rerun no Windows AMD físico falhou duas vezes, antes de qualquer teste de GPU, em `openNewProject()` → `page.waitForURL(/editor\//)` ("waiting for navigation until load", 30 s). Nenhum teste de GPU rodou (`frames=[]`, sem `videoTemporalAdvance`, sem detecção de GPU). **Isso não é `GPU_PARITY_FAIL`**: é `HARNESS_BOOTSTRAP_FAIL`. O renderer, o cache de textura, o export e os limiares ficaram **intactos** (o harness completo no Cloud reproduz exatamente os mesmos PSNR de antes: 46,24 / 43,02 / 44,70 / 44,79 dB).

## Causa raiz — o que se sabe e o que não se sabe
Não consegui reproduzir a falha no Cloud (o bootstrap sempre passou), então a causa exata no PC Windows **não está provada**. O que está provado:
1. `waitForURL` espera o evento `load`; a mensagem de timeout é idêntica quando (a) a URL já mudou mas `load` não dispara, e (b) a URL nunca muda. O código antigo não distinguia nenhum dos casos nem guardava evidência.
2. Candidatas, todas agora discriminadas pelo diagnóstico: **A** URL mudou mas `load` não completa; **B/C** `createNewProject`/`saveProject` falhou (storage); **D** `router.push` não navegou; **E** diálogo/modal; **F** erro de página; e, na infraestrutura, **servidor órfão/stale** — no Windows `Stop-Process` no wrapper `bunx` deixa o `next-server` filho vivo na porta, e a rodada seguinte conversa com um servidor antigo (hipótese consistente com "passou na 1ª rodada e falhou nas seguintes"; **testei no Cloud reconstruindo o `.next` sob um servidor em execução e a página continuou funcionando, então não a confirmo**).
3. Fragilidade real encontrada ao instrumentar: o predicado `project.getActive()` do próprio harness **lança** enquanto o projeto não carregou, o que faz o `waitForFunction` rejeitar na hora em vez de continuar tentando.

## Correção
- `e2e/bootstrap.mjs` (novo): espera por **`location.pathname`** (não por `load`); depois espera explícita por `__capiaAgent`, `__capiaEditor` e `__capiaE2E` com mensagem dizendo qual global falta; confere que o projeto ativo tem o id da rota; predicados tolerantes ao `getActive()` que lança.
- **Fallback controlado**: se a UI não navegar, registra o diagnóstico e (salvo `BOOTSTRAP_STRICT_UI=1`) cria o projeto pelo **mesmo `ProjectManager`/storage reais** (`__capiaEditor.project.createNewProject`) e abre `/editor/[id]` com retentativas (a corrida real do Next). O resultado traz `mode: "ui" | "programmatic-fallback"` e a falha da UI fica gravada — nada é escondido; não há projectId inventado, mock nem storage desligado; se o storage estiver quebrado o fallback também falha.
- **Preflight** (`serverPreflight`, `e2e/preflight.mjs`): `/projects` responde 200 **e** todos os assets `/_next/*.js|css` do HTML são servidos; senão `SERVER_BOOT_FAIL` (pega servidor stale/build trocado).
- **Classificação** no `gpu-parity.mjs`: `SERVER_BOOT_FAIL`, `HARNESS_BOOTSTRAP_FAIL`, `GPU_PARITY_FAIL`, `GPU_PARITY_PASS`, `GPU_PARITY_PENDING_HUMAN_MACHINE`. Qualquer falha **antes** de `VIDEO_TEMPORAL_ADVANCE` completar é `HARNESS_BOOTSTRAP_FAIL` (com `stage`/`failedStage`), nunca `GPU_PARITY_FAIL`.
- **`run-windows.ps1`**: limpa evidências antigas; detecta porta ocupada e encerra a árvore do dono; sobe o servidor com `server.log`/`server.err.log` preservados; espera `/projects` = 200 (90 s) ou falha como `SERVER_BOOT_FAIL` mostrando o log; roda o preflight; roda `PROJECT_BOOTSTRAP` isolado; só então o teste de GPU; no `finally` mata a **árvore** do servidor (`taskkill /T /F`).

## Diagnóstico adicionado
- App (só build E2E, só com `window.__CAPIA_TRACE__`): eventos `newProjectButton:click`, `createNewProject:start/done`, `saveProject:start/done/error`, `router.push` com `pathname`; hooks mínimos em `/projects` (`__capiaEditor`, `__capiaTrace`).
- Harness: URL antes/depois do clique e em 0/1/5/10 s; botões encontrados (texto, visível, habilitado), diálogos abertos, erros de console/página, `unhandledrejection`/pageerror, requisições falhas/4xx-5xx/pendentes, sonda de IndexedDB/OPFS, eventos de boot do app. Em qualquer falha grava `bootstrap-failure.png` + `bootstrap-diagnostics.json` (+ `project-bootstrap.json`).

## PROJECT_BOOTSTRAP (`e2e/project-bootstrap.mjs`, roda antes da GPU)
Critérios: 1 abre `/projects` · 2 cria projeto real · 3 obtém `projectId` · 4 chega em `/editor/[projectId]` · 5 `__capiaAgent` · 6 `__capiaEditor` · 7 projeto ativo = `projectId`. Também roda como primeiro check do `gpu-parity.mjs`.

## Testes negativos (Cloud) — evidências em `docs/adoption/evidence/bootstrap-2b1b/`
Injeção de falha só no harness (`E2E_FAULT`): `storage` (IndexedDB/OPFS lançam), `nav` (em `/projects`, `pushState/replaceState` viram no-op), `no-button`.
| Cenário | Resultado | O diagnóstico mostra |
|---|---|---|
| `storage`, UI estrita | `HARNESS_BOOTSTRAP_FAIL` | `saveProject:error` + `newProjectButton:error`, sonda `indexedDB: throw`, pageerror |
| `nav`, UI estrita | `HARNESS_BOOTSTRAP_FAIL` | `createNewProject:done` + `router.push` registrados, mas a URL fica em `/projects` nas 4 amostras |
| `nav`, com fallback | `PROJECT_BOOTSTRAP_PASS` com `mode=programmatic-fallback` | `uiFailure` gravada |
| `no-button`, UI estrita | `HARNESS_BOOTSTRAP_FAIL` | `locator.click` timeout |
| `gpu-parity` + `storage` | **`HARNESS_BOOTSTRAP_FAIL`** (stage `bootstrap`, `frames=0`) | `bootstrapFailure` + PNG/JSON |
| `gpu-parity` contra porta sem servidor | **`SERVER_BOOT_FAIL`** (stage `preflight`) | `ECONNREFUSED` |
Os dois últimos não viram `GPU_PARITY_FAIL`.

## Como saber que o bootstrap passou (primeiras linhas do rerun)
```
[SERVER] http://127.0.0.1:3100/projects => 200
[PREFLIGHT] PASS :: http://127.0.0.1:3100/projects 200, <N> assets ok
[PREFLIGHT] PASS :: /projects 200, <N> assets ok
[PROJECT_BOOTSTRAP] PASS :: id=<uuid> mode=ui url=/editor/<uuid> {"1 abriu /projects":true, ... "7 projeto ativo = projectId":true}
VERDICT: PROJECT_BOOTSTRAP_PASS
[PASS] PREFLIGHT (servidor responde e serve todos os assets do build) :: ...
[PASS] PROJECT_BOOTSTRAP (projeto real em /editor/[id], ...) :: id=<uuid> mode=ui ...
GPU: {"webgl2":"ANGLE (... AMD Radeon ...)", ...} software= false
```
Se aparecer `mode=programmatic-fallback`, o clique visual falhou e o diagnóstico de por quê está em `bootstrap-diagnostics.json` (`uiFailure`, `uiBootEvents`, `collector`).
