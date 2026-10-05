#!/usr/bin/env bash
# EXEMPLO (não é código de produto): fluxo canônico do CapIA por REST com curl + jq.
# Ver docs/api/canonical-flow.md. Requer: bash, curl, jq, base64, uuidgen (ou /proc/sys/kernel/random/uuid).
#
#   export CAPIA_PORT=8731
#   export CAPIA_TOKEN=capia_REDACTED          # nunca versione o token real
#   examples/rest/curl.sh raw.mp4 reference.mp4 "Produto: ... Público: ... Oferta: ... CTA: ..."
#
# Variáveis opcionais: APPROVE_KINDS="plan_approval" (tipos de decisão que o script aprova sozinho;
# padrão: nenhum), WEBHOOK_URL="http://127.0.0.1:9000/" (registra um webhook; o segredo é mostrado UMA vez).
#
# Forma das respostas: o servidor repassa o resultado dos serviços da Engine API/IA. Os caminhos jq abaixo
# (`.project.id`, `.run.status`, ...) seguem esse formato; ajuste se a sua versão do servidor diferir.
set -euo pipefail

: "${CAPIA_PORT:?defina CAPIA_PORT}" "${CAPIA_TOKEN:?defina CAPIA_TOKEN}"
RAW="${1:?uso: curl.sh <raw> <referência> <briefing>}"
REF="${2:?uso: curl.sh <raw> <referência> <briefing>}"
BRIEF="${3:?uso: curl.sh <raw> <referência> <briefing>}"
BASE="http://127.0.0.1:${CAPIA_PORT}"
APPROVE_KINDS="${APPROVE_KINDS:-}"

command -v jq >/dev/null || { echo "jq é necessário" >&2; exit 2; }

uuid() { uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid; }

# api METHOD PATH [JSON]  → imprime o corpo; em erro HTTP imprime o envelope e sai com 1.
api() {
  local method="$1" path="$2" body="${3:-}" out status
  local args=(-sS -X "$method" "$BASE$path" -H "Authorization: Bearer $CAPIA_TOKEN" -w $'\n%{http_code}')
  if [[ "$method" != "GET" && "$method" != "DELETE" ]]; then
    args+=(-H "Idempotency-Key: ${IDEM_KEY:-$(uuid)}")
  fi
  if [[ -n "$body" ]]; then args+=(-H "Content-Type: application/json" -d "$body"); fi
  out="$(curl "${args[@]}")"
  status="${out##*$'\n'}"
  out="${out%$'\n'*}"
  if [[ "$status" -ge 400 ]]; then
    echo "HTTP $status: $out" >&2
    exit 1
  fi
  printf '%s' "$out"
}

wait_for() { # wait_for "<comando que imprime o estado>" "<estados terminais separados por |>"
  local state
  while :; do
    state="$(eval "$1")"
    if [[ "|$2|" == *"|$state|"* ]]; then printf '%s' "$state"; return; fi
    sleep 1
  done
}

import_file() { # import_file <arquivo> → imprime o asset_id
  local f="$1" up ticket
  up="$(api POST /v1/uploads/inline "$(jq -n --arg n "$(basename "$f")" --arg b "$(base64 -w0 "$f" 2>/dev/null || base64 "$f" | tr -d '\n')" '{filename:$n, content_base64:$b}')" | jq -r '.upload.id // .id')"
  ticket="$(api POST "/v1/projects/$PID/assets" "$(jq -n --arg u "$up" '{upload_id:$u}')" | jq -r '.ticket.id // .id')"
  wait_for "api GET /v1/projects/$PID/imports/$ticket | jq -r '(.ticket // .).state'" "completed|failed|cancelled" >/dev/null
  api GET "/v1/projects/$PID/imports/$ticket" | jq -r '(.ticket // .) | .asset_id // .asset.id'
}

if [[ -n "${WEBHOOK_URL:-}" ]]; then
  echo "== webhook (o segredo aparece só nesta resposta; guarde-o em um cofre)"
  api POST /v1/webhooks "$(jq -n --arg u "$WEBHOOK_URL" '{url:$u, events:["run.completed","run.failed","run.waiting_user","export.completed","export.failed"]}')" | jq .
fi

echo "== 1. projeto"
PID="$(api POST /v1/projects '{"name":"Fluxo canônico"}' | jq -r '.project.id // .id')"
echo "projeto: $PID"

echo "== 2-3. bruto e referência"
RAW_ID="$(import_file "$RAW")"
REF_ID="$(import_file "$REF")"
echo "assets: $RAW_ID (bruto), $REF_ID (referência)"

echo "== 4-5. briefing + Run"
IDEM_KEY="$(uuid)"   # a MESMA chave se você repetir este POST após um timeout
RUN_BODY="$(jq -n --arg b "$BRIEF" --arg r "$RAW_ID" --arg f "$REF_ID" '{brief_text:$b, assets:[$r], references:[$f], start:true}')"
RUN_ID="$(api POST "/v1/projects/$PID/runs" "$RUN_BODY" | jq -r '.run.id // .id')"
unset IDEM_KEY
echo "run: $RUN_ID"

echo "== 6-8. acompanhar, aprovar, esperar"
while :; do
  STATE="$(wait_for "api GET /v1/projects/$PID/runs/$RUN_ID | jq -r '.run.status'" "completed|failed|cancelled|waiting_user")"
  [[ "$STATE" == "waiting_user" ]] || break
  DEC="$(api GET "/v1/projects/$PID/runs/$RUN_ID" | jq -c '.run.pending')"
  KIND="$(jq -r '.kind' <<<"$DEC")"
  echo "decisão $(jq -r '.id' <<<"$DEC") ($KIND): $(jq -r '.question' <<<"$DEC")"
  if [[ " $APPROVE_KINDS " != *" $KIND "* ]]; then
    echo "exige um humano: aprove pelo app ou rode de novo com APPROVE_KINDS=$KIND (só se for seguro)."
    exit 3
  fi
  api POST "/v1/projects/$PID/runs/$RUN_ID/approvals" "$(jq -n --arg d "$(jq -r '.id' <<<"$DEC")" '{decision_id:$d, option:"approve"}')" >/dev/null
done
echo "run terminou: $STATE"
[[ "$STATE" == "completed" ]] || exit 1

echo "== 9. variantes (opcional)"
api POST "/v1/projects/$PID/runs/$RUN_ID/variants" '{"count":3}' | jq -c .

echo "== 10. export"
SEQ="$(api GET "/v1/projects/$PID/runs/$RUN_ID" | jq -r '.run.sequences[0] | if type=="object" then .id else . end')"
EXP_ID="$(api POST "/v1/projects/$PID/exports" "$(jq -n --arg s "$SEQ" '{items:[{sequence:$s, preset:"h264-mp4"}]}')" | jq -r '.export.id // .id')"
wait_for "api GET /v1/projects/$PID/exports/$EXP_ID | jq -r '(.export // .).state'" "completed|failed|cancelled"
echo
api GET "/v1/projects/$PID/deliverables" | jq .

echo "== 11-12. o webhook chega ao receptor local; confira o resultado no CapIA"
api GET "/v1/projects/$PID/summary" | jq .
