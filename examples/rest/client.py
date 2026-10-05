#!/usr/bin/env python3
"""EXEMPLO (nao e codigo de produto): fluxo canonico do CapIA por REST em Python 3.9+ (somente biblioteca padrao).
Ver docs/api/canonical-flow.md.

    CAPIA_PORT=8731 CAPIA_TOKEN=capia_REDACTED python3 examples/rest/client.py \\
        --raw raw.mp4 --reference reference.mp4 --brief "Produto: ... CTA: ..." [--approve plan_approval] [--variants 3]

Forma das respostas: o servidor repassa o resultado dos servicos da Engine API/IA (ex.: {"run": {"id", "status",
"pending", "sequences"}}). `extract_id` aceita {"<tipo>": {"id"}} ou {"id"}. Ajuste se a sua versao diferir.
Por seguranca, decisoes (gasto, geracao, licenca) so sao aprovadas se o tipo estiver em --approve.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

TERMINAL_RUN = {"completed", "failed", "cancelled"}


class ApiError(Exception):
    def __init__(self, status: int, body):
        self.status, self.body = status, body or {}
        super().__init__(f"HTTP {status} {self.body.get('code', '')}: {self.body.get('message', '')}")


class Capia:
    def __init__(self, port, token):
        if not port or not token:
            raise SystemExit("defina CAPIA_PORT e CAPIA_TOKEN")
        self.base, self.token = f"http://127.0.0.1:{port}", token

    def request(self, method, path, body=None, query=None, idempotency_key=None):
        url = self.base + path + (("?" + urllib.parse.urlencode(query)) if query else "")
        headers = {"Authorization": f"Bearer {self.token}", "Accept": "application/json"}
        data = None
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        if method not in ("GET", "DELETE"):
            headers["Idempotency-Key"] = idempotency_key or str(uuid.uuid4())
        req = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req) as resp:
                text = resp.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            text = e.read().decode("utf-8")
            raise ApiError(e.code, json.loads(text) if text else None) from None
        return json.loads(text) if text else None

    def get(self, path, query=None):
        return self.request("GET", path, query=query)

    def post(self, path, body=None, idempotency_key=None):
        return self.request("POST", path, body=body, idempotency_key=idempotency_key)


def extract_id(res, kind):
    value = (res.get(kind) or {}).get("id") if isinstance(res.get(kind), dict) else None
    value = value or res.get("id") or res.get(f"{kind}_id")
    if not value:
        raise RuntimeError(f"resposta sem id de {kind}: {res}")
    return value


def poll(fn, done, interval=1.0, timeout=600.0, sleep=None):
    t0 = time.time()
    while True:
        v = fn()
        if done(v):
            return v
        if time.time() - t0 > timeout:
            raise TimeoutError("tempo esgotado esperando a operacao")
        (sleep or time.sleep)(interval)


def import_file(c, project_id, path):
    with open(path, "rb") as f:
        b64 = base64.b64encode(f.read()).decode("ascii")
    up = c.post("/v1/uploads/inline", {"filename": os.path.basename(path), "content_base64": b64})
    started = c.post(f"/v1/projects/{project_id}/assets", {"upload_id": extract_id(up, "upload")})
    ticket_id = extract_id(started, "ticket")

    def get_ticket():
        r = c.get(f"/v1/projects/{project_id}/imports/{ticket_id}")
        return r.get("ticket", r)

    tk = poll(get_ticket, lambda t: t.get("state") in ("completed", "failed", "cancelled"), interval=0.5)
    if tk.get("state") != "completed":
        raise RuntimeError(f"importacao falhou: {tk}")
    return tk.get("asset_id") or (tk.get("asset") or {}).get("id")


def run_flow(c, args, log=print):
    if args.webhook_url:
        wh = c.post(
            "/v1/webhooks",
            {
                "url": args.webhook_url,
                "events": ["run.completed", "run.failed", "run.waiting_user", "export.completed", "export.failed"],
            },
        )
        log(f"webhook {extract_id(wh, 'webhook')} criado (o segredo foi mostrado so agora; guarde-o)")
    pid = extract_id(c.post("/v1/projects", {"name": "Fluxo canonico"}), "project")
    log(f"1. projeto {pid}")
    raw_id, ref_id = import_file(c, pid, args.raw), import_file(c, pid, args.reference)
    log(f"2-3. assets {raw_id} (bruto) e {ref_id} (referencia)")
    created = c.post(
        f"/v1/projects/{pid}/runs",
        {"brief_text": args.brief, "assets": [raw_id], "references": [ref_id], "start": True},
        idempotency_key=str(uuid.uuid4()),
    )
    run_id = extract_id(created, "run")
    log(f"4-5. Run {run_id} iniciada")
    approve = set(filter(None, (args.approve or "").split(",")))
    while True:
        run = poll(
            lambda: c.get(f"/v1/projects/{pid}/runs/{run_id}")["run"],
            lambda r: r["status"] in TERMINAL_RUN or r["status"] == "waiting_user",
        )
        if run["status"] != "waiting_user":
            break
        d = run["pending"]
        log(f"7. decisao {d['id']} ({d['kind']}): {d.get('question', '')}")
        if d["kind"] not in approve or not any(o["id"] == "approve" for o in d["options"]):
            log(f"   exige um humano (use --approve {d['kind']} so se for seguro). Parando aqui.")
            return {"status": "waiting_user", "project_id": pid, "run_id": run_id}
        c.post(f"/v1/projects/{pid}/runs/{run_id}/approvals", {"decision_id": d["id"], "option": "approve"})
    log(f"8. Run {run['status']}")
    if run["status"] != "completed":
        return {"status": run["status"], "project_id": pid, "run_id": run_id}
    if args.variants:
        c.post(f"/v1/projects/{pid}/runs/{run_id}/variants", {"count": args.variants})
        log(f"9. {args.variants} variantes pedidas (Runs filhas; acompanhe por GET /runs)")
    seqs = run.get("sequences") or [s["id"] for s in c.get(f"/v1/projects/{pid}/sequences").get("sequences", [])]
    if not seqs:
        raise RuntimeError("a Run nao produziu nenhuma sequence para exportar")
    first = seqs[0]["id"] if isinstance(seqs[0], dict) else seqs[0]
    exp = c.post(f"/v1/projects/{pid}/exports", {"items": [{"sequence": first, "preset": "h264-mp4"}]})
    exp_id = extract_id(exp, "export")
    done = poll(
        lambda: c.get(f"/v1/projects/{pid}/exports/{exp_id}"),
        lambda e: e.get("export", e).get("state") in ("completed", "failed", "cancelled"),
    )
    log(f"10. export {exp_id}: {done.get('export', done).get('state')}")
    log("11-12. o webhook chega ao receptor local; confira o resultado no CapIA (abra o projeto) ou em /summary")
    return {"status": "completed", "project_id": pid, "run_id": run_id, "export_id": exp_id}


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--raw", required=True)
    ap.add_argument("--reference", required=True)
    ap.add_argument("--brief", required=True)
    ap.add_argument("--approve", default="", help="tipos de decisao aprovados sozinhos, separados por virgula")
    ap.add_argument("--variants", type=int, default=0)
    ap.add_argument("--webhook-url")
    args = ap.parse_args()
    try:
        result = run_flow(Capia(os.environ.get("CAPIA_PORT"), os.environ.get("CAPIA_TOKEN")), args)
    except ApiError as e:
        print(f"{e} (request_id={e.body.get('request_id')})", file=sys.stderr)
        sys.exit(1)
    print(json.dumps(result, indent=2))
    sys.exit(0 if result["status"] == "completed" else 3)


if __name__ == "__main__":
    main()
