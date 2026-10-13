# Spike H.264/AAC no WebView2 REAL (Windows PowerShell 5.1, runner/PC Windows).
#   .\run-webview2.ps1 -Mediabunny <caminho\mediabunny.min.mjs> [-Out out-webview2]
# Hospeda spike.html num controle WebView2 (WinForms, SDK via NuGet), recebe o relatório por fetch POST
# e deixa verify.mjs (ffprobe) decidir H264_PASS / H264_FAIL. Requer: node, ffprobe, WebView2 Runtime.
param(
  [Parameter(Mandatory = $true)][string]$Mediabunny,
  [string]$Out = "out-webview2",
  [int]$Port = 8767,
  [int]$TimeoutSecs = 180
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$Out = Join-Path $here $Out
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$work = Join-Path $env:TEMP "capia-webview2-sdk"
New-Item -ItemType Directory -Force -Path $work | Out-Null

# 1) SDK do WebView2 (managed + loader nativo)
$nuget = Join-Path $work "nuget.exe"
if (-not (Test-Path $nuget)) { Invoke-WebRequest "https://dist.nuget.org/win-x86-commandline/latest/nuget.exe" -OutFile $nuget }
& $nuget install Microsoft.Web.WebView2 -Version 1.0.2792.45 -OutputDirectory $work -NonInteractive | Out-Null
$pkg = Get-ChildItem $work -Directory -Filter "Microsoft.Web.WebView2.*" | Select-Object -First 1
$lib = Join-Path $pkg.FullName "lib\net462"
$native = Join-Path $pkg.FullName "runtimes\win-x64\native"
Copy-Item (Join-Path $native "WebView2Loader.dll") $lib -Force
Add-Type -Path (Join-Path $lib "Microsoft.Web.WebView2.Core.dll")
Add-Type -Path (Join-Path $lib "Microsoft.Web.WebView2.WinForms.dll")
Add-Type -AssemblyName System.Windows.Forms

# 2) servidor local
$server = Start-Process node -ArgumentList @((Join-Path $here "server.mjs"), $Port, $Out, $Mediabunny) -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 2

try {
  $form = New-Object System.Windows.Forms.Form
  $form.Width = 800; $form.Height = 600
  $wv = New-Object Microsoft.Web.WebView2.WinForms.WebView2
  $wv.Dock = "Fill"
  $props = New-Object Microsoft.Web.WebView2.WinForms.CoreWebView2CreationProperties
  $props.UserDataFolder = Join-Path $work "userdata"
  $wv.CreationProperties = $props
  $form.Controls.Add($wv)
  $form.Show()
  $init = $wv.EnsureCoreWebView2Async($null)
  $deadline = (Get-Date).AddSeconds(60)
  while (-not $init.IsCompleted -and (Get-Date) -lt $deadline) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 50 }
  if (-not $init.IsCompleted) { throw "WebView2 init timeout (runtime missing?)" }
  $version = [Microsoft.Web.WebView2.Core.CoreWebView2Environment]::GetAvailableBrowserVersionString()
  Set-Content (Join-Path $Out "webview2-version.txt") $version
  Write-Host "WebView2 runtime: $version"
  $wv.CoreWebView2.Navigate("http://127.0.0.1:$Port/spike.html")
  $deadline = (Get-Date).AddSeconds($TimeoutSecs)
  while (-not (Test-Path (Join-Path $Out "report.json")) -and (Get-Date) -lt $deadline) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }
} finally {
  if ($form) { $form.Close() }
  if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
}
node (Join-Path $here "verify.mjs") $Out "webview2"
exit $LASTEXITCODE
