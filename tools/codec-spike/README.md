# codec-spike — H.264 + AAC no WebView2 (kill criterion da Fase 2A)

O editor exporta MP4 por WebCodecs (`CanvasSource codec:"avc"` + `AudioBufferSource codec:"aac"` via
mediabunny) — sem x264/FFmpeg embarcado. Isto prova, num WebView2 **real**, se há caminho viável:

| Arquivo | Papel |
|---|---|
| `spike.html` | reproduz o caminho de export (probe `isConfigSupported` + encode 2 s 640×360@30, AAC 48 kHz) |
| `server.mjs` | serve a página e recebe o relatório (`POST /report`) |
| `run-webview2.ps1` | **WebView2 real** (WinForms + SDK NuGet), Windows PowerShell 5.1 |
| `run-chromium.mjs` | controle (Chromium do Playwright, sem H.264): o harness tem que reportar FALHA |
| `verify.mjs` | ffprobe → `H264_PASS` / `H264_FAIL` (container, codecs, duração, fps, taxa de áudio) |

CI: job `codec-webview2` em `.github/workflows/opencut-integration.yml` (windows-latest). Resultado e logs
em `docs/adoption/PHASE_2A.md`. Se WebCodecs não entregar H.264/AAC, o caminho híbrido (compositor do
editor → quadros → encoder nativo FFmpeg **LGPL** do CapIA legado; nunca x264 GPL) é o plano B.
