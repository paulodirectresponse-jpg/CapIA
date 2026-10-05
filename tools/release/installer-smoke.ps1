<#
.SYNOPSIS
  Smoke test do instalador NSIS no Windows (usado por .github/workflows/installer.yml; também na matriz de
  máquina limpa — docs/phase6/IMPL_DESKTOP_DISTRIBUTION.md §Matriz).

.DESCRIPTION
  1. instala em silêncio, por usuário;  2. `--version` e `--self-test --require-media` a partir do local INSTALADO;
  3. abre o app por alguns segundos e confere as pastas de primeira execução (sem credenciais de IA);
  4. cria um projeto .capia "do usuário" em Documentos, desinstala em silêncio e exige que ele sobreviva;
  5. reinstala e roda o self-test de novo.
  Grava smoke-report.json; sai com 1 se qualquer passo falhar. Nada de rede além da instalação do WebView2 se faltar.
#>
param(
  [Parameter(Mandatory = $true)][string]$Installer,
  [string]$ReportPath = 'smoke-report.json',
  [int]$LaunchSeconds = 15
)
$ErrorActionPreference = 'Stop'
$steps = New-Object System.Collections.ArrayList
$ok = $true
function Step([string]$name, [scriptblock]$body) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  try { $detail = & $body; [void]$steps.Add([ordered]@{ name = $name; ok = $true; ms = $sw.ElapsedMilliseconds; detail = $detail }) }
  catch { $script:ok = $false; [void]$steps.Add([ordered]@{ name = $name; ok = $false; ms = $sw.ElapsedMilliseconds; error = $_.Exception.Message }); Write-Host "FALHOU: $name — $($_.Exception.Message)" }
}

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\CapIA'
$appData = Join-Path $env:APPDATA 'app.capia.desktop'
$projectDir = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'CapIA-smoke'
$project = Join-Path $projectDir 'meu filme (não apagar).capia'

function Find-AppExe {
  foreach ($n in 'CapIA.exe', 'capia-desktop.exe') {
    $p = Join-Path $installDir $n
    if (Test-Path $p) { return $p }
  }
  throw "executável principal não encontrado em $installDir"
}
function Install { $p = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru; if ($p.ExitCode -ne 0) { throw "instalador saiu com $($p.ExitCode)" } }
function SelfTest([string]$exe, [string]$out) {
  $p = Start-Process -FilePath $exe -ArgumentList @('--self-test', '--require-media', '--out', $out) -Wait -PassThru -WindowStyle Hidden
  if (-not (Test-Path $out)) { throw 'self-test não gravou o relatório' }
  $r = Get-Content $out -Raw | ConvertFrom-Json
  if ($p.ExitCode -ne 0 -or -not $r.ok) { throw "self-test falhou (código $($p.ExitCode)): $(Get-Content $out -Raw)" }
  return @{ version = $r.version; steps = $r.steps.Count; total_ms = $r.total_ms }
}

Step 'install.silent.per-user' {
  Install
  if (-not (Test-Path $installDir)) { throw "diretório por usuário esperado ausente: $installDir" }
  @{ dir = $installDir }
}
$exe = $null
Step 'install.layout' {
  $script:exe = Find-AppExe
  foreach ($f in 'ffmpeg\ffmpeg.exe', 'ffmpeg\ffprobe.exe', 'THIRD_PARTY_NOTICES.txt') {
    if ($f -eq 'THIRD_PARTY_NOTICES.txt' -and -not (Test-Path (Join-Path $installDir $f))) { continue }
    if (-not (Test-Path (Join-Path $installDir $f))) { throw "ausente no instalado: $f" }
  }
  @{ exe = (Split-Path $script:exe -Leaf); server = (Test-Path (Join-Path $installDir 'capia-server.exe')) }
}
Step 'version' {
  $out = Join-Path $env:RUNNER_TEMP 'version.txt'
  Start-Process -FilePath $script:exe -ArgumentList @('--version', '--out', $out) -Wait -WindowStyle Hidden | Out-Null
  $v = (Get-Content $out -Raw).Trim()
  if ($v -notmatch '^CapIA \d+\.\d+\.\d+') { throw "saída inesperada: $v" }
  @{ version = $v }
}
Step 'self-test.installed' { SelfTest $script:exe (Join-Path $env:RUNNER_TEMP 'selftest-1.json') }
Step 'first-run.dirs' {
  $p = Start-Process -FilePath $script:exe -PassThru
  Start-Sleep -Seconds $LaunchSeconds
  $alive = -not $p.HasExited
  if ($alive) { Stop-Process -Id $p.Id -Force }
  if (-not $alive) { throw "o app encerrou sozinho (código $($p.ExitCode)) na primeira execução" }
  if (-not (Test-Path $appData)) { throw "pasta de dados não criada: $appData" }
  foreach ($d in 'logs', 'support') { if (-not (Test-Path (Join-Path $appData $d))) { throw "pasta esperada ausente: $d" } }
  # sem credenciais de IA: o app abre com a IA desligada e nenhum segredo foi gravado
  if (Get-ChildItem $appData -Recurse -File | Select-String -Pattern 'sk-[A-Za-z0-9]{20,}' -List) { throw 'parece haver chave de API gravada' }
  @{ appdata = $appData }
}
Step 'user-project.created' {
  New-Item -ItemType Directory -Force -Path $projectDir | Out-Null
  [IO.File]::WriteAllBytes($project, [byte[]](1..255))
  @{ path = $project; bytes = (Get-Item $project).Length }
}
Step 'uninstall.silent.keeps-projects' {
  $u = Get-ChildItem $installDir -Filter 'uninstall*.exe' | Select-Object -First 1
  if (-not $u) { throw 'desinstalador ausente' }
  $p = Start-Process -FilePath $u.FullName -ArgumentList '/S' -Wait -PassThru
  if ($p.ExitCode -ne 0) { throw "desinstalador saiu com $($p.ExitCode)" }
  Start-Sleep -Seconds 3
  if (-not (Test-Path $project)) { throw 'O PROJETO DO USUÁRIO FOI APAGADO PELO DESINSTALADOR' }
  if ((Get-Item $project).Length -ne 255) { throw 'o projeto do usuário foi alterado' }
  $left = if (Test-Path $installDir) { (Get-ChildItem $installDir -Recurse -File -ErrorAction SilentlyContinue | Measure-Object).Count } else { 0 }
  @{ project_survived = $true; files_left_in_install_dir = $left }
}
Step 'reinstall' {
  Install
  $script:exe = Find-AppExe
  SelfTest $script:exe (Join-Path $env:RUNNER_TEMP 'selftest-2.json')
}
Step 'cleanup' {
  $u = Get-ChildItem $installDir -Filter 'uninstall*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($u) { Start-Process -FilePath $u.FullName -ArgumentList '/S' -Wait | Out-Null }
  if (Test-Path $projectDir) { Remove-Item $projectDir -Recurse -Force }
  @{}
}

[ordered]@{ ok = $ok; installer = (Split-Path $Installer -Leaf); steps = $steps } | ConvertTo-Json -Depth 6 | Set-Content -Path $ReportPath -Encoding UTF8
if (-not $ok) { exit 1 }
Write-Host 'installer smoke: ok'
