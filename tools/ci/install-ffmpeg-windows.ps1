# Instala FFmpeg/ffprobe no runner Windows de forma robusta (ADR-051).
# 1) cache local; 2) Chocolatey com retentativas; 3) fallback: zip do release no GitHub (GyanD/codexffmpeg).
# Falha (exit 1) se, no fim, ffprobe/ffmpeg não rodarem — nunca mascara a infraestrutura.
$ErrorActionPreference = 'Continue'

function Test-Ff { return [bool](Get-Command ffprobe -ErrorAction SilentlyContinue) -and [bool](Get-Command ffmpeg -ErrorAction SilentlyContinue) }

$dir = Join-Path $env:RUNNER_TEMP 'ffmpeg'
if (Test-Path "$dir\bin\ffprobe.exe") { $env:PATH = "$dir\bin;$env:PATH" }

if (-not (Test-Ff)) {
  for ($i = 1; $i -le 4 -and -not (Test-Ff); $i++) {
    Write-Host "choco install ffmpeg (tentativa $i/4)"
    choco install ffmpeg --no-progress -y
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $env:PATH = "$machine;$env:PATH"
    $shim = 'C:\ProgramData\chocolatey\bin'
    if (Test-Path "$shim\ffprobe.exe") { $env:PATH = "$shim;$env:PATH" }
    if (-not (Test-Ff)) { Start-Sleep -Seconds (10 * $i) }
  }
}

if (-not (Test-Ff)) {
  Write-Host 'Chocolatey indisponível: baixando o build do FFmpeg pelo GitHub'
  $ver = '8.0'
  $url = "https://github.com/GyanD/codexffmpeg/releases/download/$ver/ffmpeg-$ver-essentials_build.zip"
  $zip = Join-Path $env:RUNNER_TEMP 'ffmpeg.zip'
  for ($i = 1; $i -le 4; $i++) {
    try {
      Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -TimeoutSec 300
      break
    } catch {
      Write-Host "download falhou ($i/4): $($_.Exception.Message)"
      Start-Sleep -Seconds (10 * $i)
    }
  }
  if (Test-Path $zip) {
    Expand-Archive -Path $zip -DestinationPath $dir -Force
    $bin = Get-ChildItem -Path $dir -Recurse -Filter ffprobe.exe | Select-Object -First 1
    if ($bin) { $env:PATH = "$($bin.DirectoryName);$env:PATH" }
  }
}

if (-not (Test-Ff)) { Write-Error 'FFmpeg/ffprobe indisponíveis após todas as tentativas'; exit 1 }

# Persiste o PATH para os próximos passos do job
$ffdir = Split-Path (Get-Command ffprobe).Source
Add-Content -Path $env:GITHUB_PATH -Value $ffdir
ffprobe -version | Select-Object -First 1
ffmpeg -version | Select-Object -First 1
