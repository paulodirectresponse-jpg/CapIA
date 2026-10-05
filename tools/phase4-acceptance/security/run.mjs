#!/usr/bin/env node
// Suíte de segurança da Fase 4 em um comando. Roda cada verificação, mapeia para o requisito
// (PHASE4_PROVIDERS_SECURITY §28) e grava `target/phase4-acceptance/security-summary.json`.
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const out = join(root, "target/phase4-acceptance");
mkdirSync(out, { recursive: true });

const cargo = (args) => ["cargo", ["test", "-q", ...args]];
const STEPS = [
  {
    req: [
      "secret canary (core)",
      "redirect credential leak",
      "gigantic response",
      "SSRF",
      "host binding",
    ],
    cmd: cargo(["-p", "capia-ai", "--test", "security"]),
  },
  {
    req: ["invalid TLS behavior", "malicious custom header"],
    cmd: cargo(["-p", "capia-ai", "--test", "security", "--", "tls", "headers"]),
  },
  {
    req: ["malformed JSON", "streaming never ends", "auth/rate-limit errors", "cancel"],
    cmd: cargo(["-p", "capia-ai", "--test", "contract"]),
  },
  {
    req: ["fallback/privacy/budget/AI Off semantics"],
    cmd: cargo(["-p", "capia-ai", "--test", "dispatcher"]),
  },
  {
    req: ["tool registry: forbidden tools, gate order, bounded output, operation ids"],
    cmd: cargo(["-p", "capia-ai", "--lib"]),
  },
  {
    req: [
      "secret canary (service: logs, project, IPC, AppDb, diagnostics)",
      "write-only credentials",
      "provider swap",
      "AI Off",
      "cancel aborts HTTP",
    ],
    cmd: cargo(["-p", "capia-intelligence", "--test", "service"]),
  },
  {
    req: [
      "provider returns unregistered tool",
      "invalid args",
      "prompt injection (transcript)",
      "cross-project tool access",
      "permission escalation",
      "cancelled response applying late tool",
      "replayed operation id",
    ],
    cmd: cargo(["-p", "capia-intelligence", "--test", "assistant"]),
  },
  {
    req: ["prompt injection (PDF/DOCX/TXT)", "fabricated sources"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "demand_flow", "--test", "demand_eval"]),
  },
  { req: ["crash output redacted"], cmd: cargo(["-p", "capia-secrets"]) },
  {
    req: ["UI: write-only key, no storage, no provider URLs, no direct writes"],
    cmd: [
      "pnpm",
      [
        "--filter",
        "@capia/editor-ui",
        "exec",
        "vitest",
        "run",
        "src/architecture.test.ts",
        "src/store/aiController.test.ts",
        "src/components/AiPanel.test.tsx",
      ],
    ],
  },
];

const results = [];
for (const s of STEPS) {
  const t0 = Date.now();
  const [bin, args] = s.cmd;
  const r = spawnSync(bin, args, {
    cwd: root,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  results.push({
    requirements: s.req,
    command: `${bin} ${args.join(" ")}`,
    passed: r.status === 0,
    seconds: Math.round((Date.now() - t0) / 100) / 10,
    tail:
      r.status === 0
        ? undefined
        : `${r.stdout ?? ""}${r.stderr ?? ""}`.split("\n").slice(-25).join("\n"),
  });
  console.log(`${r.status === 0 ? "PASS" : "FAIL"}  ${s.req[0]}`);
}
const summary = {
  suite: "phase4-security",
  generated: new Date().toISOString(),
  passed: results.every((r) => r.passed),
  steps: results,
};
writeFileSync(join(out, "security-summary.json"), JSON.stringify(summary, null, 2));
console.log(`\nsecurity-summary.json: ${summary.passed ? "ALL PASSED" : "FAILURES"}`);
process.exit(summary.passed ? 0 : 1);
