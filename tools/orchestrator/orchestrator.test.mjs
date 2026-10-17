// node --test tools/orchestrator
import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { classifyFailure, runProcess } from "./lib/agents.mjs";
import { extractVerdictJson } from "./lib/engine.mjs";
import { evaluateLimits } from "./lib/limits.mjs";
import { runAllScenarios } from "./lib/scenarios.mjs";
import { redact } from "./lib/util.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const cfg = JSON.parse(readFileSync(join(here, "orchestrator.config.json"), "utf8"));

test("dry-run: todos os cenários da máquina de estados passam", async () => {
  const results = await runAllScenarios(cfg);
  const failed = results.filter((r) => !r.pass);
  assert.deepEqual(failed, [], JSON.stringify(failed, null, 2));
  assert.ok(results.length >= 13);
});

test("timeout mata a ÁRVORE: um neto que segura o pipe não trava o orquestrador", async () => {
  // pai Node lança um neto que dorme 60 s e herda stdout; matar só o pai deixaria o `close` pendente
  const script = `require("node:child_process").spawn(process.execPath,["-e","setTimeout(()=>{},60000)"],{stdio:"inherit"});setTimeout(()=>{},60000)`;
  const t0 = Date.now();
  const r = await runProcess({
    bin: process.execPath,
    args: ["-e", script],
    input: "",
    cwd: process.cwd(),
    timeoutMs: 800,
  });
  assert.equal(r.timedOut, true);
  assert.ok(Date.now() - t0 < 8000, `demorou ${Date.now() - t0} ms`);
});

test("classificação de falhas: auth não é repetida, rate limit é", () => {
  assert.equal(classifyFailure("Not logged in · Please run /login").kind, "AUTH");
  assert.equal(classifyFailure("Credit balance is too low").kind, "AUTH");
  assert.equal(classifyFailure("429 rate limit exceeded").transient, true);
  assert.equal(classifyFailure("503 overloaded").kind, "TRANSIENT");
});

test("extractVerdictJson acha o JSON mesmo cercado de texto e ignora JSON sem verdict", () => {
  const text = 'análise {"x":1}\n```json\n{"verdict":"FAIL","findings":[{"issue":"a"}]}\n```\nfim';
  assert.equal(extractVerdictJson(text)?.verdict, "FAIL");
  assert.equal(extractVerdictJson('{"x":1}'), null);
  assert.equal(extractVerdictJson("sem json"), null);
});

test("redact remove chaves e tokens", () => {
  const out = redact(
    "key sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX e Bearer abcdefghijklmnopqrstuvwxyz0123 e API_KEY=hunter2hunter2",
  );
  assert.ok(!/sk-ant|abcdefghijklmnop|hunter2/.test(out), out);
});

test("evaluateLimits: cada limite dispara com a razão certa e nada dispara no estado inicial", () => {
  const base = {
    iteration: 1,
    counters: {
      wallMs: 0,
      costUsd: 0,
      claudeCalls: 0,
      codexCalls: 0,
      consecutiveFailures: 0,
      noProgress: 0,
    },
  };
  assert.equal(evaluateLimits({ limits: cfg.limits, state: base, sessionWallMs: 0 }), null);
  const over = (patch, counters = {}) =>
    evaluateLimits({
      limits: cfg.limits,
      state: { ...base, ...patch, counters: { ...base.counters, ...counters } },
      sessionWallMs: 0,
    });
  assert.match(over({ iteration: cfg.limits.maxIterations + 1 }), /maxIterations/);
  assert.match(over({}, { costUsd: cfg.limits.maxCostUsd }), /maxCostUsd/);
  assert.match(over({}, { wallMs: cfg.limits.maxWallClockMinutes * 60_000 }), /maxWallClock/);
  assert.match(over({}, { claudeCalls: cfg.limits.maxClaudeCalls }), /maxClaudeCalls/);
  assert.match(over({}, { codexCalls: cfg.limits.maxCodexCalls }), /maxCodexCalls/);
  assert.match(
    over({}, { consecutiveFailures: cfg.limits.maxConsecutiveFailures }),
    /maxConsecutiveFailures/,
  );
  assert.match(over({}, { noProgress: cfg.limits.maxNoProgress }), /no-progress/);
});
