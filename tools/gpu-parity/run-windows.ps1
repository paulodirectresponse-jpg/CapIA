# Paridade de GPU REAL no editor OpenCut (Windows com GPU física). Windows PowerShell 5.1 ou pwsh.
# Pré-requisitos: Git, Node 22+, Bun 1.2.18+, FFmpeg (ffmpeg/ffprobe no PATH), Google Chrome ou Edge.
#   .\tools\gpu-parity\run-windows.ps1 [-Channel chrome|msedge] [-Port 3100]
# Saída: apps\editor\gpu-parity-out\gpu-parity-verdict.json (+ PNGs). Veredito esperado: GPU_PARITY_PASS | GPU_PARITY_FAIL.
# NÃO rode em VM/RDP sem GPU: o harness detecta renderer de software e reporta PENDING_HUMAN_MACHINE.
param([string]$Channel = "chrome", [int]$Port = 3100)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$editor = Join-Path $root "apps\editor"
foreach ($t in "node","bun","ffmpeg") { if (-not (Get-Command $t -ErrorAction SilentlyContinue)) { throw "$t não encontrado no PATH" } }
Push-Location $editor
try {
  bun install --frozen-lockfile
  Push-Location "apps\web"
  $env:NEXT_PUBLIC_CAPIA_E2E_HOOKS = "1"; $env:NEXT_PUBLIC_CAPIA_AGENT_BRIDGE = "1"
  bun run build
  Pop-Location
  $srv = Start-Process -FilePath "bunx" -ArgumentList @("next","start","-p","$Port") -WorkingDirectory (Join-Path $editor "apps\web") -PassThru -WindowStyle Hidden
  try {
    Start-Sleep -Seconds 10
    $env:GPU_REAL = "1"; $env:BROWSER_CHANNEL = $Channel; $env:EDITOR_URL = "http://127.0.0.1:$Port"
    node e2e\gpu-parity.mjs
    Write-Host "`n== veredito =="; Get-Content (Join-Path $editor "gpu-parity-out\gpu-parity-verdict.json") -Raw | Write-Host
  } finally { if ($srv -and -not $srv.HasExited) { Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue } }
} finally { Pop-Location }
Write-Host "Envie a pasta apps\editor\gpu-parity-out (zip) de volta."
