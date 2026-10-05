import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const run = (obj) => {
  const f = join(mkdtempSync(join(tmpdir(), "p5-")), "r.json");
  writeFileSync(f, JSON.stringify(obj));
  const r = spawnSync("node", [new URL("./validate.mjs", import.meta.url).pathname, "--file", f], {
    encoding: "utf8",
  });
  return { code: r.status, out: JSON.parse(r.stdout) };
};
const base = (n, rating) => ({
  provider: "p",
  model: "m",
  evaluated_at: "2026-01-01",
  rater: "r",
  demands: Array.from({ length: n }, (_, i) => ({
    id: `d${i}`,
    run_id: `run-${i}`,
    brief_file: `b${i}.docx`,
    rating,
  })),
});

test("sem arquivo é pendência externa, nunca aprovado", () => {
  const r = spawnSync(
    "node",
    [new URL("./validate.mjs", import.meta.url).pathname, "--file", "/nonexistent.json"],
    { encoding: "utf8" },
  );
  assert.equal(r.status, 0);
  assert.equal(JSON.parse(r.stdout).status, "pending_external");
});
test("aceita 10 demandas com média ≥ 4,0", () =>
  assert.equal(run(base(10, 4)).out.status, "accepted"));
test("recusa média < 4,0, menos de 10, nota inválida e campos faltando", () => {
  assert.equal(run(base(10, 3)).out.status, "rejected");
  assert.equal(run(base(9, 5)).out.status, "rejected");
  assert.equal(run(base(10, 6)).out.status, "rejected");
  const x = base(10, 5);
  delete x.rater;
  assert.equal(run(x).out.status, "rejected");
});
