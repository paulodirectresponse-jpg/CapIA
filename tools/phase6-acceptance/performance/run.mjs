#!/usr/bin/env node
// Gate de regressão de desempenho (Fase 6, D-1). Roda o benchmark N vezes, lê os JSON, toma a
// MEDIANA de cada estatística entre as execuções e compara com `thresholds.json`. Escreve
// `target/phase6-acceptance/performance.json`. Nada é aprovado em vazio: métrica ausente, ou pulada
// sem motivo permitido, REPROVA.
//
// Uso: node tools/phase6-acceptance/performance/run.mjs [--runs N] [--reports dir ...]
//   --reports  avalia JSON já gerados (sem rodar cargo): cada dir tem phase6-large-project.json
// Env: CAPIA_PERF_TOLERANCE (substitui tolerance_factor; nunca afeta limites 'hard'), CAPIA_PERF_REPS.
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const root = resolve(here, "../../..");
const REPORT = "phase6-large-project.json";

export function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  const n = s.length;
  if (n === 0) return NaN;
  return n % 2 ? s[(n - 1) / 2] : (s[n / 2 - 1] + s[n / 2]) / 2;
}

/** reports = JSON de cada execução. Devolve { passed, tolerance, results[] }. */
export function evaluate(reports, thresholds, env = {}) {
  const tolEnv = Number(env.CAPIA_PERF_TOLERANCE);
  const tolerance =
    Number.isFinite(tolEnv) && tolEnv > 0 ? tolEnv : (thresholds.tolerance_factor ?? 1);
  const results = [];
  for (const [name, thr] of Object.entries(thresholds.metrics)) {
    const metrics = reports.map((r) => r?.metrics?.[name]);
    const base = {
      metric: name,
      hard: !!thr.hard,
      source: thr.source ?? null,
      runs: reports.length,
    };
    const fail = (why) => results.push({ ...base, status: "fail", reason: why });
    if (reports.length === 0 || metrics.some((m) => m === undefined)) {
      fail("métrica ausente em pelo menos uma execução");
      continue;
    }
    const skipped = metrics.filter((m) => m.skipped);
    if (skipped.length) {
      const reason = skipped[0].skipped;
      if (
        thr.allow_skip &&
        String(reason).includes(thr.allow_skip) &&
        skipped.length === metrics.length
      ) {
        results.push({ ...base, status: "skipped", reason });
      } else {
        fail(`pulada sem motivo permitido: ${reason}`);
      }
      continue;
    }
    const factor = thr.hard ? 1 : tolerance;
    const checks = [];
    let invalid = null;
    for (const stat of ["p50", "p95"]) {
      const max = thr[`max_${stat}_ms`];
      if (max === undefined) continue;
      const values = metrics.map((m) => m[stat]);
      if (values.some((v) => typeof v !== "number" || Number.isNaN(v))) {
        invalid = `${stat} inválido`;
        break;
      }
      const med = median(values);
      const limit = max * factor;
      checks.push({ stat, median_ms: med, runs_ms: values, limit_ms: limit, pass: med <= limit });
    }
    if (invalid) fail(invalid);
    else if (checks.length === 0) fail("sem limite configurado");
    else {
      results.push({
        ...base,
        status: checks.every((c) => c.pass) ? "pass" : "fail",
        factor,
        checks,
        n_per_run: metrics[0].n,
      });
    }
  }
  return { passed: results.every((r) => r.status !== "fail"), tolerance, results };
}

function parseArgs(argv) {
  const out = { runs: undefined, reports: [] };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--runs") out.runs = Number(argv[++i]);
    else if (argv[i] === "--reports") {
      while (argv[i + 1] && !argv[i + 1].startsWith("--")) out.reports.push(argv[++i]);
    }
  }
  return out;
}

function runBenchmark(dir) {
  const r = spawnSync(
    "cargo",
    [
      "test",
      "--release",
      "-p",
      "capia-project",
      "--test",
      "perf_large",
      "--",
      "--ignored",
      "--nocapture",
      "--test-threads=1",
    ],
    {
      cwd: root,
      stdio: "inherit",
      env: { ...process.env, CAPIA_PERF_OUT: dir, CARGO_INCREMENTAL: "0" },
      shell: process.platform === "win32",
    },
  );
  if (r.status !== 0) throw new Error(`benchmark falhou (status ${r.status})`);
  return JSON.parse(readFileSync(join(dir, REPORT), "utf8"));
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const thresholds = JSON.parse(readFileSync(join(here, "thresholds.json"), "utf8"));
  const runs = args.runs ?? thresholds.runs;
  let reports;
  let source;
  if (args.reports.length) {
    reports = args.reports.map((d) => JSON.parse(readFileSync(join(d, REPORT), "utf8")));
    source = "reports";
  } else {
    reports = [];
    for (let i = 0; i < runs; i++) {
      const dir = mkdtempSync(join(tmpdir(), `capia-perf-${i}-`));
      console.log(`\n=== execução ${i + 1}/${runs} ===`);
      reports.push(runBenchmark(dir));
    }
    source = "cargo";
  }
  const verdict = evaluate(reports, thresholds, process.env);
  const out = {
    suite: "phase6-performance",
    generated: new Date().toISOString(),
    source,
    runs: reports.length,
    machine: reports[0]?.machine ?? null,
    fixture: reports[0]?.extra?.fixture ?? null,
    ...verdict,
  };
  const dest = join(root, "target/phase6-acceptance");
  mkdirSync(dest, { recursive: true });
  writeFileSync(join(dest, "performance.json"), JSON.stringify(out, null, 2));
  for (const r of verdict.results) {
    const detail = r.checks
      ? r.checks
          .map((c) => `${c.stat} ${c.median_ms.toFixed(2)}/${c.limit_ms.toFixed(2)}ms`)
          .join("  ")
      : r.reason;
    console.log(`${r.status.toUpperCase().padEnd(7)} ${r.metric.padEnd(34)} ${detail}`);
  }
  console.log(
    `\n${verdict.passed ? "PERFORMANCE GATE: PASS" : "PERFORMANCE GATE: FAIL"} (tolerância ${verdict.tolerance}x nos não-hard) -> target/phase6-acceptance/performance.json`,
  );
  process.exit(verdict.passed ? 0 : 1);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
