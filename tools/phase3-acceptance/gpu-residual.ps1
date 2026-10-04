<#
.SYNOPSIS
  Residual da ADR-069 em UM comando: roda o preview P2 a 1280x720 (SharedBuffer -> canvas WebGL) em
  PC com GPU real, mede CPU/pacing e imprime PASS/FAIL contra os limiares da ADR-069. Não é preciso
  abrir logs: o resultado fica em gpu-residual-result.json (envie esse arquivo se FAIL).

.NOTES
  Não mexa no mouse/teclado (~3 min). Notebook na tomada, janela no monitor principal.
  Limiares (ADR-069): CPU do pipeline <= 25 % da máquina; slots perdidos <= 1 %; sem artefato de resize.
  Compatível com Windows PowerShell 5.1.
#>
[CmdletBinding()]
param([string]$Label = "gpu-residual", [double]$CpuMaxPercent = 25, [double]$MissedMaxPercent = 1, [double]$ResizeBadMax = 0)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$s1 = Join-Path $here "..\s1-preview-spike"
if (-not (Test-Path (Join-Path $s1 "run.ps1"))) { Write-Host "ERRO: tools\s1-preview-spike\run.ps1 não encontrado ao lado deste script." -ForegroundColor Red; exit 2 }

Push-Location $s1
try { & powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 -Label $Label -Modes p2 -P2Res 1280x720 -SkipManual -NoCountdown } finally { Pop-Location }
$report = Get-ChildItem (Join-Path $s1 "reports") -Filter "s1-report-$Label-*.json" | Where-Object { $_.Name -notlike "*.harness.json" } |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $report) { Write-Host "FAIL: o harness não gerou relatório." -ForegroundColor Red; exit 1 }
$full = Get-Content $report.FullName -Raw | ConvertFrom-Json
$r = $full.harness

function Phase($name) { foreach ($ph in @($r.modes.p2.phases)) { if ($ph.name -eq $name) { return $ph } }; return $null }
$status = $r.summary.p2.status
$cpuPhase = Phase "steady_cpu"
$cpu = $null
if ($cpuPhase) { $cpu = $cpuPhase.result.resources.percent_of_machine.total }
$eng = $r.modes.p2.engine_stats_whole_mode
$produced = [double]$eng.frames_produced; $late = [double]$eng.frames_that_missed_their_slot
$missed = if ($produced -gt 0) { 100.0 * $late / $produced } else { $null }
$rs = Phase "resize"
$bad = if ($rs) { $rs.result.continuous_resize.bad_fraction } else { $null }
$gpu = ($full.host.gpus | ForEach-Object { $_.name }) -join "; "

$checks = @()
$checks += [pscustomobject]@{ check = "medição completa (status MEASURED)"; value = $status; limit = "MEASURED"; pass = ($status -eq "MEASURED") }
$checks += [pscustomobject]@{ check = "CPU do pipeline (% da máquina)"; value = $cpu; limit = "<= $CpuMaxPercent"; pass = ($null -ne $cpu -and $cpu -le $CpuMaxPercent) }
$checks += [pscustomobject]@{ check = "slots perdidos (%)"; value = $missed; limit = "<= $MissedMaxPercent"; pass = ($null -ne $missed -and $missed -le $MissedMaxPercent) }
$checks += [pscustomobject]@{ check = "resize contínuo: fração ruim"; value = $bad; limit = "<= $ResizeBadMax"; pass = ($null -ne $bad -and $bad -le $ResizeBadMax) }
$overall = -not ($checks | Where-Object { -not $_.pass })

$result = [pscustomobject]@{
  adr = "ADR-069"; resolution = "1280x720"; gpu = $gpu; report = $report.Name
  checks = $checks; verdict = $(if ($overall) { "PASS" } else { "FAIL" })
  note = "FAIL dispara o gatilho de reabertura da OD-1 (ADR-069). Não alterar os limiares."
}
$result | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $here "gpu-residual-result.json") -Encoding UTF8
Write-Host ""
Write-Host "GPU: $gpu"
$checks | Format-Table check, value, limit, pass -AutoSize
if ($overall) { Write-Host "RESULTADO: PASS (ADR-069: P2 a 720p dentro dos limiares)" -ForegroundColor Green; exit 0 }
Write-Host "RESULTADO: FAIL — envie gpu-residual-result.json e o ZIP em tools\s1-preview-spike\reports" -ForegroundColor Red
exit 1
