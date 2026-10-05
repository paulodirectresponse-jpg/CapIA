<#
.SYNOPSIS
  Verifica a assinatura Authenticode de artefatos e relata o estado real.

.DESCRIPTION
  Estados por arquivo:
    valid             assinatura válida e confiável (release real)
    test_signed       assinada por certificado autoassinado (pipeline provado; NÃO confiável)
    external_pending  sem assinatura (artefato dev/test; o certificado real é um gate EXTERNO)
    invalid           assinatura presente mas inválida/adulterada
  -Require valid|test|any : o que torna o script falhar (exit 1).
    valid  -> todos precisam ser `valid` (release.yml)
    test   -> todos precisam ser `valid` ou `test_signed` (CI com certificado descartável)
    any    -> só `invalid` falha (CI sem segredos: reporta external_pending, sem fingir)
#>
param(
  [Parameter(Mandatory = $true)][string[]]$Path,
  [ValidateSet('valid', 'test', 'any')][string]$Require = 'any',
  [string]$ReportPath = 'signature-verification.json'
)
$ErrorActionPreference = 'Stop'
$rows = @()
$failed = $false
foreach ($p in $Path) {
  $sig = Get-AuthenticodeSignature -FilePath $p
  switch ($sig.Status.ToString()) {
    'Valid' { $state = 'valid' }
    'NotSigned' { $state = 'external_pending' }
    'UnknownError' { $state = if ($sig.SignerCertificate -and $sig.SignerCertificate.Subject -eq $sig.SignerCertificate.Issuer) { 'test_signed' } else { 'invalid' } }
    'UntrustedRoot' { $state = if ($sig.SignerCertificate -and $sig.SignerCertificate.Subject -eq $sig.SignerCertificate.Issuer) { 'test_signed' } else { 'invalid' } }
    default { $state = 'invalid' }
  }
  if ($state -eq 'invalid') { $failed = $true }
  if ($Require -eq 'valid' -and $state -ne 'valid') { $failed = $true }
  if ($Require -eq 'test' -and $state -notin @('valid', 'test_signed')) { $failed = $true }
  $rows += [ordered]@{
    file = (Split-Path $p -Leaf)
    signing = $state
    authenticode_status = $sig.Status.ToString()
    signer = $(if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { $null })
    timestamped = [bool]$sig.TimeStamperCertificate
  }
}
$overall = if ($failed) { 'failed' } elseif (($rows | Where-Object { $_.signing -eq 'external_pending' }).Count -gt 0) { 'external_pending' } elseif (($rows | Where-Object { $_.signing -eq 'test_signed' }).Count -gt 0) { 'test_signed' } else { 'valid' }
$report = [ordered]@{ require = $Require; overall = $overall; files = $rows }
$report | ConvertTo-Json -Depth 5 | Set-Content -Path $ReportPath -Encoding UTF8
$rows | ForEach-Object { Write-Host ("{0}: {1} ({2})" -f $_.file, $_.signing, $_.authenticode_status) }
Write-Host "signing: $overall"
if ($failed) { exit 1 }
