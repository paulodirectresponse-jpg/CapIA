"""Testa a logica do cliente Python contra um servidor falso em memoria (NAO e o capia-server).
Execucao: python3 -m unittest discover -s examples/rest"""

import json
import os
import tempfile
import threading
import types
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

import client as cl


def make_fake(decision_kind="plan_approval"):
    state = {"approved": False, "polls": 0, "log": []}

    class H(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def _send(self, status, obj):
            data = json.dumps(obj).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def _handle(self):
            n = int(self.headers.get("Content-Length") or 0)
            body = json.loads(self.rfile.read(n)) if n else None
            state["log"].append((self.command, self.path, dict(self.headers), body))
            if self.headers.get("Authorization") != "Bearer capia_REDACTED":
                return self._send(401, {"code": "UNAUTHORIZED", "message": "no", "request_id": "r1"})
            p, m = self.path.split("?")[0], self.command
            if m == "POST" and p == "/v1/projects":
                return self._send(201, {"project": {"id": "prj_1"}})
            if p == "/v1/uploads/inline":
                return self._send(201, {"upload": {"id": f"upl_{len(state['log'])}"}})
            if p == "/v1/projects/prj_1/assets":
                return self._send(202, {"ticket": {"id": "tkt_" + body["upload_id"]}})
            if p.startswith("/v1/projects/prj_1/imports/"):
                return self._send(200, {"ticket": {"state": "completed", "asset_id": "ast_" + p.split("_")[-1]}})
            if m == "POST" and p == "/v1/projects/prj_1/runs":
                return self._send(202, {"run": {"id": "run_1", "status": "running"}})
            if m == "GET" and p == "/v1/projects/prj_1/runs/run_1":
                state["polls"] += 1
                if state["approved"]:
                    return self._send(200, {"run": {"id": "run_1", "status": "completed", "sequences": ["seq_1"]}})
                if state["polls"] >= 2:
                    pending = {
                        "id": "dec_1",
                        "kind": decision_kind,
                        "question": "Aprovar?",
                        "options": [{"id": "approve"}, {"id": "reject"}],
                    }
                    return self._send(200, {"run": {"id": "run_1", "status": "waiting_user", "pending": pending}})
                return self._send(200, {"run": {"id": "run_1", "status": "running"}})
            if p == "/v1/projects/prj_1/runs/run_1/approvals":
                state["approved"] = True
                return self._send(200, {"ok": True})
            if p == "/v1/projects/prj_1/exports" and m == "POST":
                return self._send(202, {"export": {"id": "exp_1", "state": "queued"}})
            if p == "/v1/projects/prj_1/exports/exp_1":
                return self._send(200, {"export": {"id": "exp_1", "state": "completed"}})
            return self._send(404, {"code": "NOT_FOUND", "message": p, "request_id": "r2"})

        do_GET = do_POST = _handle

    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, state


def files():
    d = tempfile.mkdtemp()
    raw, ref = os.path.join(d, "raw.mp4"), os.path.join(d, "ref.mp4")
    for p in (raw, ref):
        with open(p, "wb") as f:
            f.write(b"x")
    return raw, ref


def args(raw, ref, approve="", variants=0):
    return types.SimpleNamespace(
        raw=raw, reference=ref, brief="briefing", approve=approve, variants=variants, webhook_url=None
    )


class ClientTests(unittest.TestCase):
    def setUp(self):
        self._orig_sleep = cl.time.sleep
        cl.time.sleep = lambda *_: None

    def tearDown(self):
        cl.time.sleep = self._orig_sleep

    def test_error_envelope(self):
        srv, _ = make_fake()
        try:
            c = cl.Capia(srv.server_address[1], "errado")
            with self.assertRaises(cl.ApiError) as cm:
                c.get("/v1/projects")
            self.assertEqual((cm.exception.status, cm.exception.body["request_id"]), (401, "r1"))
        finally:
            srv.shutdown()
            srv.server_close()

    def test_stops_at_decision_without_approve(self):
        srv, state = make_fake()
        try:
            c = cl.Capia(srv.server_address[1], "capia_REDACTED")
            out = cl.run_flow(c, args(*files()), log=lambda *_: None)
            self.assertEqual(out["status"], "waiting_user")
            self.assertFalse(any(e[1].endswith("/approvals") for e in state["log"]))
        finally:
            srv.shutdown()
            srv.server_close()

    def test_full_flow_with_authorised_approval(self):
        srv, state = make_fake()
        try:
            c = cl.Capia(srv.server_address[1], "capia_REDACTED")
            out = cl.run_flow(c, args(*files(), approve="plan_approval"), log=lambda *_: None)
            self.assertEqual((out["status"], out["export_id"]), ("completed", "exp_1"))
            run = next(e for e in state["log"] if e[0] == "POST" and e[1].endswith("/runs"))
            self.assertTrue(run[3]["start"])
            self.assertEqual(len(run[3]["assets"]), 1)
            self.assertIn("Idempotency-Key", run[2])
            get = next(e for e in state["log"] if e[0] == "GET")
            self.assertNotIn("Idempotency-Key", get[2])
        finally:
            srv.shutdown()
            srv.server_close()

    def test_other_decision_kinds_are_not_auto_approved(self):
        srv, state = make_fake("spend_approval")
        try:
            c = cl.Capia(srv.server_address[1], "capia_REDACTED")
            out = cl.run_flow(c, args(*files(), approve="plan_approval"), log=lambda *_: None)
            self.assertEqual(out["status"], "waiting_user")
            self.assertFalse(any(e[1].endswith("/approvals") for e in state["log"]))
        finally:
            srv.shutdown()
            srv.server_close()

    def test_extract_id_and_poll(self):
        self.assertEqual(cl.extract_id({"run": {"id": "a"}}, "run"), "a")
        self.assertEqual(cl.extract_id({"id": "b"}, "run"), "b")
        with self.assertRaises(RuntimeError):
            cl.extract_id({}, "run")
        with self.assertRaises(TimeoutError):
            cl.poll(lambda: 0, lambda v: False, timeout=-1, sleep=lambda *_: None)


if __name__ == "__main__":
    unittest.main()
