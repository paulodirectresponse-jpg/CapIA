<#
.SYNOPSIS
  S1 - Preview Surface spike (CapIA). Mede P1 (HWND nativo sob WebView2 transparente) e P2 (SharedBuffer -> canvas)
  em Windows 11 e gera um relatorio estruturado para enviar a analise.

.EXAMPLE
  .\run.ps1
  .\run.ps1 -Label "150pct-igpu"          # repetir em 100%/150%/200% de escala e/ou com 2 monitores
  .\run.ps1 -SteadySecs 20 -Modes p1,p2   # fases estaveis mais longas, sem o controle p1_above

.NOTES
  NAO mexa no mouse/teclado durante a execucao (~3 min): o teste usa captura de tela e entrada sintetica.
  Mantenha o notebook na tomada e a janela no monitor principal. Compativel com Windows PowerShell 5.1.
#>
[CmdletBinding()]
param(
  [string]$Label = "",
  [int]$SteadySecs = 10,
  [string]$Modes = "p1,p1_above,p2",
  [string]$P2Res = "960x540",
  [switch]$SkipBuild,
  [switch]$SkipManual,
  [switch]$SkipGpuCounters,
  [switch]$SkipInput,
  [switch]$Quick,
  [switch]$NoCountdown
)

if ($Quick) { $SkipManual = $true; $SkipGpuCounters = $true; $NoCountdown = $true }
$ErrorActionPreference = "Stop"
$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Here
$Stamp = (Get-Date).ToString("yyyyMMdd-HHmmss")
$SafeLabel = ($Label -replace '[^A-Za-z0-9_-]', '')
$Name = if ($SafeLabel) { "s1-report-$SafeLabel-$Stamp" } else { "s1-report-$Stamp" }
$OutDir = Join-Path $Here "reports"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$RawReport = Join-Path $OutDir "$Name.harness.json"
$LogFile = Join-Path $OutDir "$Name.log"
$EventsFile = Join-Path $OutDir "$Name.events.jsonl"
$BuildLog = Join-Path $OutDir "$Name.build.log"
$FinalJson = Join-Path $OutDir "$Name.json"
$FinalMd = Join-Path $OutDir "$Name.md"
$Zip = Join-Path $OutDir "$Name.zip"

function Say([string]$m) { Write-Host "[s1] $m" }
function Fail([string]$m) { Write-Host "[s1] ERRO: $m" -ForegroundColor Red; exit 1 }

# ---------- 1. Pre-checagens ----------
$os = Get-CimInstance Win32_OperatingSystem
$build = [int]$os.BuildNumber
Say "Windows: $($os.Caption) build $build ($($os.OSArchitecture))"
if ($build -lt 22000) { Write-Host "[s1] AVISO: o alvo e Windows 11 (build >= 22000). Seguindo mesmo assim; registrado no relatorio." -ForegroundColor Yellow }
if (-not [Environment]::Is64BitOperatingSystem) { Fail "Windows 64 bits e necessario." }

$wv2Key = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
$wv2Key2 = "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
$wv2 = $null
foreach ($k in @($wv2Key, $wv2Key2)) { if (-not $wv2 -and (Test-Path $k)) { $wv2 = (Get-ItemProperty $k).pv } }
if (-not $wv2) { Fail "WebView2 Runtime nao encontrado. Instale: https://developer.microsoft.com/microsoft-edge/webview2/ (Evergreen Runtime) e rode de novo." }
Say "WebView2 Runtime: $wv2"

if (-not $SkipBuild) {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Fail "Rust nao encontrado. Instale com:  winget install Rustlang.Rustup   e tambem 'Visual Studio Build Tools' com a carga 'Desktop development with C++' (winget install Microsoft.VisualStudio.2022.BuildTools --override '--add Microsoft.VisualStudio.Workload.VCTools --passive'). Reabra o terminal e rode de novo."
  }
  $host_triple = ((& rustc -vV) | Select-String '^host:' | Select-Object -First 1).ToString()
  Say "Rust: $((& rustc --version)) ($host_triple)"
  if ($host_triple -notmatch "msvc") { Write-Host "[s1] AVISO: o toolchain nao e MSVC; o alvo do projeto e x86_64-pc-windows-msvc." -ForegroundColor Yellow }
  Say "Compilando o harness (release). A primeira compilacao leva alguns minutos..."
  # Windows PowerShell 5.1: com ErrorActionPreference=Stop, QUALQUER linha em stderr de um programa nativo
  # (o cargo escreve o progresso em stderr) vira erro terminante. Relaxa so durante a chamada nativa.
  $prevEap = $ErrorActionPreference; $ErrorActionPreference = "Continue"
  & cargo build --release 2>&1 | ForEach-Object { "$_" } | Tee-Object -FilePath $BuildLog | Select-Object -Last 3
  $cargoExit = $LASTEXITCODE
  $ErrorActionPreference = $prevEap
  if ($cargoExit -ne 0) { Write-Host (Get-Content $BuildLog -Tail 25 | Out-String); Fail "cargo build falhou (codigo $cargoExit); veja $BuildLog" }
}
$Exe = Join-Path $Here "target\release\capia-s1-harness.exe"
if (-not (Test-Path $Exe)) { Fail "Executavel nao encontrado: $Exe" }

# ---------- 2. Informacoes do host ----------
Say "Coletando informacoes do sistema..."
Add-Type -AssemblyName System.Windows.Forms
$gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
  [pscustomobject]@{ name = $_.Name; driver_version = $_.DriverVersion; driver_date = "$($_.DriverDate)"; adapter_ram_bytes = $_.AdapterRAM; video_processor = $_.VideoProcessor; current_refresh_hz = $_.CurrentRefreshRate; current_resolution = "$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)" }
})
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$screens = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { [pscustomobject]@{ device = $_.DeviceName; primary = $_.Primary; bounds = "$($_.Bounds.Width)x$($_.Bounds.Height)@($($_.Bounds.X),$($_.Bounds.Y))" } })
$applied = $null; try { $applied = (Get-ItemProperty "HKCU:\Control Panel\Desktop\WindowMetrics" -ErrorAction Stop).AppliedDPI } catch {}
$battery = @(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
$plan = (& powercfg /getactivescheme) 2>$null
$HostInfo = [pscustomobject]@{
  os = $os.Caption; os_build = $build; os_version = $os.Version; webview2_runtime = $wv2
  cpu = $cpu.Name; cpu_physical_cores = $cpu.NumberOfCores; cpu_logical_cores = $cpu.NumberOfLogicalProcessors
  ram_gb = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1); gpus = $gpus; screens = $screens
  applied_dpi_primary = $applied; has_battery = ($battery.Count -gt 0); on_ac_power = if ($battery.Count -gt 0) { ($battery[0].BatteryStatus -ne 1) } else { $null }; power_plan = "$plan"
  powershell = "$($PSVersionTable.PSVersion)"; label = $Label; run_stamp = $Stamp
}
Say ("GPU(s): " + (($gpus | ForEach-Object { $_.name }) -join "; ") + " | Monitores: $($screens.Count) | DPI aplicado (primario): $applied")

# ---------- 3. Execucao ----------
Write-Host ""
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host " O teste vai abrir uma janela e rodar ~3 minutos sozinho." -ForegroundColor Cyan
Write-Host " NAO mexa no mouse nem no teclado. Observe a janela (flicker?)." -ForegroundColor Cyan
Write-Host "================================================================" -ForegroundColor Cyan
if (-not $NoCountdown) { for ($i = 5; $i -gt 0; $i--) { Write-Host -NoNewline "$i "; Start-Sleep -Seconds 1 }; Write-Host "" }

# Uma unica string com aspas: -ArgumentList com array NAO cita caminhos com espacos no Windows PowerShell 5.1.
$HarnessArgs = '--out "{0}" --events "{5}" --modes {1} --steady-secs {2} --p2-res {3} --label "{4}"' -f $RawReport, $Modes, $SteadySecs, $P2Res, $SafeLabel, $EventsFile
if ($SkipInput) { $HarnessArgs += " --skip-input" }
$proc = Start-Process -FilePath $Exe -ArgumentList $HarnessArgs -PassThru -RedirectStandardError $LogFile -RedirectStandardOutput (Join-Path $OutDir "$Name.stdout.log")
Say "Harness iniciado (pid $($proc.Id))."

$StopFile = Join-Path $OutDir "$Name.stop"
$job = $null
if (-not $SkipGpuCounters) {
  $job = Start-Job -ArgumentList $proc.Id, $StopFile -ScriptBlock {
    param($rootPid, $stopFile)
    function Get-Tree([int]$root) {
      $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
      $set = New-Object System.Collections.Generic.HashSet[int]; [void]$set.Add($root)
      $grew = $true
      while ($grew) { $grew = $false; foreach ($p in $all) { if ($set.Contains([int]$p.ParentProcessId) -and -not $set.Contains([int]$p.ProcessId)) { [void]$set.Add([int]$p.ProcessId); $grew = $true } } }
      return $set
    }
    while (-not (Test-Path $stopFile)) {
      $t = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
      $tree = @(Get-Tree $rootPid)
      $s = $null
      try { $s = Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop } catch {}
      $rows = @()
      if ($s) { foreach ($c in $s.CounterSamples) { if ($c.CookedValue -gt 0 -and $c.InstanceName -match 'pid_(\d+)_.*_engtype_(\w+)') { $rows += [pscustomobject]@{ pid = [int]$Matches[1]; engtype = $Matches[2]; v = [math]::Round($c.CookedValue, 2) } } } }
      [pscustomobject]@{ t = $t; tree = $tree; rows = $rows }
    }
  }
  Say "Contadores de GPU do Windows ativos (1 Hz)."
} else { Say "Contadores de GPU desativados (-SkipGpuCounters)." }

$deadline = (Get-Date).AddMinutes(25)
while (-not $proc.HasExited) {
  if ((Get-Date) -gt $deadline) { Say "Tempo limite (25 min): encerrando o harness."; Stop-Process -Id $proc.Id -Force; break }
  Start-Sleep -Seconds 2
}
$exit = if ($proc.HasExited) { $proc.ExitCode } else { $null }
Say "Harness encerrado (codigo $exit)."
$gpuSamples = @()
if ($job) { New-Item -ItemType File -Force -Path $StopFile | Out-Null; [void](Wait-Job $job -Timeout 15); $gpuSamples = @(Receive-Job $job); Remove-Job $job -Force; Remove-Item $StopFile -Force -ErrorAction SilentlyContinue }

if (-not (Test-Path $RawReport)) {
  Write-Host "[s1] O harness nao gerou relatorio. Veja o log:" -ForegroundColor Red; Get-Content $LogFile -Tail 30
  $Report = [pscustomobject]@{ schema = "capia-s1-report/1"; fatal = "harness nao gerou relatorio"; harness_exit_code = $exit }
} else {
  try { $Report = Get-Content $RawReport -Raw | ConvertFrom-Json }
  catch {
    Write-Host "[s1] AVISO: nao consegui interpretar o JSON do harness ($($_.Exception.Message)); o arquivo cru vai no ZIP." -ForegroundColor Yellow
    $Report = [pscustomobject]@{ schema = "capia-s1-report/1"; fatal = "JSON do harness ilegivel no PowerShell"; raw_file = (Split-Path -Leaf $RawReport) }
  }
}

# ---------- 4. Correlacao com contadores de GPU por fase ----------
function Add-GpuToPhase($phase, $samples) {
  if (-not $samples -or $samples.Count -eq 0 -or -not $phase.started_utc_ms) { return }
  $win = @($samples | Where-Object { $_.t -ge $phase.started_utc_ms -and $_.t -le $phase.ended_utc_ms })
  $byApp = @{}; $byAll = @{}
  foreach ($s in $win) {
    $treeSet = @{}; foreach ($p in @($s.tree)) { $treeSet[[int]$p] = $true }
    $app = @{}; $all = @{}
    foreach ($r in @($s.rows)) {
      $all[$r.engtype] = [double]$all[$r.engtype] + $r.v
      if ($treeSet.ContainsKey([int]$r.pid)) { $app[$r.engtype] = [double]$app[$r.engtype] + $r.v }
    }
    foreach ($k in $app.Keys) { if (-not $byApp[$k]) { $byApp[$k] = New-Object System.Collections.ArrayList }; [void]$byApp[$k].Add($app[$k]) }
    foreach ($k in $all.Keys) { if (-not $byAll[$k]) { $byAll[$k] = New-Object System.Collections.ArrayList }; [void]$byAll[$k].Add($all[$k]) }
  }
  $avg = { param($h, $n) $o = [ordered]@{}; foreach ($k in $h.Keys) { $o[$k] = [math]::Round((($h[$k] | Measure-Object -Sum).Sum / [math]::Max($n, 1)), 2) }; $o }
  $gpu = [pscustomobject]@{
    samples_in_phase = $win.Count
    app_tree_percent_by_engine = (& $avg $byApp $win.Count)
    all_processes_percent_by_engine = (& $avg $byAll $win.Count)
    note = "Soma das instancias do contador 'GPU Engine' por tipo de engine, media por amostra (ausencia de amostra = 0). Processos do app = harness + filhos (msedgewebview2). Amostragem de 1 Hz."
  }
  $phase | Add-Member -NotePropertyName gpu_counters -NotePropertyValue $gpu -Force
}
if ($Report.modes) {
  foreach ($m in $Report.modes.PSObject.Properties) {
    foreach ($ph in @($m.Value.phases)) { if ($ph -and $ph.started_utc_ms) { Add-GpuToPhase $ph $gpuSamples } }
  }
}

# ---------- 5. Observacoes manuais ----------
$manual = [ordered]@{}
if (-not $SkipManual -and (Test-Path $RawReport)) {
  Write-Host ""
  Write-Host "Observacoes MANUAIS (responda o que voce viu durante o teste; Enter = nao observei):" -ForegroundColor Cyan
  $qs = @(
    @("p1_resize_flicker", "P1: no redimensionamento continuo o preview piscou, ficou preto/branco ou desalinhado do quadro? (s/n + detalhe)"),
    @("p2_resize_flicker", "P2: idem para o modo P2."),
    @("overlay_html_visible", "O overlay vermelho/menu HTML apareceu SOBRE o preview em P1? E em P1_above (controle)?"),
    @("fullscreen_ok", "Em tela cheia o preview ficou correto (sem faixa preta, sem erro) em P1 e P2?"),
    @("smoothness", "O movimento da barra amarela pareceu fluido (sem engasgos) em P1 e em P2?"),
    @("tearing", "Viu tearing (linha cortando a imagem)?"),
    @("anything_else", "Qualquer outra anomalia visual ou comportamento estranho?")
  )
  foreach ($q in $qs) { $manual[$q[0]] = Read-Host $q[1] }
}

# ---------- 6. Relatorio final ----------
$Final = [pscustomobject]@{
  schema = "capia-s1-final/1"; generated_utc = (Get-Date).ToUniversalTime().ToString("o")
  host = $HostInfo; harness = $Report; manual_observations = $manual
  harness_exit_code = $exit; gpu_counter_samples_total = $gpuSamples.Count
  unverified_notice = "Pacote criado em ambiente sem Windows (so compile-check por cross-compilacao). Erros/campos ausentes sao dados para analise."
}
$Final | ConvertTo-Json -Depth 100 | Set-Content -Path $FinalJson -Encoding UTF8

function G($o, [string[]]$path) { foreach ($p in $path) { if ($null -eq $o) { return $null }; $o = $o.$p }; return $o }
function Phase($mode, $name) { $m = G $Report @("modes", $mode); if (-not $m) { return $null }; foreach ($ph in @($m.phases)) { if ($ph.name -eq $name) { return $ph } }; return $null }
$md = New-Object System.Text.StringBuilder
[void]$md.AppendLine("# S1 - relatorio ($Name)")
[void]$md.AppendLine("")
[void]$md.AppendLine("Sistema: $($HostInfo.os) build $($HostInfo.os_build) . WebView2 $wv2 . GPU: " + (($gpus | ForEach-Object { $_.name }) -join "; ") + " . monitores: $($screens.Count) . DPI aplicado: $applied . label: '$Label'")
[void]$md.AppendLine("")
[void]$md.AppendLine("| Modo | status | pattern visivel sob WebView transparente | overlay HTML sobre o pattern | fps medio | intervalo p95 (ms) | latencia submissao->tela p50/p95 (ms) | CPU total (nucleos) | resize continuo: fracao ruim |")
[void]$md.AppendLine("|---|---|---|---|---|---|---|---|---|")
foreach ($mode in @("p1", "p1_above", "p2")) {
  $probe = Phase $mode "probe_static"; $lat = Phase $mode "steady_latency"; $cpuP = Phase $mode "steady_cpu"; $rs = Phase $mode "resize"
  $eng = G $Report @("modes", $mode, "engine_stats_whole_mode")
  $fps = if ($eng.avg_fps) { [math]::Round($eng.avg_fps, 1) } else { "-" }
  $p95 = G $eng @("present_interval_ms", "p95"); if (-not $p95) { $p95 = G $eng @("produce_total_ms", "p95") }
  $l50 = G $lat @("result", "screen_latency", "submit_to_visible_ms", "p50"); $l95 = G $lat @("result", "screen_latency", "submit_to_visible_ms", "p95")
  $cores = G $cpuP @("result", "resources", "cores_used", "total")
  $bad = G $rs @("result", "continuous_resize", "bad_fraction")
  $st = G $Report @("summary", $mode, "status"); if (-not $st) { $st = "NAO EXECUTADO" }
  [void]$md.AppendLine("| $mode | $st | $(G $probe @('result','native_or_canvas_pattern_visible_at_probe')) | $(G $probe @('result','html_overlay_composited_over_pattern')) | $fps | $p95 | $l50 / $l95 | $cores | $bad |")
}
[void]$md.AppendLine("")
[void]$md.AppendLine("## Estado de cada medicao")
foreach ($mode in @("p1", "p1_above", "p2")) {
  $sm = G $Report @("summary", $mode)
  if ($sm) {
    [void]$md.AppendLine("- **$mode**: $($sm.status) (fases ok: $($sm.phases_ok), com falha: $($sm.phases_failed))")
    foreach ($r in @($sm.failure_reasons)) { if ($r) { [void]$md.AppendLine("    - $r") } }
  } else { [void]$md.AppendLine("- **$mode**: nao executado (veja setup_error no JSON)") }
}
$reading = G $Report @("summary", "findings", "harness_validity_and_p1_reading")
if ($reading) { [void]$md.AppendLine(""); [void]$md.AppendLine("Leitura automatica: $reading") }
$setupErr = G $Report @("setup_error"); if ($setupErr) { [void]$md.AppendLine(""); [void]$md.AppendLine("ERRO DE SETUP DO HARNESS: $setupErr") }
[void]$md.AppendLine("")
[void]$md.AppendLine("Este resumo NAO decide OD-1. A decisao segue a regra de docs/spikes/S1-preview-surface.md ?4, aplicada pelo analista sobre o JSON completo.")
[void]$md.AppendLine("Observacoes manuais: " + (($manual.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " | "))
$md.ToString() | Set-Content -Path $FinalMd -Encoding UTF8

$zipItems = @($FinalJson, $FinalMd, $LogFile, $EventsFile, $RawReport, $BuildLog) | Where-Object { Test-Path $_ }
Write-Host ""
Write-Host "---------------- RESULTADO POR MODO ----------------" -ForegroundColor Cyan
foreach ($mode in @("p1", "p1_above", "p2")) {
  $sm = G $Report @("summary", $mode)
  if ($sm) {
    $color = if ($sm.status -eq "MEASURED") { "Green" } elseif ($sm.status -eq "PARTIAL") { "Yellow" } else { "Red" }
    Write-Host ("  {0,-9} {1}  (fases ok {2}, falhas {3})" -f $mode, $sm.status, $sm.phases_ok, $sm.phases_failed) -ForegroundColor $color
    foreach ($r in @($sm.failure_reasons)) { if ($r) { Write-Host "      - $r" -ForegroundColor $color } }
  } else { Write-Host "  $mode  NAO EXECUTADO" -ForegroundColor Red }
}
if ($reading) { Write-Host "  $reading" -ForegroundColor Cyan }
if ($setupErr) { Write-Host "  ERRO DE SETUP: $setupErr" -ForegroundColor Red }
Write-Host "----------------------------------------------------" -ForegroundColor Cyan
Compress-Archive -Path $zipItems -DestinationPath $Zip -Force
Write-Host ""
Write-Host "================================================================" -ForegroundColor Green
Write-Host " Pronto. Envie este arquivo para analise:" -ForegroundColor Green
Write-Host "   $Zip" -ForegroundColor Green
Write-Host " (contem $Name.json, .md e o log; nao ha dados pessoais alem do nome da GPU/CPU/monitores)" -ForegroundColor Green
Write-Host " Para escala de DPI diferente / 2 monitores: mude a configuracao e rode:  .\run.ps1 -Label 150pct" -ForegroundColor Green
Write-Host "================================================================" -ForegroundColor Green
