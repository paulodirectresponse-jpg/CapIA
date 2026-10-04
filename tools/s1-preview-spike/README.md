# S1 — pacote de medição do Preview Surface (Windows 11)

Executa o protocolo de `docs/spikes/S1-preview-surface.md` §4 e gera **um relatório estruturado** para análise. **OD-1 foi fechada (ADR-069, presenter P2)** com base em execuções no runner Windows; este pacote continua sendo o teste de aceitação de CPU/pacing em **GPU real** (`.\run.ps1 -P2Res 1280x720`).

## Como executar

Em um **Windows 11** com WebView2 Runtime, Rust (MSVC) e uma GPU (iGPU basta), no PowerShell:

```powershell
cd tools\s1-preview-spike
.\run.ps1
```

Se a política de execução bloquear: `powershell -ExecutionPolicy Bypass -File .\run.ps1`.

Duração: ~3 min (+ a primeira compilação, alguns minutos). **Não mexa no mouse/teclado** durante o teste (ele usa captura de tela e entrada sintética) e deixe o notebook na tomada. Ao final o script faz algumas perguntas rápidas sobre o que você viu (flicker etc.).

**Envie** o arquivo `reports\s1-report-<…>.zip` que o script imprime no final.

### Repetições recomendadas (cada uma gera outro relatório; use `-Label`)

| Cenário | Comando |
|---|---|
| Escala 100% | `.\run.ps1 -Label 100pct` |
| Escala 150% (Configurações → Sistema → Tela) | `.\run.ps1 -Label 150pct` |
| 2 monitores com escalas diferentes, janela no monitor principal | `.\run.ps1 -Label dual-mixed-dpi` |
| Notebook híbrido: outra GPU (Configurações → Tela → Elementos gráficos) | `.\run.ps1 -Label dgpu` |

Opções: `-SteadySecs 20` (fases estáveis mais longas) · `-Modes p1,p2` · `-P2Res 1280x720` · `-SkipInput` · `-SkipGpuCounters` · `-SkipManual` · `-SkipBuild`.

## O que é medido

| Item do S1 | Como | Onde no relatório |
|---|---|---|
| WebView2 | versão do runtime; criação de `SharedBuffer` (`ICoreWebView2Environment12`/`ICoreWebView2_17`) | `host.webview2_runtime`, `modes.p2…setup_error` |
| **P1** — HWND nativo sob WebView2 transparente | filho nativo + swapchain wgpu (D3D12, Fifo) atrás do WebView2; lê **pixels da tela pós-DWM** no ponto de uma zona cinza: se aparece o cinza, o WebView2 transparente **revela** o HWND irmão | `p1…probe_static.native_or_canvas_pattern_visible_at_probe` |
| Overlay de UI | caixa HTML vermelha 50% sobre a zona cinza; esperado = mistura (191,64,64) | `html_overlay_composited_over_pattern` |
| **P2** — SharedBuffer → canvas | wgpu offscreen → readback → memória compartilhada → `texSubImage2D` num canvas WebGL | `p2…engine_stats_whole_mode`, `page_stats` |
| Controle (airspace conhecido) | `p1_above`: nativo **acima** do WebView2 — deve esconder o overlay HTML | `modes.p1_above` |
| Pacing | intervalo entre apresentações (P1) / produção (P2), frames fora do slot, `rAF` e frames pulados na página | `present_interval_ms`, `frames_that_missed_their_slot`, `raf_interval_ms` |
| Latência | faixa de 16 bits com o número do frame desenhada no padrão; amostrador lê a tela e mede **submissão → aparece**; frames pulados | `steady_latency.screen_latency` |
| Resize | 5 tamanhos fixos + 60 passos contínuos; atraso do layout; tempo até o padrão ficar certo; fração de amostras desalinhadas | `resize` |
| DPI | `GetDpiForWindow`, fator de escala, monitores; o padrão é lido corretamente em qualquer escala | `runtime.window`, `host.screens` |
| Fullscreen | `set_fullscreen`; padrão visível e frame decodificado da tela | `fullscreen` |
| Input | cliques sintéticos: botão da toolbar, botão sobre o preview, área nua do preview + teclado; conta quem recebeu (HTML × HWND nativo) | `input` |
| CPU | `GetProcessTimes` do app + descendentes (`msedgewebview2`) em núcleos e % da máquina (fase **sem** o amostrador de pixels, que contaminaria o CPU) | `steady_cpu.resources` |
| GPU | contadores do Windows "GPU Engine" (1 Hz) por tipo de engine, só do app e de todos os processos, correlacionados por fase pelo `run.ps1` | `phases[].gpu_counters` |
| Cópias CPU/GPU | contagem **estrutural** + tempos medidos de readback e memcpy (P2) | `copies`, `gpu_to_cpu_readback_wait_ms` |

### Não medido automaticamente (registrado em `unavailable_metrics`)

Latência fóton-a-fóton · mudança de DPI/monitor durante a execução (repita com `-Label`) · flicker/tearing (só você vê; vai nas perguntas manuais) · contadores de hardware de cópias · pacing interno do compositor do WebView2.

## Limites da latência medida

Lê a **tela já composta pelo DWM** via `BitBlt`. Inclui o atraso de composição, mas **não** o do monitor; a resolução depende da taxa de captura (`capture_rate_hz` no relatório) e da programação das threads. É adequada para **comparar P1 × P2**, não para citar milissegundos absolutos de ponta a ponta.

## Como ler o resultado (por modo)

O `run.ps1` imprime ao final **"RESULTADO POR MODO"** e o mesmo vai em `summary` no JSON/MD:

| Status | Significado |
|---|---|
| `MEASURED` | todas as fases do modo rodaram e produziram amostras válidas |
| `PARTIAL` | o modo rodou, mas alguma fase/métrica ficou sem dados (ex.: latência com 0 amostras válidas, fase abortada) — o motivo vem em `failure_reasons` |
| `FAILED` | o modo não chegou a medir (ex.: SharedBuffer não recebido em 10 s, GPU não inicializou) — `setup_error` diz por quê |
| `NAO EXECUTADO` | modo fora de `-Modes` |

`pattern_visible=false` no P1 **não é falha do harness**: é o resultado da medição (o WebView2 transparente não deixa ver a janela-irmã abaixo — "airspace"). O harness só garante que a sonda foi lida com a janela posicionada e após espera/polling (`ms_until_pattern_visible`). Os critérios de decisão do S1 **não** foram alterados.

Diagnóstico: `reports\*.events.jsonl` (eventos estruturados: fases, handshakes, erros da página, criação do HWND filho, readback), `runtime.window_tree` (árvore de janelas Win32), `first_invalid_samples` (faixa de código de latência não decodificável). Um watchdog grava relatório parcial e encerra o processo se uma fase travar.

`-Quick` (3 s por fase, sem perguntas manuais nem contadores de GPU) serve para smoke test; não use para decidir.

## Status de validação deste pacote (honestidade)

- O fluxo `run.ps1` (Windows PowerShell 5.1) → build → execução → ZIP é exercitado no CI (`.github/workflows/s1-harness.yml`, runner `windows-latest`: WebView2, DWM e desktop reais, **sem GPU física** — adaptador WARP). Isso valida encoding/PS 5.1, build MSVC, handshake de layout, SharedBuffer, HWND filho, sondas e coleta de métricas.
- **Não** decide OD-1: fps/latência/CPU/GPU, escalas de DPI, múltiplos monitores e as observações manuais (flicker, resize) só valem no PC real do usuário.
- Bugs do harness corrigidos nesta revisão: `.ps1` com não-ASCII (PS 5.1 lia como ANSI), stderr do cargo virando erro terminante, HWND filho criado em thread sem message loop (travava a UI), `Mutex` travado duas vezes na mesma expressão (deadlock no resize), readback sem timeout, ausência de logs/watchdog, e sonda lida antes do primeiro quadro do P2.
- Riscos remanescentes: `SendInput` exige sessão interativa desbloqueada; alguns drivers/políticas devolvem preto em `BitBlt` (`invalid_samples` alto); contadores "GPU Engine" podem não existir em drivers antigos.

## Arquivos

```
run.ps1            orquestra: pré-checagens, build, hardware, contadores de GPU, execução, perguntas manuais, zip
selftest.ps1       autoteste da lógica não específica de Windows do run.ps1
src/               harness Rust (Tauri 2 + wgpu 29 + webview2-com): main.rs, engines.rs, gpu.rs, win.rs, pattern.wgsl
ui/index.html      página: preview transparente, overlay HTML, botões, canvas WebGL do P2
```

É código **descartável** (workspace Cargo próprio, fora do build e do CI do produto).
