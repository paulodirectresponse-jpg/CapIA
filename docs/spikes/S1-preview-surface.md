# S1 — Preview Surface (Tauri/WebView2)

**Pergunta:** dá para exibir preview de baixa latência dentro de Tauri/WebView2 (P1 superfície nativa filha, P2 SharedBuffer → canvas)? Fecha OD-1.

**Resultado (§6): MEDIDO no runner Windows (WebView2/DWM reais, GPU por software). P1 eliminado (airspace); P2 passa os critérios funcionais e de latência → presenter = P2 (ADR-069). OD-1 FECHADA**, com validação residual de CPU/pacing em GPU real como critério de aceitação da Fase 3 (gatilho de reabertura na ADR-069). As §1–§5 abaixo são o registro histórico anterior à medição.

## 0. Status do harness (revisão pós-primeiro teste em Windows 11)

O primeiro teste real em Windows 11 **não completou a medição** por defeitos do **harness**, não por resultado do experimento: `.ps1` com não-ASCII quebrava no PowerShell 5.1; o HWND filho do P1 era criado numa thread sem message loop (UI "não respondeu ao layout"); o P2 não recebia o SharedBuffer sem diagnóstico; havia `Mutex` travado duas vezes no resize, readback sem timeout e nenhum log estruturado. Tudo isso foi corrigido (ver `tools/s1-preview-spike/README.md`), com eventos JSONL, watchdog com relatório parcial e um resumo por modo (`MEASURED` / `PARTIAL` / `FAILED`).

O fluxo (`run.ps1` → build → execução → ZIP) é reproduzido no CI (`.github/workflows/s1-harness.yml`) num runner Windows com WebView2/DWM reais e **adaptador WARP (sem GPU física)**. Essa execução valida o harness, **não** o presenter: observações do runner (p. ex. P1 com o filho abaixo do WebView2 transparente não mostrar o padrão, enquanto o controle com o filho acima mostra) são **indícios**, não decisão. **Critérios de decisão inalterados; OD-1 permanece ABERTA** até uma execução em PC com GPU real (DPI 100%/150%, multi-monitor, observações manuais).

## 1. Evidências obtidas (e seu grau de confiança)

| Fato | Fonte | Confiança |
|---|---|---|
| `wry 0.57.0` (usado por `tauri 2.12.1`) cria o WebView2 **somente em windowed hosting**: `CreateCoreWebView2ControllerWithOptions(hwnd, …)`. **Zero** menções a `CompositionController`/DirectComposition | leitura do código-fonte baixado do crates.io | **Alta** (verificado) |
| Transparência no wry = `DefaultBackgroundColor` com alpha 0; nenhuma janela em camadas (`WS_EX_LAYERED`) ou tratamento de irmãos | mesmo código (`webview2/mod.rs`) | **Alta** |
| `tauri::Window::add_child` existe (webviews filhas); não há API para filho *nativo não-web* | `tauri-2.12.1/src/window/mod.rs` | **Alta** |
| No modo janela, o WebView2 é um HWND filho **em outro processo** (`msedgewebview2.exe`); **não cria D3D device no processo do app** nem usa o do app | resposta de mantenedor do WebView2 em [MicrosoftEdge/WebView2Feedback#4019](https://github.com/MicrosoftEdge/WebView2Feedback/discussions/4019) | Média-alta (fonte primária, 1 resposta) |
| `ICoreWebView2Environment12::CreateSharedBuffer` + `PostSharedBufferToScript` entregam memória compartilhada como `ArrayBuffer` ao JS; limite < 2 GB | resultados de busca da documentação Microsoft (`learn.microsoft.com` está **bloqueada** pelo proxy desta sessão; não li a página) | Média (secundária) |
| Em windowed hosting, cor de fundo transparente mostra "conteúdo do app host" (Win > 7). **Não** está estabelecido se isso inclui um **HWND irmão** posicionado abaixo | idem; e inferência | **Baixa — é exatamente a pergunta do S1** |
| Existe pacote comunitário de "airspace fix" para WebView2, o que indica que o problema é real na prática | busca (nuget `webview2backhost`) | Baixa-média |

## 2. Medição de proxy executada (NÃO é medição do WebView2)

Chromium 141 headless (mesma família do WebView2), **GL por software (SwiftShader)**, 4 vCPU. Custo do lado do navegador para exibir um frame RGBA que vive num `ArrayBuffer` (formato do SharedBuffer):

| Tamanho | MB/frame | cópia (stand-in de memcpy) p50 | WebGL `texSubImage2D`+draw+sync p50 / p95 | Canvas2D `putImageData` p50 |
|---|---|---|---|---|
| 960×540 | 1,98 | 0,2 ms | 5,2 / 7,7 ms | 0,2 ms |
| 1280×720 | 3,52 | 0,3 ms | 5,4 / 6,8 ms | 0,4 ms |
| 1920×1080 | 7,91 | 0,7 ms | 6,6 / 9,5 ms | 0,8 ms |

Leitura honesta: o **custo de CPU do lado do navegador** de P2 não é, por si só, um gargalo (≪ 16,7 ms mesmo em software). **Não medido** e provavelmente dominante: latência de composição entre processos, *frame pacing* do WebView2, sincronização com vsync, e **readback GPU→CPU** do frame composto pelo wgpu (custo e stalls dependem de GPU/driver). Esses são os itens que decidem OD-1.

Código: `spikes/s1-preview-surface/proxy_upload.{html,mjs}` (descartável).

## 3. O que isso muda na análise (sem escolher o presenter)

- **P1 (HWND nativo filho)**: depende de um comportamento **não verificado** do WebView2 em modo janela (transparência compondo com HWND irmão). Se não compuser, P1 só funciona *acima* da WebView (cobrindo a UI = airspace clássico) ou exige a UI de controles do preview em janelas nativas.
- **P2 (SharedBuffer → canvas)**: caminho mais seguro de integração (sem airspace, UI HTML pode sobrepor), custo de CPU do lado web comprovadamente baixo em proxy; incógnitas são readback e pacing.
- **P4 (novo): visual hosting (DirectComposition)** — arquitetura limpa para compor swapchain nativa e UI web numa árvore única. **Não disponível no Tauri atual** (wry só faz windowed hosting, verificado). Exigiria *fork/contribuição* ao wry ou shell próprio — custo alto; só considerar se P1 e P2 falharem.
- A ADR-001 **não** é invalidada por esta página; a *stop condition* ("S1 demonstra inviabilidade") **não foi atingida**, porque nada foi demonstrado em nenhum sentido.

## 4. Protocolo para executar S1 (Windows 11, 1 máquina com iGPU basta; repetir com dGPU se disponível)

Construir dois harnesses mínimos (~300 linhas cada, descartáveis) em Tauri 2, ambos exibindo um quad animado a 1080p@60 (padrão com contador de frame embutido, como o S2) produzido por wgpu/D3D12:

- **H-P1:** HWND filho (windows-rs) com swapchain wgpu posicionado **abaixo** do WebView2 transparente; UI HTML (botões, menu dropdown, handles) sobreposta.
- **H-P2:** wgpu → readback assíncrono → `SharedBuffer` → `texSubImage2D` num canvas na WebView (resolução de preview 960×540 e 1280×720).

Medir, para cada harness: (1) **latência de apresentação** (timestamp de composição do frame vs. chegada à tela; usar contador de frame + câmera/high-speed ou `DwmGetCompositionTimingInfo`); (2) **fps sustentado** 60 s e frames perdidos; (3) CPU% e GPU% (Gerenciador/ETW); (4) cópias CPU/GPU por frame; (5) **resize** contínuo (flicker, atraso de sincronização de bounds); (6) **DPI** 100/150/200% e troca entre monitores; (7) **fullscreen** do preview; (8) **input** (clique/arrasto sobre o preview, foco, teclado); (9) **overlay de UI** (dropdown/tooltip HTML sobre o preview funciona? recorta?); (10) compatibilidade com o compositor wgpu (mesmo device/queue? formato?).

**Critérios (herdados de `PREVIEW_RENDER.md` §5):** 1080p@60 estável; seek < 100 ms; CPU < 25% em notebook médio; resize/DPI/multimonitor sem artefatos; overlays HTML funcionais sobre o preview.

**Regra de decisão (definida agora, antes dos números):**
1. Se H-P1 passar **todos** os critérios incluindo overlay HTML sobre o preview → **P1**.
2. Senão, se H-P2 passar latência/fps/CPU em 720p preview → **P2** (P1 só como janela destacável).
3. Senão → documentar, **parar** trabalhos dependentes de preview embutido e abrir ADR para P4 (fork do wry/visual hosting) ou revisão da ADR-001.

## 5. Impacto no plano

- Fase 2 (motor headless, render, export, CLI) **não depende** de S1: o `PreviewPresenter` é um trait e o compositor produz uma textura/frames independentemente do destino.
- Dependem de OD-1: o presenter nativo de `capia-preview` e toda a Fase 3 (UI do editor).
- **Decisão do PO (ADR-037):** a Fase 2 pode iniciar com OD-1 aberto; **OD-1 é hard gate da Fase 3**. Para executar: `tools/s1-preview-spike/` (`.\run.ps1` em Windows 11; gera um `.zip` com o relatório estruturado para análise). O pacote só foi compilado/cross-linkado e testado no que não é específico de Windows — ver o README dele.

## 6. Resultados medidos (harness corrigido, runner `windows-latest`)

**Ambiente:** Windows Server 2025 (desktop interativo), WebView2 153, DWM a 64 Hz, adaptador **WARP** (D3D12 por software, sem GPU física), 4 vCPU, tela pequena (janela ajustada à área útil). Workflow `.github/workflows/s1-harness.yml` (runs 6–9 no commit `ac5f3c2`); relatório por modo `MEASURED` nos três modos. **Critérios e regra de decisão (§4) inalterados.**

| Medida | P1 (filho **abaixo**) | P1_above (controle) | P2 (SharedBuffer → canvas) |
|---|---|---|---|
| Padrão visível na tela | **NÃO** (grade 3×3 = fundo da página, 0 amostras de latência) | sim (26–39 ms após posicionar) | sim (18–37 ms) |
| Overlay HTML sobre o preview | n/a (padrão invisível) | não (nativo cobre o HTML — esperado) | **sim** (192,64,64 = mistura exata) |
| Input sobre o preview | — | nativo captura | HTML recebe; 0 cliques no HWND nativo |
| Latência submissão→tela (p50 / p95) | — | — | **36 ms / 48 ms** (540p e 720p) |
| fps / slots perdidos | ~62 fps (DWM 64 Hz), 0 erros de surface | ~62 fps | 540p: 60,0 fps, 0 perdidos · 720p: 57,4 fps, 59/1807 (3,3 %) |
| Readback GPU→CPU (p50) | — | — | 5,5 ms (540p) / 10,9 ms (720p) — WARP |
| Resize contínuo / fullscreen | — | — | sem amostra fora do retângulo esperado; padrão correto 47–92 ms após cada passo (1º passo frio 304 ms); fullscreen decodifica frame da tela |
| CPU (GetProcessTimes) | — | — | 24 % da máquina (540p), 47 % (720p) **incluindo o rasterizador WARP**; WebView2 ≈ 0,15 %; JS upload+draw p50 0,3 ms |

**Leitura (regra §4):**
1. **P1 falha** o critério "overlay/padrão sob WebView2 transparente": reproduzido em todas as execuções; o controle com o mesmo HWND **acima** mostra o padrão, logo a sonda/posição estão corretas. Árvore de janelas medida: `WRY_WEBVIEW → Chrome_WidgetWin_0/1 → Chrome_RenderWidgetHostHWND → Intermediate D3D Window` cobrindo toda a região — airspace por *windowed hosting*; é propriedade da hospedagem, não da GPU (inferência).
2. **P2 passa** overlay, input, resize, fullscreen e latência. **fps/CPU em GPU real não são mensuráveis** neste runner: o rasterizador de software roda dentro do processo medido, então 47 % de CPU a 720p é número confundido (nem a favor nem contra o critério < 25 %).
3. **Decisão:** P2 (ADR-069). **Residual obrigatório:** rodar `powershell -ExecutionPolicy Bypass -File .\run.ps1 -P2Res 1280x720` em PC com GPU real (e 150 % DPI / 2 monitores se possível) na primeira entrega de preview da Fase 3; gatilho de reabertura na ADR-069.
