// Utilitários do pacote de aceitação da Fase 5: roda passos (cargo/pnpm), mede, grava JSON em
// `target/phase5-acceptance/<suite>-summary.json`. Nada aqui inventa resultado: passo que não rodou
// ou falhou nunca vira "passou".
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
export const out = join(root, "target/phase5-acceptance");
mkdirSync(out, { recursive: true });

export const cargo = (args) => ["cargo", ["test", "-q", ...args]];
export const pnpm = (args) => ["pnpm", args];

export function runSuite(suite, steps) {
  const results = [];
  for (const s of steps) {
    const t0 = Date.now();
    const [bin, args] = s.cmd;
    const r = spawnSync(bin, args, {
      cwd: root,
      encoding: "utf8",
      shell: process.platform === "win32",
      env: { ...process.env, CARGO_INCREMENTAL: "0" },
    });
    const passed = r.status === 0;
    results.push({
      requirements: s.req,
      command: `${bin} ${args.join(" ")}`,
      passed,
      seconds: Math.round((Date.now() - t0) / 100) / 10,
      tail: passed
        ? undefined
        : `${r.stdout ?? ""}${r.stderr ?? ""}`.split("\n").slice(-25).join("\n"),
    });
    console.log(`${passed ? "PASS" : "FAIL"}  ${s.req[0]}`);
  }
  const summary = {
    suite,
    generated: new Date().toISOString(),
    passed: results.every((r) => r.passed),
    steps: results,
  };
  writeFileSync(join(out, `${suite}-summary.json`), JSON.stringify(summary, null, 2));
  console.log(`\n${suite}-summary.json: ${summary.passed ? "ALL PASSED" : "FAILURES"}`);
  return summary;
}
