# S5 — Timeline em canvas com 10.000 clips

**Pergunta:** canvas + virtualização aguenta 10.000 clips (scroll/zoom/drag) no orçamento de frame? Confirma C3 / ADR-005.

**Resultado: CONFIRMADO** (em Chromium headless, **sem GPU**, raster por CPU — limite inferior de desempenho).

## Método

`spikes/s5-timeline-canvas/timeline.html` + `run.mjs` (Playwright/Chromium 141, 1920×500, 4 vCPU, `--disable-gpu`). Dataset sintético: 50 tracks × 200 clips = 10.000 clips (~19 min de timeline). Dois renderers sobre os mesmos dados: **(a) canvas 2D virtualizado** (busca binária por range de tempo por track; só desenha tracks/clips visíveis; LOD: sem label/borda/waveform abaixo de larguras mínimas; waveform simulada com 200 traços em 30% dos clips) e **(b) DOM sem virtualização** (10.000 `div`s, como fazem os candidatos open-source). Cenários: scroll contínuo, zoom contínuo (2→400 px/s), drag de um clip, e *fit* com **todos os 10.000 clips visíveis**.

## Resultados (frame = intervalo entre `requestAnimationFrame`; teto = 16,7 ms por vsync de 60 Hz)

| Renderer | Cenário | Clips desenhados | Frame p50 / p95 | JS de desenho p50 / p95 |
|---|---|---|---|---|
| Canvas | scroll | 114 | 16,7 / 17,7 ms | 1,0 / 1,6 ms |
| Canvas | zoom | 1.685 | 16,7 / 17,1 ms | 0,5 / 1,2 ms |
| Canvas | drag | 233 | 16,7 / 17,0 ms | 0,8 / 1,2 ms |
| Canvas | **fit (10.000 visíveis)** | **10.000** | 16,7 / 17,0 ms | **2,1 / 3,8 ms** |
| DOM | **zoom** | 10.000 nós | **124,7 / 188 ms (~8 fps)** | 65,9 / 106 ms |
| DOM | drag | 10.000 nós | 23,6 / 28,6 ms | 12,7 / 16,2 ms |
| DOM | scroll (via `transform`) | 10.000 nós | 16,8 / 33,5 ms (máx 81 ms) | 0,4 / 0,5 ms |
| DOM | fit | 10.000 nós | 16,6 / 20,8 ms | 8,9 / 12,9 ms |

Canvas: **60 fps sustentado** em todos os cenários com folga (JS 0,5–2,1 ms p50 de 16,7 ms), inclusive com os 10.000 clips visíveis, **só com raster por CPU**. DOM: zoom — operação central de uma timeline — cai a ~8 fps.

## Limites

- Sem thumbnails (imagens/tiles) nem waveform real (pirâmide) no desenho: adicionam custo de upload/blit; há folga, mas **não medido**.
- Sem texto rico, seleção, handles, snapping visual; sem a camada de interação.
- `requestAnimationFrame` limita a 60 Hz: p50 = 16,7 ms só indica "não perdeu frames"; a folga real é o tempo de JS.
- Chromium Linux ≠ WebView2 (mesma família de motor; diferenças de raster/compositing não medidas).
- DOM virtualizado (não testado) poderia melhorar o DOM; o ponto é que **o DOM só vira viável com virtualização equivalente à do canvas**, perdendo sua vantagem.

## Decisão

C3 e **ADR-005 confirmados**: `ui-timeline` em canvas virtualizado com LOD. Orçamentos para a Fase 3: JS de desenho ≤ 4 ms p95 com 10.000 clips visíveis; 60 fps em scroll/zoom/drag.
