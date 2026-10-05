import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { evaluateData, evaluateFile } from "../evidence-lib.mjs";
import { evaluate } from "./validate-parity.mjs";

const NOW = Date.parse("2026-10-20T00:00:00Z");
const run = (d) => evaluateData("external-flow-parity", d, evaluate, NOW);
const surface = (over = {}) => ({
  project_revision: 42,
  sequences: 4,
  clips: 87,
  variants: 3,
  run_status: "completed",
  export: { codec: "h264", width: 1080, height: 1920, duration_ticks: 21168000000 },
  ...over,
});
const doc = (over = {}) => ({
  schema: "capia.phase6.parity/1",
  performed_at: "2026-10-18T12:00:00Z",
  attestation: { performed_by: "Tester", results_are_real: true },
  task: { description: "mesma tarefa", same_inputs_for_all_surfaces: true },
  surfaces: { ui: surface(), rest: surface(), mcp: surface() },
  webhook: { received: true, signature_verified: true },
  rest_and_mcp_used_same_token_scopes: true,
  ...over,
});

test("três superfícies com estado idêntico ⇒ accepted", () => {
  const r = run(doc());
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "accepted");
  assert.deepEqual(r.divergences, []);
});

test("qualquer divergência de ESTADO é recusada, mesmo se o arquivo diz equal=true", () => {
  for (const [field, patch] of [
    ["project_revision", { project_revision: 43 }],
    ["sequences", { sequences: 5 }],
    ["clips", { clips: 88 }],
    ["variants", { variants: 2 }],
    [
      "export.codec",
      { export: { codec: "mpeg4", width: 1080, height: 1920, duration_ticks: 21168000000 } },
    ],
    [
      "export.duration_ticks",
      { export: { codec: "h264", width: 1080, height: 1920, duration_ticks: 1 } },
    ],
  ]) {
    const d = doc({ equal: true });
    d.surfaces.mcp = surface(patch);
    const r = run(d);
    assert.equal(r.status, "rejected", field);
    assert.ok(
      r.problems.some((p) => p.includes(`DIVERGÊNCIA em ${field}`)),
      field,
    );
    assert.ok(r.problems.some((p) => p.includes("equal=true")));
  }
});

test("Run não concluída, superfície ausente, webhook não verificado ⇒ rejected", () => {
  const a = doc();
  a.surfaces.rest = surface({ run_status: "failed" });
  assert.equal(run(a).status, "rejected");
  const b = doc();
  delete b.surfaces.ui;
  assert.equal(run(b).status, "rejected");
  assert.equal(
    run(doc({ webhook: { received: true, signature_verified: false } })).status,
    "rejected",
  );
  assert.equal(
    run(doc({ webhook: { received: false, signature_verified: false } })).status,
    "rejected",
  );
  assert.equal(run(doc({ rest_and_mcp_used_same_token_scopes: false })).status, "rejected");
  assert.equal(
    run(doc({ task: { description: "x", same_inputs_for_all_surfaces: false } })).status,
    "rejected",
  );
});

test("atestado e data", () => {
  assert.equal(
    run(doc({ attestation: { performed_by: "x", results_are_real: false } })).status,
    "rejected",
  );
  assert.equal(run(doc({ performed_at: "2031-01-01T00:00:00Z" })).status, "rejected");
  assert.equal(run(doc({ schema: "x" })).status, "rejected");
});

test("sem arquivo e modelo ⇒ pending_external", () => {
  assert.equal(
    evaluateFile("external-flow-parity", "/nonexistent/p.json", evaluate, NOW).status,
    "pending_external",
  );
  const tpl = JSON.parse(readFileSync(new URL("./template-parity.json", import.meta.url), "utf8"));
  assert.equal(run(tpl).status, "pending_external");
  delete tpl.template;
  assert.equal(run(tpl).status, "rejected");
});
