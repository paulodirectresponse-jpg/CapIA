# Paridade de GPU REAL no editor OpenCut (Windows com GPU física). Windows PowerShell 5.1 ou pwsh.
# Pré-requisitos: Git, Node 22+, Bun 1.2.18+, FFmpeg (ffmpeg/ffprobe no PATH), Google Chrome ou Edge.
#   .\tools\gpu-parity\run-windows.ps1 [-Channel chrome|msedge] [-Port 3100]
# Saída: apps\editor\gpu-parity-out\  (gpu-parity-verdict.json, server.log, bootstrap-*.{json,png}, PNGs).
# Vereditos: GPU_PARITY_PASS | GPU_PARITY_FAIL | GPU_PARITY_PENDING_HUMAN_MACHINE
#            SERVER_BOOT_FAIL (servidor não subiu/ficou stale) | HARNESS_BOOTSTRAP_FAIL (projeto não abriu).
# SERVER_BOOT_FAIL e HARNESS_BOOTSTRAP_FAIL NÃO são falhas de GPU.
param([string]$Channel = "chrome", [int]$Port = 3100)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$editor = Join-Path $root "apps\editor"
$out = Join-Path $editor "gpu-parity-out"
$web = Join-Path $editor "apps\web"
foreach ($t in "node","bun","ffmpeg") { if (-not (Get-Command $t -ErrorAction SilentlyContinue)) { throw "$t não encontrado no PATH" } }

function Write-ServerBootFail([string]$reason) {
  $verdict = @{ verdict = "SERVER_BOOT_FAIL"; reason = $reason; port = $Port; note = "Falha de infraestrutura do harness; não é falha de GPU." }
  ($verdict | ConvertTo-Json) | Set-Content (Join-Path $out "gpu-parity-verdict.json")
  Write-Host "VERDICT: SERVER_BOOT_FAIL :: $reason" -ForegroundColor Red
  foreach ($f in "server.log","server.err.log") {
    $p = Join-Path $out $f
    if (Test-Path $p) { Write-Host "---- $f (últimas 40 linhas) ----"; Get-Content $p -Tail 40 }
  }
}

# 0) estado limpo: evidências antigas e QUALQUER servidor preso na porta (típico: o `next-server` filho
#    sobrevive quando só o wrapper `bunx` é encerrado — a rodada seguinte falaria com um servidor velho).
New-Item -ItemType Directory -Force -Path $out | Out-Null
Get-ChildItem $out -Include "gpu-parity-verdict.json","bootstrap-*","project-bootstrap.json","server*.log" -Recurse -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
$busy = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
if ($busy) {
  foreach ($c in $busy) {
    $proc = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
    Write-Host "Porta $Port ocupada por PID $($c.OwningProcess) ($($proc.ProcessName)); encerrando a árvore."
    & taskkill /PID $c.OwningProcess /T /F | Out-Null
  }
  Start-Sleep -Seconds 2
  if (Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue) { Write-ServerBootFail "porta $Port continua ocupada"; exit 2 }
}

Push-Location $editor
$srv = $null
try {
  bun install --frozen-lockfile
  Push-Location $web
  $env:NEXT_PUBLIC_CAPIA_E2E_HOOKS = "1"; $env:NEXT_PUBLIC_CAPIA_AGENT_BRIDGE = "1"
  bun run build
  Pop-Location

  # 1) servidor com logs preservados
  $srv = Start-Process -FilePath "bunx" -ArgumentList @("next","start","-p","$Port") -WorkingDirectory $web -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $out "server.log") -RedirectStandardError (Join-Path $out "server.err.log")
  $base = "http://127.0.0.1:$Port"
  $ready = $false
  for ($i = 0; $i -lt 90; $i++) {
    if ($srv.HasExited) { break }
    try { $r = Invoke-WebRequest -Uri "$base/projects" -UseBasicParsing -TimeoutSec 3; if ($r.StatusCode -eq 200) { $ready = $true; break } } catch { }
    Start-Sleep -Seconds 1
  }
  if (-not $ready) {
    $why = if ($srv.HasExited) { "o servidor saiu (código $($srv.ExitCode))" } else { "$base/projects não respondeu 200 em 90 s" }
    Write-ServerBootFail $why; exit 2
  }
  Write-Host "[SERVER] $base/projects => 200"

  # 2) o servidor serve os assets do build atual? (senão: servidor stale)
  node e2e\preflight.mjs $base
  if ($LASTEXITCODE -ne 0) { Write-ServerBootFail "servidor stale ou build inconsistente (assets /_next/ não servidos)"; exit 2 }

  # 3) bootstrap isolado (PROJECT_BOOTSTRAP) antes do teste de GPU
  $env:GPU_REAL = "1"; $env:BROWSER_CHANNEL = $Channel; $env:EDITOR_URL = $base
  node e2e\project-bootstrap.mjs
  if ($LASTEXITCODE -ne 0) {
    Write-Host "VERDICT: HARNESS_BOOTSTRAP_FAIL (não é falha de GPU). Evidência: $out\bootstrap-failure.png e bootstrap-diagnostics.json" -ForegroundColor Red
    exit 3
  }

  # 4) paridade de GPU
  node e2e\gpu-parity.mjs
  Write-Host "`n== veredito =="; Get-Content (Join-Path $out "gpu-parity-verdict.json") -Raw | Write-Host
} finally {
  if ($srv -and -not $srv.HasExited) { & taskkill /PID $srv.Id /T /F | Out-Null }   # mata a árvore (bunx + next-server)
  Pop-Location
}
Write-Host "Envie a pasta apps\editor\gpu-parity-out (zip) de volta."
