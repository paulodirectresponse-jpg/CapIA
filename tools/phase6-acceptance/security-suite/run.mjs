#!/usr/bin/env node
// Suíte de segurança do servidor REST (Fase 6, Track D-2): testes Rust de pentest/canário/fuzz/queda
// + mutação. Grava `target/phase6-acceptance/security-suite.json`. Uso:
//   node tools/phase6-acceptance/security-suite/run.mjs [--skip-mutation] [--fuzz-cases N]
// A mutação leva dezenas de minutos (cada mutação recompila): `--skip-mutation` a omite e o
// resumo diz `mutation_run: false` (nunca finge que rodou).
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, existsSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildSummary, failedTests, parseCargoSummary, stepPassed, summarizeMutation } from "./lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const out = join(root, "target/phase6-acceptance");
mkdirSync(out, { recursive: true });

const args = process.argv.slice(2);
const skipMutation = args.includes("--skip-mutation");
const fuzzIdx = args.indexOf("--fuzz-cases");
const env = {
  ...process.env,
  CARGO_INCREMENTAL: "0",
  ...(fuzzIdx >= 0 ? { CAPIA_FUZZ_CASES: args[fuzzIdx + 1] } : {}),
};

function run(label, bin, binArgs) {
  const t0 = Date.now();
  const r = spawnSync(bin, binArgs, { cwd: root, encoding: "utf8", env, maxBuffer: 1 << 28 });
  const text = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const passed = stepPassed(r.status);
  console.log(`${passed ? "PASS" : "FAIL"}  ${label}`);
  return {
    label,
    command: `${bin} ${binArgs.join(" ")}`,
    passed,
    seconds: Math.round((Date.now() - t0) / 100) / 10,
    tests: parseCargoSummary(text),
    failed_tests: failedTests(text),
    tail: passed ? undefined : text.split("\n").slice(-30).join("\n"),
  };
}

const cargo = (label, extra) => run(label, "cargo", ["test", "-p", "capia-server", "--no-fail-fast", ...extra]);

const steps = [
  cargo("pentest REST (auth, escopos, tokens, rotas, caminhos, SSRF, upload, forma, CORS, idempotência, DoS)", [
    "--test",
    "security_rest",
  ]),
  cargo("canário de segredo (respostas, disco, SSE, stdout/stderr do processo real)", ["--test", "secret_canary"]),
  cargo("fuzz determinístico + corpus commitado", ["--test", "fuzz_rest"]),
  cargo("resiliência a SIGKILL (upload, apply, idempotência, export)", ["--test", "crash_rest"]),
  cargo("testes unitários do servidor (sanitização, sniff, rate limit, http)", ["--lib"]),
  run("política de URL/SSRF do capia-ai", "cargo", ["test", "-p", "capia-ai", "--lib", "http"]),
  run("arquitetura: fronteiras e testkit fora do produto", "pnpm", ["check:arch"]),
];

let mutation = null;
if (!skipMutation) {
  const t0 = Date.now();
  const r = spawnSync("python3", ["tools/mutation-phase6.py"], { cwd: root, encoding: "utf8", env, maxBuffer: 1 << 28 });
  console.log(r.stdout ?? "");
  const file = join(root, "target/mutation-phase6.json");
  const rows = existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : [];
  mutation = { ...summarizeMutation(rows), seconds: Math.round((Date.now() - t0) / 1000), exit: r.status };
  console.log(`${mutation.passed ? "PASS" : "FAIL"}  mutação: ${mutation.detected}/${mutation.total} detectadas`);
}

const summary = buildSummary({ generated: new Date().toISOString(), steps, mutation });
writeFileSync(join(out, "security-suite.json"), JSON.stringify(summary, null, 2));
console.log(
  `\nsecurity-suite.json: ${summary.passed ? "ALL PASSED" : "FAILURES"} (${summary.tests.passed} testes Rust ok, ${summary.tests.failed} falhas${
    mutation ? "" : "; mutação NÃO rodada"
  })`,
);
process.exit(summary.passed ? 0 : 1);
