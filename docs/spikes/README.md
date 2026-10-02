# SPIKES — Fase 1 (M03)

Um spike responde **uma pergunta** para fechar uma decisão. Código descartável fica em `/spikes/<id>` e **não** é base de produto. Resultados abaixo; relatórios completos em `S1..S7`.

## Ambiente de execução (importante para interpretar as evidências)

Container Linux x86_64 (Ubuntu 24.04), 4 vCPU, 15 GB RAM, **sem GPU e sem Windows**. Rust 1.97, Node 22, FFmpeg 6.1 (build Ubuntu, GPL), Chromium (Playwright). Consequências:

- Tudo que **depende de Windows/WebView2/GPU real** (S1, decode por hardware do S2) **não pôde ser medido**. Está marcado como **NÃO MEDIDO**; não há números inventados nem extrapolados.
- Números de CPU/software-render são **limites inferiores de desempenho** (uma GPU real só melhora o render do canvas/compositor; não melhora lógica de CPU/WASM).
- Métricas de acompanhamento desta fase: marcos, critérios de aceitação, riscos eliminados, blockers, retrabalho (não pessoas-semanas).

## Matriz (definida antes da execução)

| Spike | Pergunta que responde | Resultado necessário para decidir |
|---|---|---|
| **S1** Preview surface | Dá para exibir frames de baixa latência dentro de Tauri/WebView2 sem airspace/DPI/resize inviáveis? P1 (superfície nativa filha) ou P2 (SharedBuffer→canvas)? Compatível com wgpu? | Medições em Windows: latência apresentação, CPU/GPU, cópias, resize/DPI/fullscreen/input, overlay de UI. **Fecha OD-1** |
| **S2** Decode frame-exato | A regra `frame_at` (sample-and-hold sobre frame index) dá seek frame-exato e cadência correta com VFR/CFR? Sync A/V? HW decode/rotação/HDR no Windows? | Teste automatizado com mídia sintética VFR/CFR (lógica); medição Windows para HW decode (pendente) |
| **S3** FFmpeg LGPL | Uma build LGPL/shared tem os encoders/decoders necessários sem GPL/nonfree? Reproduzível/pinada? | Inventário de encoders/decoders, flags, manifesto de licenças, alvo Windows. **Fecha parte técnica de OD-2** |
| **S4** Core em WASM | `capia-time/model/commands` compilam para WASM com paridade com o nativo e latência de ghost preview adequada? Custo de IPC de patches? | Paridade bit-a-bit nativo×WASM, tempo/chamada, tamanho do binário, custo de serialização. **Aceita/rejeita ADR-016** |
| **S5** Timeline em canvas | Canvas+virtualização aguenta 10.000 clips (scroll/zoom/drag) dentro do orçamento de frame? | Tempo de frame p95 sob zoom/scroll/drag vs DOM. **Confirma/refuta C3 (ADR-005)** |
| **S6** Compositor OpenCut | Os crates MIT `gpu/compositor/effects/masks` reduzem trabalho real? | Veredito `REUSE_AS_SEED` / `REIMPLEMENT_WITH_REFERENCE` / `REJECT` com acoplamento, deps, qualidade, funcionalidades |
| **S7** Suíte de aceitação de timeline | Conseguimos especificar o comportamento CapCut-like (placement, snapping, ripple, retime, group move, keyframes) como suíte **nossa**, independente do OpenCut? | Cenários executáveis (dados) validados por oráculo descartável; proveniência registrada |

## Resultados

| Spike | Resultado | Decisão |
|---|---|---|
| **S1** Preview surface | **NÃO MEDIDO** (sem Windows/GPU/WebView2). Verificado: `wry 0.57` só faz *windowed hosting*; proxy de upload no navegador é barato (≤ 7 ms em software a 1080p). Protocolo e regra de decisão definidos | **OD-1 ABERTA**. Stop condition **não** atingida |
| **S2** Frame-exato/VFR | **Lógica CONFIRMADA** (sintético): índice→Ticks exato; seek por índice 100% exato; `-ss` ingênuo erra (até 75 frames); filtro `fps` diverge 13–38%; AAC +1 amostra. HW decode/HDR/mídia real **não medidos** | ADR-035 |
| **S3** FFmpeg LGPL | **Viável.** BtbN `lgpl-shared` é **LGPL-3.0** (não 2.1); build mínima própria **LGPL-2.1+**, sem libs externas, `libavcodec` 5,7 MB; fallback SW: OpenH264 Baseline / kvazaar. Reprodutibilidade, build Windows e HW encoders **não testados** | ADR-032 (**fecha OD-2**) |
| **S4** Core em WASM | **CONFIRMADO:** hash idêntico nativo×WASM (200k ops, 10k clips); ghost-move 1,3 µs p50; 36 KB. IPC Tauri **não medido** | ADR-016 **Accepted** (condicionada) |
| **S5** Timeline canvas | **CONFIRMADO:** 60 fps com 10.000 clips visíveis (JS 2,1 ms p50) sem GPU; DOM ~8 fps no zoom | ADR-005 confirmada |
| **S6** Compositor OpenCut | **`REIMPLEMENT_WITH_REFERENCE`:** 9/9 checagens de correção ok em wgpu nativo; porém 8 bits, ~16 MB/camada (3,3 GB com 200), sem YUV/texto/transições | ADR-034 |
| **S7** Aceitação de timeline | **108 cenários**, 108/108 consistentes; mutação detecta regressões; nada copiado | ADR-036 (+ D-S7-1..8 a confirmar) |

Código dos spikes (descartável): `/spikes/{s1-preview-surface,s2-frame-exact,s3-ffmpeg-minimal,s4-wasm-core,s5-timeline-canvas,s6-compositor-probe,s7-oracle}`. O único ativo **permanente** produzido é `tests/acceptance/timeline/`.
