import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { evaluate, median } from "./run.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const real = JSON.parse(readFileSync(join(here, "thresholds.json"), "utf8"));
const m = (p50, p95) => ({ unit: "ms", n: 9, p50, p95, max: p95, skipped: null });
const thr = (metrics) => ({ tolerance_factor: 2, metrics });
const rep = (metrics) => ({ metrics });

test("median handles odd/even/empty", () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  assert.ok(Number.isNaN(median([])));
});

test("one noisy run does not fail the gate (median of runs) but a real regression does", () => {
  const t = thr({ a: { max_p95_ms: 10 } });
  const noisy = evaluate([rep({ a: m(1, 5) }), rep({ a: m(1, 90) }), rep({ a: m(1, 6) })], t);
  assert.equal(noisy.passed, true);
  const regressed = evaluate([rep({ a: m(1, 40) }), rep({ a: m(1, 50) }), rep({ a: m(1, 6) })], t);
  assert.equal(regressed.passed, false);
  assert.equal(regressed.results[0].checks[0].median_ms, 40);
});

test("tolerance multiplies regression bounds but NEVER hard targets", () => {
  const t = thr({ soft: { max_p95_ms: 10 }, hard: { max_p95_ms: 10, hard: true } });
  const r = evaluate([rep({ soft: m(1, 19), hard: m(1, 19) })], t);
  const by = Object.fromEntries(r.results.map((x) => [x.metric, x]));
  assert.equal(by.soft.status, "pass"); // 19 <= 10 * 2
  assert.equal(by.hard.status, "fail"); // 19 > 10, sem folga
  const env = evaluate([rep({ soft: m(1, 19), hard: m(1, 9) })], t, { CAPIA_PERF_TOLERANCE: "1" });
  assert.equal(env.results.find((x) => x.metric === "soft").status, "fail");
  assert.equal(env.tolerance, 1);
});

test("missing, invalid and unexplained-skipped metrics fail (no vacuous pass)", () => {
  const t = thr({ a: { max_p95_ms: 10 } });
  assert.equal(evaluate([], t).passed, false);
  assert.equal(evaluate([rep({})], t).passed, false);
  assert.equal(evaluate([rep({ a: m(1, 5) }), rep({})], t).passed, false);
  assert.equal(evaluate([rep({ a: { skipped: "whatever" } })], t).passed, false);
  assert.equal(evaluate([rep({ a: m(1, Number.NaN) })], t).passed, false);
  assert.equal(evaluate([rep({ a: m(1, 5) })], thr({ a: {} })).passed, false);
});

test("a skip is accepted only with the explicitly allowed reason, in every run", () => {
  const t = thr({ a: { max_p95_ms: 10, allow_skip: "no ffmpeg" } });
  const skip = { skipped: "no ffmpeg/ffprobe: x" };
  assert.equal(evaluate([rep({ a: skip }), rep({ a: skip })], t).results[0].status, "skipped");
  assert.equal(evaluate([rep({ a: skip }), rep({ a: m(1, 1) })], t).passed, false);
  assert.equal(evaluate([rep({ a: { skipped: "disk" } })], t).passed, false);
});

test("p50 and p95 limits are both enforced", () => {
  const t = thr({ a: { max_p50_ms: 5, max_p95_ms: 50 } });
  assert.equal(evaluate([rep({ a: m(4, 40) })], t).passed, true);
  assert.equal(evaluate([rep({ a: m(11, 40) })], t).passed, false);
});

test("shipped thresholds are well-formed and keep the documented hard targets", () => {
  assert.ok(real.runs >= 3 && real.tolerance_factor >= 1);
  const need = {
    commit_small: 30,
    undo: 50,
    redo: 50,
    preview_frame_warm: 100,
    open_project: 2000,
    reload_state: 2000,
  };
  for (const [name, max] of Object.entries(need)) {
    assert.equal(real.metrics[name].hard, true, name);
    assert.ok(real.metrics[name].max_p95_ms <= max, `${name} must not be relaxed above ${max}`);
  }
  for (const [name, t] of Object.entries(real.metrics)) {
    assert.ok(t.max_p95_ms > 0 || t.max_p50_ms > 0, name);
    if (t.baseline_p95_ms !== undefined)
      assert.ok(t.max_p95_ms >= t.baseline_p95_ms, `${name}: bound below baseline`);
  }
});

test("evaluating a full report checks every threshold metric", () => {
  const all = Object.fromEntries(Object.keys(real.metrics).map((k) => [k, m(0.5, 1)]));
  const r = evaluate([rep(all), rep(all), rep(all)], real);
  assert.equal(r.passed, true);
  assert.equal(r.results.length, Object.keys(real.metrics).length);
});
