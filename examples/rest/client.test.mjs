// Testa a LÓGICA do cliente de exemplo contra um servidor falso em memória (não é o capia-server):
// idempotência nos POST, envelope de erro, aprovação só do que foi autorizado, polling e export.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ApiError, CapiaClient, extractId, poll, runCanonicalFlow } from "./client.mjs";

function fakeServer({ decisionKind = "plan_approval" } = {}) {
  const log = [];
  let approved = false;
  let polls = 0;
  const srv = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      const body = raw ? JSON.parse(raw) : undefined;
      log.push({ method: req.method, url: req.url, headers: req.headers, body });
      const send = (status, obj) => {
        res.writeHead(status, { "content-type": "application/json" });
        res.end(JSON.stringify(obj));
      };
      if (req.headers.authorization !== "Bearer capia_REDACTED")
        return send(401, { code: "UNAUTHORIZED", message: "no", request_id: "r1" });
      const u = req.url.split("?")[0];
      if (req.method === "POST" && u === "/v1/projects")
        return send(201, { project: { id: "prj_1" } });
      if (u === "/v1/uploads/inline") return send(201, { upload: { id: `upl_${log.length}` } });
      if (u === "/v1/projects/prj_1/assets")
        return send(202, { ticket: { id: `tkt_${body.upload_id}` } });
      if (u.startsWith("/v1/projects/prj_1/imports/"))
        return send(200, { ticket: { state: "completed", asset_id: `ast_${u.split("_").pop()}` } });
      if (req.method === "POST" && u === "/v1/projects/prj_1/runs")
        return send(202, { run: { id: "run_1", status: "running" } });
      if (u === "/v1/projects/prj_1/runs/run_1" && req.method === "GET") {
        polls++;
        if (!approved && polls >= 2)
          return send(200, {
            run: {
              id: "run_1",
              status: "waiting_user",
              pending: {
                id: "dec_1",
                kind: decisionKind,
                question: "Aprovar?",
                options: [{ id: "approve" }, { id: "reject" }],
              },
            },
          });
        if (!approved) return send(200, { run: { id: "run_1", status: "running" } });
        return send(200, { run: { id: "run_1", status: "completed", sequences: ["seq_1"] } });
      }
      if (u === "/v1/projects/prj_1/runs/run_1/approvals") {
        approved = true;
        return send(200, { ok: true });
      }
      if (u === "/v1/projects/prj_1/runs/run_1/variants") return send(202, { runs: [] });
      if (req.method === "POST" && u === "/v1/projects/prj_1/exports")
        return send(202, { export: { id: "exp_1", state: "queued" } });
      if (u === "/v1/projects/prj_1/exports/exp_1")
        return send(200, { export: { id: "exp_1", state: "completed" } });
      if (u === "/v1/projects/prj_1/summary") return send(200, { revision: 7 });
      return send(404, { code: "NOT_FOUND", message: u, request_id: "r2" });
    });
  });
  return { srv, log };
}

async function withServer(opts, fn) {
  const { srv, log } = fakeServer(opts);
  await new Promise((r) => srv.listen(0, "127.0.0.1", r));
  try {
    await fn(srv.address().port, log);
  } finally {
    srv.close();
    srv.closeAllConnections();
  }
}

const files = () => {
  const d = mkdtempSync(join(tmpdir(), "capia-ex-"));
  writeFileSync(join(d, "raw.mp4"), "raw");
  writeFileSync(join(d, "ref.mp4"), "ref");
  return { raw: join(d, "raw.mp4"), reference: join(d, "ref.mp4") };
};

test("erro HTTP vira ApiError com o envelope {code,message,request_id}", async () => {
  await withServer({}, async (port) => {
    const bad = new CapiaClient({ port, token: "errado" });
    await assert.rejects(
      () => bad.get("/v1/projects"),
      (e) =>
        e instanceof ApiError &&
        e.status === 401 &&
        e.code === "UNAUTHORIZED" &&
        e.body.request_id === "r1",
    );
  });
});

test("POST leva Idempotency-Key (reutilizável); GET não leva", async () => {
  await withServer({}, async (port, log) => {
    const c = new CapiaClient({ port, token: "capia_REDACTED" });
    await c.post("/v1/projects", { name: "x" }, "chave-fixa");
    await c.get("/v1/projects/prj_1/summary");
    assert.equal(log[0].headers["idempotency-key"], "chave-fixa");
    assert.equal(log[1].headers["idempotency-key"], undefined);
  });
});

test("sem --approve o fluxo PARA na decisão em vez de aprovar sozinho", async () => {
  await withServer({}, async (port) => {
    const c = new CapiaClient({ port, token: "capia_REDACTED" });
    const out = await runCanonicalFlow(c, { ...files(), brief: "b" }, () => {});
    assert.equal(out.status, "waiting_user");
    assert.equal(out.decision.id, "dec_1");
  });
});

test("fluxo completo com aprovação autorizada: projeto → assets → Run → aprovação → export", async () => {
  await withServer({}, async (port, log) => {
    const c = new CapiaClient({ port, token: "capia_REDACTED" });
    const sleepFn = async () => {};
    const out = await runCanonicalFlow(
      c,
      { ...files(), brief: "briefing", autoApprove: ["plan_approval"], variants: 2, sleepFn },
      () => {},
    );
    assert.equal(out.status, "completed");
    assert.equal(out.exportId, "exp_1");
    const run = log.find((l) => l.method === "POST" && l.url.endsWith("/runs"));
    assert.equal(run.body.brief_text, "briefing");
    assert.equal(run.body.start, true);
    assert.equal(run.body.assets.length, 1);
    assert.equal(run.body.references.length, 1);
    const exp = log.find((l) => l.method === "POST" && l.url.endsWith("/exports"));
    assert.deepEqual(exp.body.items, [{ sequence: "seq_1", preset: "h264-mp4" }]);
    assert.ok(log.some((l) => l.url.endsWith("/variants") && l.body.count === 2));
  });
});

test("decisão de outro tipo (gasto) não é aprovada por --approve plan_approval", async () => {
  await withServer({ decisionKind: "spend_approval" }, async (port, log) => {
    const c = new CapiaClient({ port, token: "capia_REDACTED" });
    const out = await runCanonicalFlow(
      c,
      { ...files(), brief: "b", autoApprove: ["plan_approval"] },
      () => {},
    );
    assert.equal(out.status, "waiting_user");
    assert.ok(!log.some((l) => l.url.endsWith("/approvals")));
  });
});

test("extractId e poll", async () => {
  assert.equal(extractId({ run: { id: "a" } }, "run"), "a");
  assert.equal(extractId({ id: "b" }, "run"), "b");
  assert.throws(() => extractId({}, "run"), /sem id/);
  let n = 0;
  assert.equal(
    await poll(
      async () => ++n,
      (v) => v === 3,
      { sleepFn: async () => {} },
    ),
    3,
  );
  await assert.rejects(
    () =>
      poll(
        async () => 0,
        () => false,
        { timeoutMs: -1, sleepFn: async () => {} },
      ),
    /tempo esgotado/,
  );
});
