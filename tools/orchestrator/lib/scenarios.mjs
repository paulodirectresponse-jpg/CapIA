// Cenários determinísticos do dry-run: a MESMA máquina de estados, com agentes simulados e falhas injetadas.
// Nenhum CLI real é chamado. Usado por `dry-run` e pelos testes (node --test).
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AgentError } from "./agents.mjs";
import { SimulatedCrash, run } from "./engine.mjs";
import { Store } from "./state.mjs";
import { sha256 } from "./util.mjs";

const verdict = (v, findings = [], notes = "") => JSON.stringify({ verdict: v, findings, notes });
const finding = (issue, file = "src/a.ts") => ({
  severity: "medium",
  file,
  issue,
  evidence: "mock",
  required_fix: "mock",
});

function testConfig(base, over = {}) {
  return {
    ...base,
    retry: { ...base.retry, baseDelaySeconds: 0, maxDelaySeconds: 0, jitter: 0 },
    limits: { ...base.limits, ...(over.limits ?? {}) },
    protectedBranches: base.protectedBranches,
  };
}

async function exec({
  baseCfg,
  over,
  exec: executor,
  audit,
  git,
  hooks,
  prep,
  reuseDir,
  humanDone,
}) {
  const dir = reuseDir ?? mkdtempSync(join(tmpdir(), "capia-orch-"));
  const store = new Store(dir);
  if (!store.exists()) store.create(sha256("goal"));
  prep?.(dir, store);
  const calls = { executor: 0, auditor: 0, prompts: [] };
  const adapters = {
    executor: async (c) => {
      calls.executor++;
      calls.prompts.push(["executor", c.prompt]);
      return executor(c, calls);
    },
    auditor: async (c) => {
      calls.auditor++;
      calls.prompts.push(["auditor", c.prompt]);
      return audit(c, calls);
    },
  };
  const result = await run({
    root: process.cwd(),
    cfg: testConfig(baseCfg, over),
    store,
    adapters,
    git: git ?? {
      branch: () => "feature/dry-run",
      head: () => "abc1234",
      status: () => "",
      clean: () => true,
    },
    hooks,
    sleepFn: async () => {},
    humanDone,
  });
  return { dir, store, result, calls };
}

const ok = (text, costUsd = 0.01) => ({ text, costUsd });

export const SCENARIOS = [
  {
    name: "feliz: FAIL na 1ª auditoria, PASS na 2ª",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        exec: async () => ok('feito\nEXECUTOR_RESULT: {"status":"done","summary":"ok"}'),
        audit: async (c, calls) =>
          ok(calls.auditor === 1 ? verdict("FAIL", [finding("falta teste")]) : verdict("PASS")),
      });
      return {
        pass:
          r.result.status === "done" &&
          r.store.state.iteration === 2 &&
          r.calls.executor === 2 &&
          r.calls.auditor === 2,
        detail: `status=${r.result.status} iter=${r.store.state.iteration} execs=${r.calls.executor} audits=${r.calls.auditor}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "retry: falha transitória no executor é repetida e o laço segue",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        exec: async (_c, calls) => {
          if (calls.executor === 1)
            throw new AgentError("429 rate limit", { kind: "TRANSIENT", transient: true });
          return ok("feito");
        },
        audit: async () => ok(verdict("PASS")),
      });
      return {
        pass:
          r.result.status === "done" &&
          r.store.state.counters.retries === 1 &&
          r.calls.executor === 2,
        detail: `status=${r.result.status} retries=${r.store.state.counters.retries} execs=${r.calls.executor}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "recuperação: crash depois do executor; resume NÃO refaz o executor",
    async run(baseCfg) {
      const dir = mkdtempSync(join(tmpdir(), "capia-orch-"));
      let execCalls = 0;
      const executor = async () => (execCalls++, ok("feito"));
      const audit = async () => ok(verdict("PASS"));
      let crashed = false;
      try {
        await exec({
          baseCfg,
          reuseDir: dir,
          exec: executor,
          audit,
          hooks: {
            afterStep: ({ phase }) => {
              if (phase === "executor") throw new SimulatedCrash("kill -9");
            },
          },
        });
      } catch (e) {
        crashed = e instanceof SimulatedCrash;
      }
      const mid = new Store(dir).state; // o que um processo novo encontraria em disco
      const r2 = await exec({ baseCfg, reuseDir: dir, exec: executor, audit });
      return {
        pass:
          crashed &&
          mid.status === "running" &&
          mid.phase === "auditor" &&
          r2.result.status === "done" &&
          execCalls === 1 &&
          r2.store.state.counters.recoveries === 1,
        detail: `crash=${crashed} emDisco=${mid.status}/${mid.phase} final=${r2.result.status} execCalls=${execCalls} recoveries=${r2.store.state.counters.recoveries}`,
        dir,
      };
    },
  },
  {
    name: "hard stop: maxIterations",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        over: { limits: { maxIterations: 3, maxNoProgress: 99 } },
        exec: async () => ok("feito"),
        audit: async (c) => ok(verdict("FAIL", [finding(`problema ${c.iteration}`)])),
      });
      return {
        pass: r.result.status === "hard_stop" && /maxIterations/.test(r.result.reason),
        detail: `${r.result.status}: ${r.result.reason}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "hard stop: sem progresso (mesmos achados)",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        over: { limits: { maxNoProgress: 2, maxIterations: 10 } },
        exec: async () => ok("feito"),
        audit: async () => ok(verdict("FAIL", [finding("mesmo problema")])),
      });
      return {
        pass: r.result.status === "hard_stop" && /no-progress/.test(r.result.reason),
        detail: `${r.result.status}: ${r.result.reason}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "hard stop: custo",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        over: { limits: { maxCostUsd: 5 } },
        exec: async () => ok("feito", 6),
        audit: async () => ok(verdict("PASS")),
      });
      return {
        pass:
          r.result.status === "hard_stop" &&
          /maxCostUsd/.test(r.result.reason) &&
          r.calls.auditor === 0,
        detail: `${r.result.status}: ${r.result.reason} audits=${r.calls.auditor}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "hard stop: branch protegida (nenhum agente é chamado)",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        git: { branch: () => "main", head: () => "x", status: () => "", clean: () => true },
        exec: async () => ok("x"),
        audit: async () => ok(verdict("PASS")),
      });
      return {
        pass:
          r.result.status === "hard_stop" &&
          /PROTECTED_BRANCH/.test(r.result.reason) &&
          r.calls.executor === 0,
        detail: `${r.result.status}: ${r.result.reason}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "hard stop: arquivo STOP",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        prep: (dir) => writeFileSync(join(dir, baseCfg.hardStops.stopFile), "stop"),
        exec: async () => ok("x"),
        audit: async () => ok(verdict("PASS")),
      });
      return {
        pass: r.result.status === "stopped" && r.calls.executor === 0,
        detail: `${r.result.status}: ${r.result.reason}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "gate humano: pausa, recusa retomar sem --human-done, retoma com resposta",
    async run(baseCfg) {
      const marker = baseCfg.hardStops.humanGateMarker;
      let total = 0; // contador compartilhado entre as 3 execuções do mesmo estado
      const executor = async () =>
        ok(
          ++total === 1
            ? `${marker} rode run-windows.ps1 na máquina física`
            : "feito com a resposta humana",
        );
      const audit = async () => ok(verdict("PASS"));
      const a = await exec({ baseCfg, exec: executor, audit });
      const b = await exec({ baseCfg, reuseDir: a.dir, exec: executor, audit });
      const c = await exec({
        baseCfg,
        reuseDir: a.dir,
        exec: executor,
        audit,
        humanDone: true,
        prep: (d) => writeFileSync(join(d, "human-response.md"), "PASS no AMD"),
      });
      return {
        pass:
          a.result.status === "paused_human_gate" &&
          b.result.status === "paused_human_gate" &&
          c.result.status === "done",
        detail: `1ª=${a.result.status} sem flag=${b.result.status} com --human-done=${c.result.status}`,
        dir: a.dir,
      };
    },
  },
  {
    name: "auth: falha de login não é repetida (pausa)",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        exec: async () => {
          throw new AgentError("Not logged in · Please run /login", { kind: "AUTH" });
        },
        audit: async () => ok(verdict("PASS")),
      });
      return {
        pass: r.result.status === "paused_auth" && r.calls.executor === 1,
        detail: `${r.result.status} execs=${r.calls.executor}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "retries esgotados: failed após maxAttempts",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        over: { limits: { maxConsecutiveFailures: 99 } },
        exec: async () => {
          throw new AgentError("503 overloaded", { kind: "TRANSIENT", transient: true });
        },
        audit: async () => ok(verdict("PASS")),
      });
      return {
        pass: r.result.status === "failed" && r.calls.executor === baseCfg.retry.maxAttempts,
        detail: `${r.result.status}: ${r.result.reason} execs=${r.calls.executor}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "auditor devolve lixo uma vez: reparo com aviso no prompt",
    async run(baseCfg) {
      const r = await exec({
        baseCfg,
        exec: async () => ok("feito"),
        audit: async (_c, calls) =>
          ok(calls.auditor === 1 ? "tudo parece ótimo!" : verdict("PASS")),
      });
      const second = r.calls.prompts.filter(([k]) => k === "auditor")[1]?.[1] ?? "";
      return {
        pass: r.result.status === "done" && /rejeitada/.test(second),
        detail: `${r.result.status} audits=${r.calls.auditor} repairHint=${/rejeitada/.test(second)}`,
        dir: r.dir,
      };
    },
  },
  {
    name: "segredos: erro com chave nunca chega ao log",
    async run(baseCfg) {
      const secret = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX";
      const r = await exec({
        baseCfg,
        exec: async (_c, calls) => {
          if (calls.executor === 1)
            throw new AgentError(`503 temporarily unavailable key=${secret}`, {
              kind: "TRANSIENT",
              transient: true,
            });
          return ok("feito");
        },
        audit: async () => ok(verdict("PASS")),
      });
      const log = readFileSync(join(r.dir, "logs", "orchestrator.jsonl"), "utf8");
      const state = readFileSync(join(r.dir, "STATE.json"), "utf8");
      return {
        pass: !log.includes(secret) && !state.includes(secret) && log.includes("[REDACTED]"),
        detail: `vazou=${log.includes(secret) || state.includes(secret)} redigido=${log.includes("[REDACTED]")}`,
        dir: r.dir,
      };
    },
  },
];

export async function runAllScenarios(baseCfg, { keep = false } = {}) {
  const results = [];
  for (const s of SCENARIOS) {
    let r;
    try {
      r = await s.run(baseCfg);
    } catch (e) {
      r = { pass: false, detail: `exceção: ${e.stack ?? e}` };
    }
    results.push({ name: s.name, pass: r.pass, detail: r.detail });
    if (!keep && r.dir && existsSync(r.dir)) rmSync(r.dir, { recursive: true, force: true });
  }
  return results;
}
