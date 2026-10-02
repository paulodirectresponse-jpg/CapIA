# S1 — pacote de medição do Preview Surface (Windows 11)

Executa o protocolo de `docs/spikes/S1-preview-surface.md` §4 e gera **um relatório estruturado** para análise. Fecha (ou não) **OD-1**, hard gate da Fase 3.

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

## Status de validação deste pacote (honestidade)

Criado numa sessão de nuvem **sem Windows**. Foi verificado apenas:

- compila e **linka** para `x86_64-pc-windows-gnu` (cross, MinGW) sem erros e passa no `clippy` sem avisos — **não** foi compilado com MSVC;
- `run.ps1` passa no **parser real do PowerShell** (7.6) e a lógica de correlação de GPU passa no `selftest.ps1` (`.\selftest.ps1`);
- **nenhuma execução** do harness em Windows. Se uma fase falhar, o relatório registra `status: "error"` com a mensagem; o processo sempre grava o JSON (parcial) em vez de sumir. Um relatório com erros é **resultado válido** para análise.

Riscos conhecidos do próprio harness (podem aparecer como erros/anomalias): `SendInput` exige sessão interativa desbloqueada; alguns drivers/políticas devolvem preto em `BitBlt` (`invalid_samples` alto); `SetForegroundWindow` pode ser negado; contadores "GPU Engine" podem não existir em GPUs/drivers antigos.

## Arquivos

```
run.ps1            orquestra: pré-checagens, build, hardware, contadores de GPU, execução, perguntas manuais, zip
selftest.ps1       autoteste da lógica não específica de Windows do run.ps1
src/               harness Rust (Tauri 2 + wgpu 29 + webview2-com): main.rs, engines.rs, gpu.rs, win.rs, pattern.wgsl
ui/index.html      página: preview transparente, overlay HTML, botões, canvas WebGL do P2
```

É código **descartável** (workspace Cargo próprio, fora do build e do CI do produto).
