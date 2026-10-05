#!/usr/bin/env node
// Agregador de um clique: roda as 5 suítes automáticas e valida o pacote humano (demandas reais).
// Grava `target/phase5-acceptance/summary.json`. As pendências externas aparecem como
// `pending_external` — nunca como aprovadas.
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { out, root } from "./lib.mjs";

const suites = ["autonomy", "crash-resume", "security", "gateway", "memory"];
const results = {};
for (const s of suites) {
  const r = spawnSync("node", [join(root, `tools/phase5-acceptance/${s}/run.mjs`)], {
    cwd: root,
    stdio: "inherit",
  });
  let summary = null;
  try {
    summary = JSON.parse(readFileSync(join(out, `${s}-summary.json`), "utf8"));
  } catch {
    /* suíte não gerou resumo */
  }
  results[s] = { exit: r.status, passed: r.status === 0 && summary?.passed === true };
}
const real = spawnSync("node", [join(root, "tools/phase5-acceptance/real-demands/validate.mjs")], {
  cwd: root,
  encoding: "utf8",
});
console.log(real.stdout);
let realSummary = null;
try {
  realSummary = JSON.parse(readFileSync(join(out, "real-demands-summary.json"), "utf8"));
} catch {
  /* sem resumo */
}
const automatic = suites.every((s) => results[s].passed);
const summary = {
  generated: new Date().toISOString(),
  automatic: { passed: automatic, suites: results },
  external: {
    realDemands: realSummary?.status ?? "pending_external",
    liveProviders: "pending_external",
    note: "Demandas reais (≥ 10, nota humana média ≥ 4,0) e provedores reais não rodam neste pacote automático.",
  },
  status: automatic
    ? realSummary?.status === "accepted"
      ? "PHASE 5 ACCEPTED"
      : "PHASE 5 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING"
    : "FAILURES",
};
writeFileSync(join(out, "summary.json"), JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary, null, 2));
process.exit(automatic ? 0 : 1);
