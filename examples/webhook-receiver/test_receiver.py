"""Testes do receptor Python. Vetores calculados de forma independente (Node nao foi usado) sobre
a string exata "<timestamp>.<corpo>". Execucao: python3 -m unittest discover -s examples/webhook-receiver"""

import json
import threading
import unittest
import urllib.error
import urllib.request
from http.server import HTTPServer

import receiver as r

SECRET = "whsec_test_secret_0123456789"
BODY1 = (
    '{"id":"evt_0001","type":"webhook.test","version":1,"occurred_at":"2026-01-01T00:00:00Z",'
    '"occurred_ms":1767225600000,"project_id":null,"run_id":null,"export_id":null,"data":{"hello":"world"}}'
).encode()
TS1 = "1767225600"
SIG1 = "dc8fcc87c204cde41da6d8ff882cda7090123cde14ff9b6abb58cb2edf71b656"
BODY2 = (
    '{"id":"evt_0002","type":"run.completed","version":1,"occurred_at":"2026-01-01T00:00:01Z",'
    '"occurred_ms":1767225601000,"project_id":"prj_1","run_id":"run_1","export_id":null,'
    '"data":{"note":"ação concluída — vídeo ✓"}}'
).encode()
TS2 = "1767225601"
SIG2 = "b84b3d4249aec9b532784742201456e62b2157dbf60d21a27381a061f34c462c"
OTHER_SIG1 = "dc1a3b72a9aac6aad8667479b47e85fb8e0ff82471a9cfb56f0e4b7c3749bdad"
NOW = 1767225600000 + 1000


def hdr(ts, sig, eid):
    return {"x-capia-timestamp": ts, "x-capia-signature": f"v1={sig}", "x-capia-event-id": eid}


class VerifyTests(unittest.TestCase):
    def test_independent_vectors(self):
        self.assertEqual(r.compute_signature(SECRET, TS1, BODY1), SIG1)
        self.assertEqual(r.compute_signature(SECRET, TS2, BODY2), SIG2)
        self.assertEqual(r.compute_signature("another_secret", TS1, BODY1), OTHER_SIG1)

    def test_valid(self):
        self.assertEqual(r.verify_signature(SECRET, TS1, f"v1={SIG1}", BODY1, NOW), (True, None))

    def test_raw_bytes_matter(self):
        pretty = json.dumps(json.loads(BODY1), indent=1).encode()
        self.assertEqual(r.verify_signature(SECRET, TS1, f"v1={SIG1}", pretty, NOW)[1], "bad_signature")

    def test_tamper_wrong_secret_wrong_timestamp(self):
        self.assertEqual(r.verify_signature(SECRET, TS1, f"v1={SIG1}", BODY1 + b" ", NOW)[1], "bad_signature")
        self.assertEqual(r.verify_signature("outro", TS1, f"v1={SIG1}", BODY1, NOW)[1], "bad_signature")
        self.assertEqual(r.verify_signature(SECRET, TS2, f"v1={SIG1}", BODY1, NOW)[1], "bad_signature")
        self.assertEqual(r.verify_signature(SECRET, TS1, f"v1={OTHER_SIG1}", BODY1, NOW)[1], "bad_signature")

    def test_replay_window(self):
        t0 = 1767225600000
        self.assertTrue(r.verify_signature(SECRET, TS1, f"v1={SIG1}", BODY1, t0 + 300_000)[0])
        self.assertEqual(r.verify_signature(SECRET, TS1, f"v1={SIG1}", BODY1, t0 + 300_001)[1], "stale_timestamp")
        self.assertEqual(r.verify_signature(SECRET, TS1, f"v1={SIG1}", BODY1, t0 - 300_001)[1], "stale_timestamp")

    def test_malformed_inputs(self):
        self.assertEqual(r.verify_signature(SECRET, None, "v1=aa", BODY1, NOW)[1], "missing_headers")
        self.assertEqual(r.verify_signature(SECRET, TS1, None, BODY1, NOW)[1], "missing_headers")
        for bad in ("abc", "-1", "1.5", "99999999999999999999"):
            self.assertEqual(r.verify_signature(SECRET, bad, f"v1={SIG1}", BODY1, NOW)[1], "bad_timestamp", bad)
        for bad in (SIG1, "v2=" + SIG1, "v1=zz", f"v1={SIG1[2:]}", f"v1={SIG1.upper()}"):
            self.assertEqual(r.verify_signature(SECRET, TS1, bad, BODY1, NOW)[1], "malformed_signature", bad)
        with self.assertRaises(ValueError):
            r.verify_signature("", TS1, "x", BODY1)

    def test_milliseconds_timestamp(self):
        sig = r.compute_signature(SECRET, "1767225600000", BODY1)
        self.assertTrue(r.verify_signature(SECRET, "1767225600000", f"v1={sig}", BODY1, NOW)[0])

    def test_rotation_multiple_candidates(self):
        self.assertTrue(r.verify_signature(SECRET, TS1, f"v1={OTHER_SIG1}, v1={SIG1}", BODY1, NOW)[0])


class DeliveryTests(unittest.TestCase):
    def test_dedupe_bounded(self):
        d = r.Dedupe(2)
        self.assertFalse(d.seen_before("a"))
        self.assertTrue(d.seen_before("a"))
        d.seen_before("b")
        d.seen_before("c")
        self.assertEqual(len(d), 2)
        self.assertFalse(d.seen_before("a"))

    def test_process_once_then_duplicate(self):
        d, seen = r.Dedupe(), []
        args = (SECRET, hdr(TS1, SIG1, "evt_0001"), BODY1, d, NOW, lambda e, h: seen.append(e["id"]))
        self.assertEqual(r.handle_delivery(*args), (200, {"ok": True, "duplicate": False}))
        self.assertEqual(r.handle_delivery(*args), (200, {"ok": True, "duplicate": True}))
        self.assertEqual(seen, ["evt_0001"])

    def test_forged_delivery_does_not_poison_dedupe(self):
        d = r.Dedupe()
        status, _ = r.handle_delivery(SECRET, hdr(TS1, OTHER_SIG1, "evt_0001"), BODY1, d, NOW)
        self.assertEqual(status, 401)
        self.assertEqual(len(d), 0)
        self.assertFalse(r.handle_delivery(SECRET, hdr(TS1, SIG1, "evt_0001"), BODY1, d, NOW)[1]["duplicate"])

    def test_stale_is_401_unreadable_is_400(self):
        d = r.Dedupe()
        self.assertEqual(r.handle_delivery(SECRET, hdr(TS1, SIG1, "e"), BODY1, d, NOW + 400_000)[0], 401)
        self.assertEqual(r.handle_delivery(SECRET, hdr("xx", SIG1, "e"), BODY1, d, NOW)[0], 400)


class HttpTests(unittest.TestCase):
    def test_real_http_server(self):
        events = []
        handler = r.make_handler(SECRET, r.Dedupe(), lambda e, h: events.append(e["id"]), lambda: NOW)
        srv = HTTPServer(("127.0.0.1", 0), handler)
        t = threading.Thread(target=srv.serve_forever, daemon=True)
        t.start()
        url = f"http://127.0.0.1:{srv.server_address[1]}/"

        def post(body):
            req = urllib.request.Request(
                url,
                data=body,
                method="POST",
                headers={"X-CapIA-Timestamp": TS2, "X-CapIA-Signature": f"v1={SIG2}", "X-CapIA-Event-Id": "evt_0002"},
            )
            try:
                with urllib.request.urlopen(req) as resp:
                    return resp.status, json.load(resp)
            except urllib.error.HTTPError as e:
                return e.code, json.load(e)

        try:
            self.assertEqual(post(BODY2), (200, {"ok": True, "duplicate": False}))
            self.assertEqual(post(BODY2), (200, {"ok": True, "duplicate": True}))
            self.assertEqual(post(BODY2 + b" ")[0], 401)
            self.assertEqual(events, ["evt_0002"])
        finally:
            srv.shutdown()
            srv.server_close()


if __name__ == "__main__":
    unittest.main()
