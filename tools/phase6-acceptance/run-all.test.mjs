import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { validateSpec } from "./steps-runner.mjs";
import {
  STEP_SUITES,
  VALIDATOR_SUITES,
  finalStatus,
  fromPerformanceSummary,
  fromStepSummary,
  fromValidatorSummary,
} from "./run-all.mjs";

const here = new URL(".", import.meta.url).pathname;
const S = (...statuses) => Object.fromEntries(statuses.map((st, i) => [`s${i}`, { status: st }]));

test("estado final: FAILURES > INCOMPLETE > ENGINEERING COMPLETE > PHASE 6 COMPLETE", () => {
  assert.equal(finalStatus(S("passed", "failed", "not_available")), "FAILURES");
  assert.match(finalStatus(S("passed", "not_available", "pending_external")), /^INCOMPLETE/);
  assert.equal(
    finalStatus(S("passed", "pending_external")),
    "PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING",
  );
  assert.equal(finalStatus(S("passed", "passed")), "PHASE 6 COMPLETE");
});

test("PHASE 6 COMPLETE nunca sai com pendência externa ou suíte indisponível", () => {
  for (const st of ["pending_external", "not_available", "failed"])
    assert.notEqual(finalStatus(S("passed", "passed", st)), "PHASE 6 COMPLETE");
});

test("validadores: partial e pending_external contam como pendência externa; rejected é falha", () => {
  assert.equal(fromValidatorSummary({ status: "accepted" }).status, "passed");
  assert.equal(
    fromValidatorSummary({ status: "partial", note: "falta win10" }).status,
    "pending_external",
  );
  assert.equal(fromValidatorSummary({ status: "pending_external" }).status, "pending_external");
  assert.equal(fromValidatorSummary({ status: "rejected", problems: ["x"] }).status, "failed");
  assert.equal(fromValidatorSummary({ status: "algo-novo" }).status, "failed");
  assert.equal(fromValidatorSummary(null).status, "not_available");
});

test("resumo de passos: lista o que está pendente/indisponível e por quê", () => {
  const r = fromStepSummary({
    status: "not_available",
    counts: {},
    steps: [
      { id: "a", status: "passed" },
      { id: "b", status: "pending_external", why: "certificado" },
      { id: "c", status: "not_available", reason: "falta binário" },
    ],
  });
  assert.deepEqual(r.pending_external, [{ id: "b", why: "certificado" }]);
  assert.deepEqual(r.not_available, [{ id: "c", reason: "falta binário" }]);
  assert.equal(fromStepSummary(null).status, "not_available");
});

test("desempenho (outra frente): ausente é not_available; aceita status ou passed", () => {
  assert.equal(fromPerformanceSummary(null).status, "not_available");
  assert.equal(fromPerformanceSummary({ status: "pending_external" }).status, "pending_external");
  assert.equal(fromPerformanceSummary({ passed: true }).status, "passed");
  assert.equal(fromPerformanceSummary({ passed: false }).status, "failed");
  assert.equal(fromPerformanceSummary({ foo: 1 }).status, "failed");
});

test("todos os steps.json do pacote são válidos e cada passo externo tem evidência e motivo", () => {
  for (const s of STEP_SUITES) {
    const spec = JSON.parse(readFileSync(join(here, s, "steps.json"), "utf8"));
    assert.equal(spec.suite, s);
    validateSpec(spec);
    for (const step of spec.steps.filter((x) => x.external)) {
      assert.match(step.evidence, /^target\/phase6-acceptance\/evidence\//);
      assert.ok(step.why.length > 20);
    }
  }
  assert.deepEqual(VALIDATOR_SUITES, ["clean-machine", "beta-feedback"]);
});

test("run.mjs de cada suíte lista os passos e funciona sem os binários de outras frentes", () => {
  for (const s of STEP_SUITES) {
    const out = mkdtempSync(join(tmpdir(), "p6-all-"));
    const env = { ...process.env, CAPIA_P6_OUT: out, CAPIA_P6_HEAVY: "" };
    const list = spawnSync(process.execPath, [join(here, s, "run.mjs"), "--list"], {
      encoding: "utf8",
      env,
    });
    assert.equal(list.status, 0, s);
    assert.ok(list.stdout.trim().split("\n").length >= 3, s);
  }
});

test("run.mjs da suíte security roda sem Rust pesado: passos pesados ficam not_available, externo pendente", () => {
  const out = mkdtempSync(join(tmpdir(), "p6-sec-"));
  const env = { ...process.env, CAPIA_P6_OUT: out, CAPIA_P6_HEAVY: "" };
  delete env.CAPIA_P6_HEAVY;
  const r = spawnSync(
    process.execPath,
    [
      join(here, "security/run.mjs"),
      "--only",
      "catalog-scope-invariants,independent-pentest,mac-primitives",
    ],
    {
      encoding: "utf8",
      env,
    },
  );
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const sum = JSON.parse(readFileSync(join(out, "security-summary.json"), "utf8"));
  const by = Object.fromEntries(sum.steps.map((x) => [x.id, x.status]));
  assert.equal(by["catalog-scope-invariants"], "not_available");
  assert.equal(by["mac-primitives"], "not_available");
  assert.equal(by["independent-pentest"], "pending_external");
  assert.notEqual(sum.status, "passed");
});
