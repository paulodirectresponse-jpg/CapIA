# @capia/ui-timeline

Timeline própria em **canvas 2D virtualizado** (ADR-005): nenhum elemento DOM por clip; só o que está na janela de tempo/faixas é pintado (busca binária por faixa). Interação (seleção, marquee, mover, reordenar, trim, blade, scrub, drop de mídia) com **ghost local**: snap, movimento de grupo e colocação vêm do **WASM do core** (`capia-timeline-wasm`, ADR-070) — nenhuma matemática de snap em JS, nenhum IPC durante o arrasto (IPC só no drop).

- `wasm.ts` ponte tipada; o `.wasm` é gerado por `pnpm build:wasm` (gitignored; o CI gera antes dos testes).
- `layout.ts` coordenadas/zoom; `data.ts` índice por faixa; `view.ts` renderer + gestos; `palette.ts` cores dos tokens.
- Amostras de pintura/gesto (`stats.paintSamples/gestureSamples`) alimentam o benchmark de `TIMELINE_UX.md` §6.
