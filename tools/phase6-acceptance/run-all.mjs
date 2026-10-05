#!/usr/bin/env node
// Agregador da aceitação da Fase 6 (mesma filosofia de tools/phase5-acceptance/run-all.mjs):
// roda as suítes declarativas (external-flow, installer, update, security), os validadores de
// evidência humana (clean-machine, beta-feedback) e — se a frente de desempenho já entregou — a
// suíte `performance`. Grava `target/phase6-acceptance/summary.json`.
//
// Vocabulário: passed | failed | pending_external | not_available. O estado final segue o spec:
//   FAILURES                      alguma suíte falhou
//   INCOMPLETE …                  há suíte `not_available` (binários/testes de outras frentes ausentes):
//                                 NÃO se declara engenharia completa
//   PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING
//                                 tudo automático passou; falta evidência externa real
//   PHASE 6 COMPLETE              TODAS as suítes passaram, inclusive as evidências externas reais
//   node tools/phase6-acceptance/run-all.mjs [--strict]    (--strict: exit 1 se não for PHASE 6 COMPLETE)
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { aggregateStatus, outDir, root } from "./steps-runner.mjs";

export const STEP_SUITES = ["external-flow", "installer", "update", "security"];
export const VALIDATOR_SUITES = ["clean-machine", "beta-feedback"];
const VALIDATOR_MAP = {
  accepted: "passed",
  passed: "passed",
  rejected: "failed",
  pending_external: "pending_external",
  partial: "pending_external",
};

/** Estado de uma suíte de passos a partir do resumo do runner. */
export function fromStepSummary(sum) {
  if (!sum || !sum.status) return { status: "not_available", note: "a suíte não produziu resumo" };
  const pend = sum.steps?.filter((s) => s.status === "pending_external") ?? [];
  const na = sum.steps?.filter((s) => s.status === "not_available") ?? [];
  return {
    status: sum.status,
    counts: sum.counts,
    pending_external: pend.map((s) => ({ id: s.id, why: s.why ?? s.reason })),
    not_available: na.map((s) => ({ id: s.id, reason: s.reason })),
  };
}

/** Estado de um validador de evidência humana. */
export function fromValidatorSummary(sum) {
  if (!sum || !sum.status)
    return { status: "not_available", note: "o validador não produziu resumo" };
  return {
    status: VALIDATOR_MAP[sum.status] ?? "failed",
    validator_status: sum.status,
    reason: sum.reason ?? sum.note,
    problems: sum.problems,
  };
}

/** Suíte de desempenho (outra frente): aceita `status` no nosso vocabulário ou `passed: boolean`. */
export function fromPerformanceSummary(sum) {
  if (!sum)
    return {
      status: "not_available",
      note: "a suíte de desempenho (outra frente) ainda não existe ou não gerou resumo",
    };
  if (["passed", "failed", "pending_external", "not_available"].includes(sum.status))
    return { status: sum.status };
  if (typeof sum.passed === "boolean") return { status: sum.passed ? "passed" : "failed" };
  return { status: "failed", note: "resumo de desempenho em formato desconhecido" };
}

export function finalStatus(suites) {
  const agg = aggregateStatus(Object.values(suites).map((s) => s.status));
  if (agg === "failed") return "FAILURES";
  if (agg === "not_available")
    return "INCOMPLETE — suítes sem binários/testes de outras frentes (veja summary.json); engenharia NÃO declarada completa";
  if (agg === "pending_external")
    return "PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING";
  return "PHASE 6 COMPLETE";
}

const readJson = (file) => {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
};

function runNode(script, args = []) {
  return spawnSync(process.execPath, [script, ...args], {
    cwd: root,
    stdio: ["ignore", "inherit", "inherit"],
    env: { ...process.env, CAPIA_P6_OUT: outDir },
  });
}

export function runAll() {
  mkdirSync(outDir, { recursive: true });
  const suites = {};
  for (const s of STEP_SUITES) {
    runNode(join(root, `tools/phase6-acceptance/${s}/run.mjs`));
    suites[s] = fromStepSummary(readJson(join(outDir, `${s}-summary.json`)));
  }
  for (const s of VALIDATOR_SUITES) {
    runNode(join(root, `tools/phase6-acceptance/${s}/validate.mjs`));
    suites[s] = fromValidatorSummary(readJson(join(outDir, `${s}-summary.json`)));
  }
  const perf = join(root, "tools/phase6-acceptance/performance/run.mjs");
  if (existsSync(perf)) {
    runNode(perf);
    suites.performance = fromPerformanceSummary(readJson(join(outDir, "performance-summary.json")));
  } else {
    suites.performance = fromPerformanceSummary(null);
  }
  const status = finalStatus(suites);
  const summary = {
    generated: new Date().toISOString(),
    status,
    suites,
    note: "Só `passed` é aprovação. `pending_external` = depende de humano/hardware/certificado real e NÃO foi fabricado; `not_available` = o passo não pôde rodar neste ambiente.",
  };
  writeFileSync(join(outDir, "summary.json"), JSON.stringify(summary, null, 2));
  return summary;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const summary = runAll();
  console.log(
    JSON.stringify(
      {
        status: summary.status,
        suites: Object.fromEntries(Object.entries(summary.suites).map(([k, v]) => [k, v.status])),
      },
      null,
      2,
    ),
  );
  const strict = process.argv.includes("--strict");
  process.exit(
    summary.status === "FAILURES" || (strict && summary.status !== "PHASE 6 COMPLETE") ? 1 : 0,
  );
}
