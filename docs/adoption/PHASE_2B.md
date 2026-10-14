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
