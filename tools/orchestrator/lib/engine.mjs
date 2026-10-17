// Máquina de estados do laço executor -> auditor com retry, recuperação e hard stops.
import { existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { AgentError } from "./agents.mjs";
import { evaluateLimits } from "./limits.mjs";
import { nowIso, redact, sha256, sleep } from "./util.mjs";

export class SimulatedCrash extends Error {}

const HERE = dirname(fileURLToPath(import.meta.url));
const ORCH_DIR = join(HERE, "..");

export const EXIT_CODES = {
  done: 0,
  paused_human_gate: 10,
  paused_auth: 11,
  blocked: 11,
  stopped: 12,
  hard_stop: 20,
  failed: 30,
};

export function realGit(root) {
  const git = (...args) => {
    try {
      return execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
    } catch {
      return "";
    }
  };
  return {
    branch: () => git("rev-parse", "--abbrev-ref", "HEAD"),
    head: () => git("rev-parse", "--short", "HEAD"),
    status: () => git("status", "--short").split("\n").slice(0, 25).join("\n"),
    clean: () => git("status", "--porcelain") === "",
  };
}

function pidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === "EPERM";
  }
}

function acquireLock(dir) {
  const file = join(dir, "lock.json");
  if (existsSync(file)) {
    try {
      const lock = JSON.parse(readFileSync(file, "utf8"));
      if (lock.pid !== process.pid && pidAlive(lock.pid))
        throw new Error(`outra instância do orquestrador está ativa (pid ${lock.pid})`);
    } catch (e) {
      if (String(e.message).startsWith("outra instância")) throw e;
    }
  }
  writeFileSync(file, JSON.stringify({ pid: process.pid, startedAt: nowIso() }));
  return () => {
    try {
      unlinkSync(file);
    } catch {}
  };
}

/** Último objeto JSON com a chave "verdict" (a saída do auditor pode vir cercada de texto). */
export function extractVerdictJson(text) {
  for (let start = text.indexOf("{"); start !== -1; start = text.indexOf("{", start + 1)) {
    for (let end = text.lastIndexOf("}"); end > start; end = text.lastIndexOf("}", end - 1)) {
      try {
        const value = JSON.parse(text.slice(start, end + 1));
        if (value && typeof value === "object" && "verdict" in value) return value;
      } catch {}
    }
  }
  return null;
}

function parseAuditor(text) {
  const v = extractVerdictJson(text);
  if (!v)
    throw new AgentError("saída do auditor não contém JSON com `verdict`", {
      kind: "INVALID_OUTPUT",
      transient: true,
    });
  if (!["PASS", "FAIL", "BLOCKED"].includes(v.verdict))
    throw new AgentError(`verdict inválido: ${String(v.verdict)}`, {
      kind: "INVALID_OUTPUT",
      transient: true,
    });
  const findings = Array.isArray(v.findings) ? v.findings : [];
  if (findings.some((f) => !f || typeof f.issue !== "string"))
    throw new AgentError("cada finding precisa de `issue` (string)", {
      kind: "INVALID_OUTPUT",
      transient: true,
    });
  if (
    v.verdict === "PASS" &&
    findings.some((f) => ["blocker", "high"].includes(String(f.severity).toLowerCase()))
  )
    throw new AgentError("PASS é incompatível com finding blocker/high", {
      kind: "INVALID_OUTPUT",
      transient: true,
    });
  return { verdict: v.verdict, findings, notes: v.notes ?? "", gates: v.gates ?? [] };
}

function parseExecutor(text, marker) {
  const gate = text.split("\n").find((line) => line.trim().startsWith(marker));
  let result = null;
  const line = text
    .split("\n")
    .reverse()
    .find((l) => l.trim().startsWith("EXECUTOR_RESULT:"));
  if (line) {
    try {
      result = JSON.parse(line.trim().slice("EXECUTOR_RESULT:".length));
    } catch {}
  }
  return {
    humanGate: gate ? gate.trim() : null,
    result,
    summary: (result?.summary ?? text.slice(-600)).toString(),
  };
}

function fill(template, vars) {
  return template.replace(/\{\{(\w+)\}\}/g, (_, k) => String(vars[k] ?? ""));
}

export async function run(ctx) {
  const { root, cfg, store, adapters, git, hooks = {}, sleepFn = sleep, humanDone = false } = ctx;
  const st = store.state;
  const started = Date.now();
  const stopFile = join(store.dir, cfg.hardStops.stopFile);
  const release = acquireLock(store.dir);

  const finish = (status, reason) => {
    st.status = status;
    st.stopReason = reason ?? null;
    st.counters.wallMs += Date.now() - started;
    store.save();
    store.log("finish", { status, reason });
    return { status, reason: reason ?? null, exit: EXIT_CODES[status] ?? 30 };
  };

  const promptFor = (phase, extra = "") => {
    const template = readFileSync(join(ORCH_DIR, cfg.roles[phase].promptFile), "utf8");
    const goal = readFileSync(join(ORCH_DIR, cfg.goalFile), "utf8");
    const humanFile = join(store.dir, "human-response.md");
    return (
      fill(template, {
        GOAL: goal,
        ITERATION: st.iteration,
        MAX_ITERATIONS: cfg.limits.maxIterations,
        GIT_BRANCH: git.branch(),
        GIT_HEAD: git.head(),
        GIT_STATUS: git.status(),
        PREVIOUS_FINDINGS: JSON.stringify(st.lastFindings, null, 2),
        LAST_EXECUTOR_SUMMARY: st.lastExecutorSummary ?? "(primeira iteração)",
        HUMAN_RESPONSE: existsSync(humanFile) ? readFileSync(humanFile, "utf8") : "(nenhuma)",
        HUMAN_GATE_MARKER: cfg.hardStops.humanGateMarker,
      }) + extra
    );
  };

  async function step(phase) {
    const key = `${st.iteration}:${phase}`;
    const agent = cfg.roles[phase].agent;
    const dir = store.runDir(st.iteration);
    const artifact = join(dir, `${phase}.out.txt`);
    if (st.steps[key]?.done) {
      store.log("step.skip", { key });
      return { text: readFileSync(artifact, "utf8") };
    }
    let extra = "";
    for (let attempt = 1; ; attempt++) {
      const lim = evaluateLimits({
        limits: cfg.limits,
        state: st,
        sessionWallMs: Date.now() - started,
      });
      if (lim) return { terminal: finish("hard_stop", lim) };
      const prompt = promptFor(phase, extra);
      writeFileSync(join(dir, `${phase}.prompt.md`), redact(prompt));
      try {
        store.log("call.start", { key, agent, attempt });
        st.counters[agent === "claude" ? "claudeCalls" : "codexCalls"]++;
        const res = await adapters[phase]({
          prompt,
          cwd: root,
          timeoutMs: cfg.limits.callTimeoutMinutes * 60_000,
          iteration: st.iteration,
          attempt,
        });
        st.counters.costUsd += Number(res.costUsd ?? 0);
        writeFileSync(artifact, redact(res.text));
        if (phase === "auditor") parseAuditor(res.text); // valida antes de marcar como concluído
        st.steps[key] = {
          done: true,
          agent,
          attempts: attempt,
          costUsd: res.costUsd ?? 0,
          at: nowIso(),
        };
        st.counters.consecutiveFailures = 0;
        st.lastError = null;
        store.save();
        store.log("call.done", { key, agent, attempt, costUsd: res.costUsd ?? 0 });
        return { text: res.text };
      } catch (e) {
        if (e instanceof SimulatedCrash) throw e;
        const kind = e.kind ?? "FAILED";
        st.lastError = {
          key,
          kind,
          message: redact(String(e.message)).slice(0, 400),
          at: nowIso(),
        };
        store.log("call.error", { key, agent, attempt, kind, message: st.lastError.message });
        if (kind === "AUTH" || kind === "CLI_MISSING") {
          store.save();
          return { terminal: finish("paused_auth", `${agent}: ${st.lastError.message}`) };
        }
        st.counters.consecutiveFailures++;
        st.counters.retries++;
        store.save();
        if (attempt >= cfg.retry.maxAttempts)
          return { terminal: finish("failed", `retries_exhausted:${key}:${kind}`) };
        if (kind === "INVALID_OUTPUT")
          extra = `\n\n---\nATENÇÃO: sua resposta anterior foi rejeitada (${st.lastError.message}). Responda de novo seguindo EXATAMENTE o formato de saída pedido.`;
        const base = Math.min(
          cfg.retry.maxDelaySeconds,
          cfg.retry.baseDelaySeconds * 2 ** (attempt - 1),
        );
        const jitter = 1 + (Math.random() * 2 - 1) * cfg.retry.jitter;
        await sleepFn(Math.round(base * jitter * 1000));
      }
    }
  }

  try {
    const branch = git.branch();
    if (cfg.protectedBranches.includes(branch))
      return finish("hard_stop", `PROTECTED_BRANCH:${branch}`);
    if (cfg.requireCleanTree && !git.clean()) return finish("hard_stop", "DIRTY_TREE");
    if (st.status === "paused_human_gate") {
      if (!humanDone) {
        store.log("refused", { reason: "aguardando ação humana (use --human-done)" });
        return { status: st.status, reason: st.humanGate, exit: EXIT_CODES.paused_human_gate };
      }
      st.humanGate = null;
      // o passo do executor é refeito com a resposta humana no prompt
      delete st.steps[`${st.iteration}:executor`];
    }
    if (st.status === "running") {
      st.counters.recoveries++;
      store.log("recovered", { iteration: st.iteration, phase: st.phase });
    }
    st.status = "running";
    st.stopReason = null;
    store.save();
    store.log("start", { iteration: st.iteration, phase: st.phase });

    for (;;) {
      if (existsSync(stopFile)) return finish("stopped", "arquivo STOP presente");
      const lim = evaluateLimits({
        limits: cfg.limits,
        state: st,
        sessionWallMs: Date.now() - started,
      });
      if (lim) return finish("hard_stop", lim);

      if (st.phase === "executor") {
        const out = await step("executor");
        if (out.terminal) return out.terminal;
        const parsed = parseExecutor(out.text, cfg.hardStops.humanGateMarker);
        st.lastExecutorSummary = redact(parsed.summary).slice(0, 1500);
        if (parsed.humanGate) {
          st.humanGate = redact(parsed.humanGate);
          delete st.steps[`${st.iteration}:executor`];
          return finish("paused_human_gate", st.humanGate);
        }
        if (parsed.result?.status === "blocked") return finish("blocked", st.lastExecutorSummary);
        st.phase = "auditor";
        store.save();
        hooks.afterStep?.({ phase: "executor", state: st });
      } else {
        const out = await step("auditor");
        if (out.terminal) return out.terminal;
        const verdict = parseAuditor(out.text);
        store.log("verdict", {
          iteration: st.iteration,
          verdict: verdict.verdict,
          findings: verdict.findings.length,
        });
        if (verdict.verdict === "PASS") return finish("done", "auditor PASS");
        if (verdict.verdict === "BLOCKED")
          return finish("blocked", verdict.notes || "auditor BLOCKED");
        const hash = sha256(
          JSON.stringify(verdict.findings.map((f) => `${f.file ?? ""}|${f.issue}`).sort()),
        );
        st.counters.noProgress = hash === st.lastFindingsHash ? st.counters.noProgress + 1 : 0;
        st.lastFindingsHash = hash;
        st.lastFindings = verdict.findings;
        st.iteration += 1;
        st.phase = "executor";
        store.save();
        hooks.afterStep?.({ phase: "auditor", state: st });
      }
    }
  } finally {
    release();
  }
}
