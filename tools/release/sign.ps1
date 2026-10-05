<#
.SYNOPSIS
  Assina artefatos Windows (Authenticode) ou os rotula como UNSIGNED-DEV-TEST-ONLY.

.DESCRIPTION
  Certificado SOMENTE de segredos de CI (variáveis de ambiente); nada no repositório, nada em log:
    CAPIA_SIGN_PFX_BASE64     PFX em base64
    CAPIA_SIGN_PFX_PASSWORD   senha do PFX
    CAPIA_SIGN_TIMESTAMP_URL  (opcional) servidor RFC 3161; padrão http://timestamp.digicert.com
  Modos:
    real      (padrão com segredos presentes) assina com o PFX e grava signing.json {status: signed}
    -TestMode cria um certificado autoassinado DESCARTÁVEL em tempo de execução (prova o pipeline; não é
              confiável fora da máquina) e grava {status: test_signed}
    sem cert  copia o artefato para <nome>-UNSIGNED-DEV-TEST-ONLY<ext> e grava {status: external_pending}
  Nunca finge assinatura: o status descreve o que realmente aconteceu.
#>
param(
  [Parameter(Mandatory = $true)][string[]]$Path,
  [string]$ReportPath = 'signing.json',
  [switch]$TestMode
)
$ErrorActionPreference = 'Stop'

function Find-SignTool {
  $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
  if (Test-Path $kits) {
    $f = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
    if ($f) { return $f.FullName }
  }
  return $null
}

$hasReal = [bool]$env:CAPIA_SIGN_PFX_BASE64 -and [bool]$env:CAPIA_SIGN_PFX_PASSWORD
$ts = if ($env:CAPIA_SIGN_TIMESTAMP_URL) { $env:CAPIA_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
$results = @()
$tmpPfx = $null
$cleanupThumb = $null

try {
  if ($TestMode) { $mode = 'test' } elseif ($hasReal) { $mode = 'real' } else { $mode = 'none' }
  $signtool = $null
  if ($mode -ne 'none') {
    $signtool = Find-SignTool
    if (-not $signtool) { throw 'signtool.exe não encontrado (instale o Windows SDK)' }
  }
  if ($mode -eq 'real') {
    $tmpPfx = Join-Path $env:RUNNER_TEMP ([guid]::NewGuid().ToString() + '.pfx')
    [IO.File]::WriteAllBytes($tmpPfx, [Convert]::FromBase64String($env:CAPIA_SIGN_PFX_BASE64))
  }
  if ($mode -eq 'test') {
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=CapIA CI TEST ONLY (ephemeral)' `
      -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddDays(2)
    $cleanupThumb = $cert.Thumbprint
  }

  foreach ($p in $Path) {
    if (-not (Test-Path $p)) { throw "artefato não encontrado: $p" }
    $item = Get-Item $p
    if ($mode -eq 'none') {
      $dst = Join-Path $item.DirectoryName ($item.BaseName + '-UNSIGNED-DEV-TEST-ONLY' + $item.Extension)
      Copy-Item $item.FullName $dst -Force
      $results += [ordered]@{ file = $item.Name; output = (Split-Path $dst -Leaf); status = 'external_pending'; note = 'sem certificado: artefato de desenvolvimento/teste, não distribuir' }
      continue
    }
    if ($mode -eq 'real') {
      & $signtool sign /fd SHA256 /f $tmpPfx /p $env:CAPIA_SIGN_PFX_PASSWORD /tr $ts /td SHA256 $item.FullName | Out-Null
    } else {
      & $signtool sign /fd SHA256 /sha1 $cleanupThumb $item.FullName | Out-Null
    }
    if ($LASTEXITCODE -ne 0) { throw "signtool falhou para $($item.Name) (código $LASTEXITCODE)" }
    $results += [ordered]@{ file = $item.Name; output = $item.Name; status = $(if ($mode -eq 'real') { 'signed' } else { 'test_signed' }); note = $(if ($mode -eq 'real') { 'Authenticode com certificado de CI' } else { 'certificado autoassinado descartável: NÃO confiável' }) }
  }
}
finally {
  if ($tmpPfx -and (Test-Path $tmpPfx)) { Remove-Item $tmpPfx -Force }
  if ($cleanupThumb) { Remove-Item "Cert:\CurrentUser\My\$cleanupThumb" -Force -ErrorAction SilentlyContinue }
}

$report = [ordered]@{ mode = $mode; artifacts = $results }
$report | ConvertTo-Json -Depth 5 | Set-Content -Path $ReportPath -Encoding UTF8
Write-Host "signing: modo=$mode, $($results.Count) artefato(s); relatório em $ReportPath"
