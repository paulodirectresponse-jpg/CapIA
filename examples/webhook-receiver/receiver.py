#!/usr/bin/env python3
"""EXEMPLO (nao e codigo de produto): receptor local de webhooks do CapIA que VERIFICA a assinatura.

    CAPIA_WEBHOOK_SECRET=<segredo mostrado uma unica vez> python3 examples/webhook-receiver/receiver.py [--port 9000]

Contrato (docs/api/webhooks.md): `X-CapIA-Signature: v1=<hex HMAC-SHA256(segredo, "<timestamp>.<corpo bruto>")>`,
`X-CapIA-Timestamp` (Unix em segundos; valores > 1e11 sao lidos como milissegundos), `X-CapIA-Event-Id`.
HMAC sobre os BYTES brutos, comparacao em tempo constante, janela de 5 minutos, de-duplicacao por event id
(somente apos a assinatura ser valida) e corpo limitado. Escuta so em 127.0.0.1. Somente biblioteca padrao.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import os
import re
import sys
import time
from collections import OrderedDict
from http.server import BaseHTTPRequestHandler, HTTPServer

TOLERANCE_MS = 5 * 60 * 1000
MAX_BODY_BYTES = 1024 * 1024
_HEX64 = re.compile(r"^[0-9a-f]{64}$")


def compute_signature(secret: str, timestamp: str, raw_body: bytes) -> str:
    msg = timestamp.encode("utf-8") + b"." + raw_body
    return hmac.new(secret.encode("utf-8"), msg, hashlib.sha256).hexdigest()


def parse_timestamp_ms(value):
    if not isinstance(value, str) or not re.fullmatch(r"\d{1,16}", value):
        return None
    n = int(value)
    return n if n > 10**11 else n * 1000


def verify_signature(secret, timestamp, signature, raw_body, now_ms=None, tolerance_ms=TOLERANCE_MS):
    """Devolve (ok, reason). reason: missing_headers | bad_timestamp | stale_timestamp |
    malformed_signature | bad_signature."""
    if not secret:
        raise ValueError("secret e obrigatorio")
    if not timestamp or not signature:
        return False, "missing_headers"
    ts_ms = parse_timestamp_ms(timestamp)
    if ts_ms is None:
        return False, "bad_timestamp"
    now = int(time.time() * 1000) if now_ms is None else now_ms
    if abs(now - ts_ms) > tolerance_ms:
        return False, "stale_timestamp"
    candidates = [p.strip()[3:] for p in signature.split(",") if p.strip().startswith("v1=")]
    if not candidates or any(not _HEX64.match(c) for c in candidates):
        return False, "malformed_signature"
    expected = compute_signature(secret, timestamp, raw_body)
    match = False
    for c in candidates:  # sem curto-circuito entre candidatas (rotacao de segredo)
        if hmac.compare_digest(c, expected):
            match = True
    return (True, None) if match else (False, "bad_signature")


class Dedupe:
    def __init__(self, max_entries: int = 10_000):
        self._seen: OrderedDict[str, None] = OrderedDict()
        self._max = max_entries

    def seen_before(self, event_id: str) -> bool:
        if event_id in self._seen:
            return True
        self._seen[event_id] = None
        if len(self._seen) > self._max:
            self._seen.popitem(last=False)
        return False

    def __len__(self) -> int:
        return len(self._seen)


def handle_delivery(secret, headers, raw_body, dedupe, now_ms=None, on_event=None):
    """headers: dict com chaves em minusculas. Devolve (status, corpo-dict)."""
    ok, reason = verify_signature(
        secret, headers.get("x-capia-timestamp"), headers.get("x-capia-signature"), raw_body, now_ms
    )
    if not ok:
        return (401 if reason in ("bad_signature", "stale_timestamp") else 400), {"error": reason}
    try:
        event = json.loads(raw_body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return 400, {"error": "bad_json"}
    event_id = headers.get("x-capia-event-id") or (event.get("id") if isinstance(event, dict) else None)
    if not isinstance(event_id, str) or not event_id:
        return 400, {"error": "no_event_id"}
    if dedupe.seen_before(event_id):
        return 200, {"ok": True, "duplicate": True}
    if on_event:
        on_event(event, headers)
    return 200, {"ok": True, "duplicate": False}


def make_handler(secret, dedupe, on_event=None, now_fn=None):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):  # silencioso: o exemplo imprime so os eventos
            pass

        def _reply(self, status, body):
            data = json.dumps(body).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self):
            length = int(self.headers.get("Content-Length") or 0)
            if length > MAX_BODY_BYTES:
                self._reply(413, {"error": "too_large"})
                return
            raw = self.rfile.read(length)
            headers = {k.lower(): v for k, v in self.headers.items()}
            now = now_fn() if now_fn else None
            status, body = handle_delivery(secret, headers, raw, dedupe, now, on_event)
            self._reply(status, body)

        def do_GET(self):
            self._reply(405, {"error": "method_not_allowed"})

    return Handler


def main():
    secret = os.environ.get("CAPIA_WEBHOOK_SECRET")
    if not secret:
        print("defina CAPIA_WEBHOOK_SECRET (o segredo e mostrado uma unica vez na criacao)", file=sys.stderr)
        sys.exit(2)
    port = int(sys.argv[sys.argv.index("--port") + 1]) if "--port" in sys.argv else 9000

    def on_event(event, headers):
        print(
            f"evento {event.get('type')} id={event.get('id')} projeto={event.get('project_id') or '-'} "
            f"run={event.get('run_id') or '-'} tentativa={headers.get('x-capia-attempt', '?')}"
        )

    server = HTTPServer(("127.0.0.1", port), make_handler(secret, Dedupe(), on_event))
    print(f"receptor em http://127.0.0.1:{port}/ (assinatura verificada, janela de 5 min)")
    server.serve_forever()


if __name__ == "__main__":
    main()
