import assert from "node:assert/strict";
import { test } from "node:test";
import {
  buildSummary,
  failedTests,
  parseCargoSummary,
  stepPassed,
  summarizeMutation,
} from "./lib.mjs";

const SAMPLE = `running 3 tests
test a ... ok
test b ... FAILED
test c ... ok
test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s
running 1 test
test d ... ok
test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
`;

test("parseCargoSummary soma todos os binários", () => {
  assert.deepEqual(parseCargoSummary(SAMPLE), { passed: 3, failed: 1, ignored: 1, binaries: 2 });
  assert.deepEqual(parseCargoSummary("nada"), { passed: 0, failed: 0, ignored: 0, binaries: 0 });
});

test("failedTests lista só os que falharam", () => {
  assert.deepEqual(failedTests(SAMPLE), ["b"]);
});

test("summarizeMutation: sobrevivente ou build quebrado reprova", () => {
  const ok = summarizeMutation([
    [1, "a", "DETECTED", 10],
    [2, "b", "DETECTED", 12],
  ]);
  assert.equal(ok.passed, true);
  assert.equal(ok.detected, 2);
  const bad = summarizeMutation([
    [1, "a", "DETECTED", 10],
    [2, "b", "SURVIVED", 12],
    [3, "c", "BROKE-BUILD", 1],
  ]);
  assert.equal(bad.passed, false);
  assert.deepEqual(
    bad.survivors.map((s) => s.id),
    [2, 3],
  );
  assert.equal(summarizeMutation([]).passed, false, "zero mutações não prova nada");
});

test("stepPassed: só o código 0 passa", () => {
  assert.equal(stepPassed(0), true);
  assert.equal(stepPassed(1), false);
  assert.equal(stepPassed(null), false);
  assert.equal(stepPassed(undefined), false);
});

test("buildSummary: falha de passo ou mutação reprova; mutação não rodada não reprova", () => {
  const steps = [
    { passed: true, tests: { passed: 5, failed: 0 } },
    { passed: true, tests: { passed: 2, failed: 0 } },
  ];
  const mut = { passed: true };
  const a = buildSummary({ generated: "t", steps, mutation: mut });
  assert.equal(a.passed, true);
  assert.deepEqual(a.tests, { passed: 7, failed: 0 });
  assert.equal(buildSummary({ generated: "t", steps, mutation: null }).passed, true);
  assert.equal(buildSummary({ generated: "t", steps, mutation: null }).mutation_run, false);
  assert.equal(buildSummary({ generated: "t", steps, mutation: { passed: false } }).passed, false);
  assert.equal(
    buildSummary({
      generated: "t",
      steps: [{ passed: false, tests: { passed: 0, failed: 1 } }],
      mutation: mut,
    }).passed,
    false,
  );
});
