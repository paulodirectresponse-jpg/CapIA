#!/usr/bin/env node
// Soak manual/noturno (Fase 6, D-1): ciclos de abrir/commit/undo/ler/fechar no projeto grande,
// com RSS/descritores/threads amostrados. Parametrizado por duração; NÃO roda no CI principal.
//   node tools/phase6-acceptance/performance/soak.mjs --seconds 28800 [--spec default|medium]
// 8 h = `--seconds 28800`. Grava target/perf/phase6-soak.json (amostras) e
// target/phase6-acceptance/soak.json (veredito). Falha se RSS/fd/threads crescem sem limite
// ou se o documento mudou (asserções do teste `soak_long`).
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const seconds = Number(arg("seconds", "60"));
const spec = arg("spec", "medium");
if (!Number.isFinite(seconds) || seconds < 1 || !["default", "medium"].includes(spec)) {
  console.error("uso: soak.mjs --seconds <n>=1 [--spec default|medium]");
  process.exit(2);
}
const t0 = Date.now();
const r = spawnSync(
  "cargo",
  [
    "test",
    "--release",
    "-p",
    "capia-project",
    "--test",
    "soak",
    "soak_long",
    "--",
    "--ignored",
    "--nocapture",
  ],
  {
    cwd: root,
    stdio: "inherit",
    env: {
      ...process.env,
      CAPIA_SOAK_SECS: String(seconds),
      CAPIA_SOAK_SPEC: spec,
      CARGO_INCREMENTAL: "0",
    },
    shell: process.platform === "win32",
  },
);
let report = null;
try {
  const base =
    process.env.CAPIA_PERF_OUT ??
    join(process.env.CARGO_TARGET_DIR ?? join(root, "target"), "perf");
  report = JSON.parse(readFileSync(join(base, "phase6-soak.json"), "utf8"));
} catch {
  /* sem relatório: reprova abaixo */
}
const passed = r.status === 0 && report !== null;
const out = {
  suite: "phase6-soak",
  generated: new Date().toISOString(),
  requested_seconds: seconds,
  spec,
  wall_seconds: Math.round((Date.now() - t0) / 1000),
  passed,
  cycles: report?.extra?.cycles ?? null,
  samples: report?.extra?.samples?.length ?? 0,
  machine: report?.machine ?? null,
};
const dest = join(root, "target/phase6-acceptance");
mkdirSync(dest, { recursive: true });
writeFileSync(join(dest, "soak.json"), JSON.stringify(out, null, 2));
console.log(JSON.stringify(out, null, 2));
process.exit(passed ? 0 : 1);
