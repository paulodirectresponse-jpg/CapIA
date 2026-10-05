// Testes do receptor de exemplo. Os vetores abaixo foram calculados de forma INDEPENDENTE (Python
// `hmac`/`hashlib`, não este código) sobre a string exata "<timestamp>.<corpo>".
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  computeSignature,
  createDedupe,
  createReceiver,
  handleDelivery,
  parseTimestampMs,
  verifySignature,
} from "./receiver.mjs";

const SECRET = "whsec_test_secret_0123456789";
const BODY1 =
  '{"id":"evt_0001","type":"webhook.test","version":1,"occurred_at":"2026-01-01T00:00:00Z","occurred_ms":1767225600000,"project_id":null,"run_id":null,"export_id":null,"data":{"hello":"world"}}';
const TS1 = "1767225600";
const SIG1 = "dc8fcc87c204cde41da6d8ff882cda7090123cde14ff9b6abb58cb2edf71b656";
const BODY2 =
  '{"id":"evt_0002","type":"run.completed","version":1,"occurred_at":"2026-01-01T00:00:01Z","occurred_ms":1767225601000,"project_id":"prj_1","run_id":"run_1","export_id":null,"data":{"note":"ação concluída — vídeo ✓"}}';
const TS2 = "1767225601";
const SIG2 = "b84b3d4249aec9b532784742201456e62b2157dbf60d21a27381a061f34c462c";
const OTHER_SECRET_SIG1 = "dc1a3b72a9aac6aad8667479b47e85fb8e0ff82471a9cfb56f0e4b7c3749bdad";
const NOW = 1767225600000 + 1000; // 1 s depois do TS1

test("os vetores independentes batem com computeSignature (ASCII e UTF-8)", () => {
  assert.equal(computeSignature(SECRET, TS1, Buffer.from(BODY1)), SIG1);
  assert.equal(computeSignature(SECRET, TS2, Buffer.from(BODY2)), SIG2);
  assert.equal(computeSignature("another_secret", TS1, Buffer.from(BODY1)), OTHER_SECRET_SIG1);
});

test("assinatura válida é aceita", () => {
  const r = verifySignature({
    secret: SECRET,
    timestamp: TS1,
    signature: `v1=${SIG1}`,
    rawBody: Buffer.from(BODY1),
    nowMs: NOW,
  });
  assert.deepEqual(r, { ok: true });
});

test("o HMAC é sobre os bytes brutos: re-serializar o JSON invalida", () => {
  const reserialized = JSON.stringify(JSON.parse(BODY1), null, 1);
  const r = verifySignature({
    secret: SECRET,
    timestamp: TS1,
    signature: `v1=${SIG1}`,
    rawBody: Buffer.from(reserialized),
    nowMs: NOW,
  });
  assert.equal(r.reason, "bad_signature");
});

test("corpo adulterado, segredo errado e timestamp trocado são recusados", () => {
  const base = { secret: SECRET, timestamp: TS1, signature: `v1=${SIG1}`, nowMs: NOW };
  assert.equal(
    verifySignature({ ...base, rawBody: Buffer.from(`${BODY1} `) }).reason,
    "bad_signature",
  );
  assert.equal(
    verifySignature({ ...base, secret: "outro", rawBody: Buffer.from(BODY1) }).reason,
    "bad_signature",
  );
  assert.equal(
    verifySignature({ ...base, timestamp: "1767225601", rawBody: Buffer.from(BODY1) }).reason,
    "bad_signature",
  );
  assert.equal(
    verifySignature({
      ...base,
      signature: `v1=${OTHER_SECRET_SIG1}`,
      rawBody: Buffer.from(BODY1),
    }).reason,
    "bad_signature",
  );
});

test("janela de replay de 5 minutos (passado e futuro)", () => {
  const args = {
    secret: SECRET,
    timestamp: TS1,
    signature: `v1=${SIG1}`,
    rawBody: Buffer.from(BODY1),
  };
  const t0 = 1767225600000;
  assert.equal(verifySignature({ ...args, nowMs: t0 + 299_000 }).ok, true);
  assert.equal(verifySignature({ ...args, nowMs: t0 + 300_000 }).ok, true);
  assert.equal(verifySignature({ ...args, nowMs: t0 + 300_001 }).reason, "stale_timestamp");
  assert.equal(verifySignature({ ...args, nowMs: t0 - 300_001 }).reason, "stale_timestamp");
});

test("cabeçalhos ausentes, timestamp ruim e assinatura malformada", () => {
  const raw = Buffer.from(BODY1);
  assert.equal(
    verifySignature({ secret: SECRET, timestamp: undefined, signature: "v1=aa", rawBody: raw })
      .reason,
    "missing_headers",
  );
  assert.equal(
    verifySignature({ secret: SECRET, timestamp: TS1, signature: undefined, rawBody: raw }).reason,
    "missing_headers",
  );
  for (const bad of ["abc", "-1", "1.5", "", "99999999999999999999"])
    assert.equal(
      verifySignature({
        secret: SECRET,
        timestamp: bad,
        signature: `v1=${SIG1}`,
        rawBody: raw,
        nowMs: NOW,
      }).reason,
      bad === "" ? "missing_headers" : "bad_timestamp",
      bad,
    );
  for (const bad of [
    SIG1,
    "v2=" + SIG1,
    "v1=zz",
    `v1=${SIG1.slice(2)}`,
    `v1=${SIG1.toUpperCase()}`,
  ])
    assert.equal(
      verifySignature({ secret: SECRET, timestamp: TS1, signature: bad, rawBody: raw, nowMs: NOW })
        .reason,
      "malformed_signature",
      bad,
    );
  assert.throws(() => verifySignature({ secret: "", timestamp: TS1, signature: "", rawBody: raw }));
});

test("timestamp em milissegundos também é aceito", () => {
  assert.equal(parseTimestampMs("1767225600"), 1767225600000);
  assert.equal(parseTimestampMs("1767225600000"), 1767225600000);
  const sig = computeSignature(SECRET, "1767225600000", Buffer.from(BODY1));
  const r = verifySignature({
    secret: SECRET,
    timestamp: "1767225600000",
    signature: `v1=${sig}`,
    rawBody: Buffer.from(BODY1),
    nowMs: NOW,
  });
  assert.equal(r.ok, true);
});

test("rotação de segredo: várias v1= no cabeçalho, basta uma bater", () => {
  const r = verifySignature({
    secret: SECRET,
    timestamp: TS1,
    signature: `v1=${OTHER_SECRET_SIG1}, v1=${SIG1}`,
    rawBody: Buffer.from(BODY1),
    nowMs: NOW,
  });
  assert.equal(r.ok, true);
});

test("dedupe limitado por tamanho", () => {
  const d = createDedupe(2);
  assert.equal(d.seenBefore("a"), false);
  assert.equal(d.seenBefore("a"), true);
  d.seenBefore("b");
  d.seenBefore("c"); // expulsa "a"
  assert.equal(d.size(), 2);
  assert.equal(d.seenBefore("a"), false);
});

const headers = (ts, sig, id) => ({
  "x-capia-timestamp": ts,
  "x-capia-signature": `v1=${sig}`,
  "x-capia-event-id": id,
  "x-capia-attempt": "1",
});

test("entrega: processa uma vez; reentrega do mesmo evento é 200 duplicada", () => {
  const dedupe = createDedupe();
  const seen = [];
  const args = {
    secret: SECRET,
    headers: headers(TS1, SIG1, "evt_0001"),
    rawBody: Buffer.from(BODY1),
    dedupe,
    nowMs: NOW,
    onEvent: (e) => seen.push(e.id),
  };
  assert.deepEqual(handleDelivery(args), { status: 200, body: { ok: true, duplicate: false } });
  assert.deepEqual(handleDelivery(args), { status: 200, body: { ok: true, duplicate: true } });
  assert.deepEqual(seen, ["evt_0001"]);
});

test("entrega forjada nunca entra no dedupe (não dá para envenenar ids)", () => {
  const dedupe = createDedupe();
  const forged = handleDelivery({
    secret: SECRET,
    headers: headers(TS1, OTHER_SECRET_SIG1, "evt_0001"),
    rawBody: Buffer.from(BODY1),
    dedupe,
    nowMs: NOW,
  });
  assert.equal(forged.status, 401);
  assert.equal(dedupe.size(), 0);
  const real = handleDelivery({
    secret: SECRET,
    headers: headers(TS1, SIG1, "evt_0001"),
    rawBody: Buffer.from(BODY1),
    dedupe,
    nowMs: NOW,
  });
  assert.equal(real.body.duplicate, false);
});

test("replay fora da janela vira 401; timestamp ilegível vira 400", () => {
  const dedupe = createDedupe();
  const base = { secret: SECRET, rawBody: Buffer.from(BODY1), dedupe };
  assert.equal(
    handleDelivery({ ...base, headers: headers(TS1, SIG1, "e"), nowMs: NOW + 400_000 }).status,
    401,
  );
  assert.equal(
    handleDelivery({ ...base, headers: headers("xx", SIG1, "e"), nowMs: NOW }).status,
    400,
  );
});

test("servidor HTTP real: 200, duplicata, 401 e 405", async () => {
  const events = [];
  const srv = createReceiver({
    secret: SECRET,
    onEvent: (e) => events.push(e.id),
    nowFn: () => NOW,
  });
  await new Promise((r) => srv.listen(0, "127.0.0.1", r));
  const { port } = srv.address();
  const post = (body, h) =>
    fetch(`http://127.0.0.1:${port}/`, { method: "POST", body, headers: h });
  try {
    const h = {
      "X-CapIA-Timestamp": TS2,
      "X-CapIA-Signature": `v1=${SIG2}`,
      "X-CapIA-Event-Id": "evt_0002",
    };
    const nowAtTs2 = NOW; // 1767225601000 - 1767225601000 → dentro da janela
    assert.ok(nowAtTs2 - 1767225601000 < 300_000);
    const a = await post(BODY2, h);
    assert.equal(a.status, 200);
    assert.equal((await a.json()).duplicate, false);
    const b = await post(BODY2, h);
    assert.equal((await b.json()).duplicate, true);
    const c = await post(BODY2 + " ", h);
    assert.equal(c.status, 401);
    const d = await fetch(`http://127.0.0.1:${port}/`);
    assert.equal(d.status, 405);
    assert.deepEqual(events, ["evt_0002"]);
  } finally {
    srv.close();
    srv.closeAllConnections();
  }
});
