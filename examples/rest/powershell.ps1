<#
EXEMPLO (não é código de produto): fluxo canônico do CapIA por REST em PowerShell 5.1+ / 7+.
Ver docs/api/canonical-flow.md.

  $env:CAPIA_PORT  = "8731"
  $env:CAPIA_TOKEN = "capia_REDACTED"     # nunca versione o token real
  .\examples\rest\powershell.ps1 -Raw raw.mp4 -Reference reference.mp4 -Brief "Produto: ... CTA: ..." [-ApproveKinds plan_approval] [-WebhookUrl http://127.0.0.1:9000/]

Forma das respostas: o servidor repassa o resultado dos serviços da Engine API/IA (ex.: run.id, run.status,
run.pending). Ajuste se a sua versão do servidor diferir. Só fala com 127.0.0.1.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$Raw,
  [Parameter(Mandatory)][string]$Reference,
  [Parameter(Mandatory)][string]$Brief,
  [string[]]$ApproveKinds = @(),     # tipos de decisão aprovados automaticamente (padrão: nenhum)
  [string]$WebhookUrl = ""
)
$ErrorActionPreference = "Stop"
if (-not $env:CAPIA_PORT -or -not $env:CAPIA_TOKEN) { throw "defina CAPIA_PORT e CAPIA_TOKEN" }
$Base = "http://127.0.0.1:$($env:CAPIA_PORT)"

function Invoke-Capia {
  param([string]$Method, [string]$Path, $Body = $null, [string]$IdempotencyKey = "")
  $headers = @{ Authorization = "Bearer $($env:CAPIA_TOKEN)"; Accept = "application/json" }
  if ($Method -ne "GET" -and $Method -ne "DELETE") {
    $headers["Idempotency-Key"] = $(if ($IdempotencyKey) { $IdempotencyKey } else { [guid]::NewGuid().ToString() })
  }
  $args = @{ Method = $Method; Uri = "$Base$Path"; Headers = $headers; ContentType = "application/json; charset=utf-8" }
  if ($null -ne $Body) { $args.Body = [System.Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 20 -Compress)) }
  try { Invoke-RestMethod @args }
  catch {
    # Envelope de erro: { code, message, details?, request_id }
    $detail = $_.ErrorDetails.Message
    throw "HTTP erro em $Method $Path : $detail"
  }
}

function Wait-Until([scriptblock]$Get, [string[]]$Terminal) {
  while ($true) {
    $state = & $Get
    if ($Terminal -contains $state) { return $state }
    Start-Sleep -Seconds 1
  }
}

function Import-CapiaFile([string]$ProjectId, [string]$File) {
  $b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Resolve-Path $File)))
  $up = Invoke-Capia POST "/v1/uploads/inline" @{ filename = (Split-Path $File -Leaf); content_base64 = $b64 }
  $uploadId = if ($up.upload) { $up.upload.id } else { $up.id }
  $t = Invoke-Capia POST "/v1/projects/$ProjectId/assets" @{ upload_id = $uploadId }
  $ticketId = if ($t.ticket) { $t.ticket.id } else { $t.id }
  $null = Wait-Until { $r = Invoke-Capia GET "/v1/projects/$ProjectId/imports/$ticketId"; $(if ($r.ticket) { $r.ticket.state } else { $r.state }) } @("completed", "failed", "cancelled")
  $r = Invoke-Capia GET "/v1/projects/$ProjectId/imports/$ticketId"
  $tk = if ($r.ticket) { $r.ticket } else { $r }
  if ($tk.asset_id) { $tk.asset_id } else { $tk.asset.id }
}

if ($WebhookUrl) {
  "== webhook (o segredo aparece só nesta resposta; guarde-o em um cofre)"
  Invoke-Capia POST "/v1/webhooks" @{ url = $WebhookUrl; events = @("run.completed", "run.failed", "run.waiting_user", "export.completed", "export.failed") } | ConvertTo-Json -Depth 5
}

"== 1. projeto"
$p = Invoke-Capia POST "/v1/projects" @{ name = "Fluxo canônico" }
$pid_ = if ($p.project) { $p.project.id } else { $p.id }
"projeto: $pid_"

"== 2-3. bruto e referência"
$rawId = Import-CapiaFile $pid_ $Raw
$refId = Import-CapiaFile $pid_ $Reference
"assets: $rawId (bruto), $refId (referência)"

"== 4-5. briefing + Run"
$runKey = [guid]::NewGuid().ToString()   # a MESMA chave se você repetir este POST após um timeout
$r = Invoke-Capia POST "/v1/projects/$pid_/runs" @{ brief_text = $Brief; assets = @($rawId); references = @($refId); start = $true } $runKey
$runId = if ($r.run) { $r.run.id } else { $r.id }
"run: $runId"

"== 6-8. acompanhar, aprovar, esperar"
while ($true) {
  $state = Wait-Until { (Invoke-Capia GET "/v1/projects/$pid_/runs/$runId").run.status } @("completed", "failed", "cancelled", "waiting_user")
  if ($state -ne "waiting_user") { break }
  $d = (Invoke-Capia GET "/v1/projects/$pid_/runs/$runId").run.pending
  "decisão $($d.id) ($($d.kind)): $($d.question)"
  if ($ApproveKinds -notcontains $d.kind) {
    "exige um humano: aprove pelo app ou rode de novo com -ApproveKinds $($d.kind) (só se for seguro)."
    exit 3
  }
  $null = Invoke-Capia POST "/v1/projects/$pid_/runs/$runId/approvals" @{ decision_id = $d.id; option = "approve" }
}
"run terminou: $state"
if ($state -ne "completed") { exit 1 }

"== 9. variantes (opcional)"
Invoke-Capia POST "/v1/projects/$pid_/runs/$runId/variants" @{ count = 3 } | ConvertTo-Json -Compress

"== 10. export"
$seq = (Invoke-Capia GET "/v1/projects/$pid_/runs/$runId").run.sequences[0]
if ($seq.id) { $seq = $seq.id }
$e = Invoke-Capia POST "/v1/projects/$pid_/exports" @{ items = @(@{ sequence = $seq; preset = "h264-mp4" }) }
$expId = if ($e.export) { $e.export.id } else { $e.id }
$null = Wait-Until { $x = Invoke-Capia GET "/v1/projects/$pid_/exports/$expId"; $(if ($x.export) { $x.export.state } else { $x.state }) } @("completed", "failed", "cancelled")
Invoke-Capia GET "/v1/projects/$pid_/deliverables" | ConvertTo-Json -Depth 8

"== 11-12. o webhook chega ao receptor local; confira o resultado no CapIA"
Invoke-Capia GET "/v1/projects/$pid_/summary" | ConvertTo-Json -Depth 8
