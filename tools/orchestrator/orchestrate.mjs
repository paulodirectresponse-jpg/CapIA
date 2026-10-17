#!/usr/bin/env node
// Orquestrador Claude (executor) <-> Codex (auditor) para o CapIA.
//   node tools/orchestrator/orchestrate.mjs <init|doctor|dry-run|selftest|run|status|stop|reset|confirm-goal> [flags]
// Sem dependências; Node >= 20; Windows/macOS/Linux.
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import { createInterface } from "node:readline/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildAdapters } from "./lib/agents.mjs";
import { doctor } from "./lib/doctor.mjs";
import { EXIT_CODES, realGit, run } from "./lib/engine.mjs";
import { runAllScenarios } from "./lib/scenarios.mjs";
import { selftest } from "./lib/selftest.mjs";
import { Store } from "./lib/state.mjs";
import { readJson, sha256 } from "./lib/util.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const command = argv[0] && !argv[0].startsWith("--") ? argv[0] : "help";
const flag = (name) => argv.includes(`--${name}`);
const option = (name) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : undefined;
};

function repoRoot() {
  try {
    return execFileSync("git", ["rev-parse", "--show-toplevel"], {
      cwd: HERE,
      encoding: "utf8",
    }).trim();
  } catch {
    return process.cwd();
  }
}

const cfgPath = option("config") ?? join(HERE, "orchestrator.config.json");
const cfg = readJson(cfgPath);
if (!cfg) {
  console.error(`config ilegível: ${cfgPath}`);
  process.exit(1);
}
const root = repoRoot();
const runtimeDir = join(root, cfg.runtimeDir);
const goalText = () => readFileSync(join(HERE, cfg.goalFile), "utf8");
const goalHash = () => sha256(goalText());
const line = (s = "") => console.log(s);

function ensureGitignore() {
  const file = join(root, ".gitignore");
  const entry = `${cfg.runtimeDir}/`;
  const current = existsSync(file) ? readFileSync(file, "utf8") : "";
  if (!current.split(/\r?\n/).includes(entry)) {
    appendFileSync(
      file,
      `${current.endsWith("\n") || current === "" ? "" : "\n"}# orquestrador: estado e logs locais\n${entry}\n`,
    );
    return true;
  }
  return false;
}

function checkFiles() {
  const required = [cfg.goalFile, cfg.roles.executor.promptFile, cfg.roles.auditor.promptFile];
  return required.filter((f) => !existsSync(join(HERE, f)));
}

async function cmdInit() {
  const missing = checkFiles();
  if (missing.length) {
    console.error(
      `arquivos ausentes em tools/orchestrator: ${missing.join(", ")} (restaure com: git checkout -- tools/orchestrator)`,
    );
    return 1;
  }
  mkdirSync(runtimeDir, { recursive: true });
  const store = new Store(runtimeDir);
  const created = !store.exists();
  if (created) store.create(goalHash());
  const ignored = ensureGitignore();
  line(`[init] runtime: ${runtimeDir}`);
  line(
    `[init] STATE.json: ${created ? "criado" : "já existia (preservado)"} — status=${store.state.status} iteração=${store.state.iteration}/${store.state.phase}`,
  );
  line(`[init] MASTER_GOAL.md, prompts e config: ok (tools/orchestrator)`);
  line(
    `[init] .gitignore: ${ignored ? "adicionada a entrada " + cfg.runtimeDir + "/" : "já continha a entrada"}`,
  );
  line(`[init] logs: ${join(runtimeDir, "logs", "orchestrator.jsonl")}`);
  return 0;
}

async function cmdDoctor(probeAuth = true) {
  const r = await doctor({ root, cfg, probeAuth });
  for (const c of r.checks) {
    line(`[${c.ok ? "OK  " : c.severity === "fatal" ? "FAIL" : "WARN"}] ${c.name} — ${c.detail}`);
    if (!c.ok && c.fix) line(`       como consertar: ${c.fix}`);
  }
  line(r.ok ? "[doctor] ambiente pronto" : "[doctor] há pendências fatais acima");
  return r.ok ? 0 : 1;
}

async function cmdDryRun() {
  line(
    "[dry-run] máquina de estados REAL + agentes simulados + falhas injetadas (nenhum CLI é chamado)",
  );
  const results = await runAllScenarios(cfg);
  for (const r of results) line(`[${r.pass ? "PASS" : "FAIL"}] ${r.name}\n         ${r.detail}`);
  const failed = results.filter((r) => !r.pass).length;
  line(`[dry-run] ${results.length - failed}/${results.length} cenários passaram`);
  return failed ? 1 : 0;
}

async function cmdSelftest() {
  const r = await selftest({ cfg, root, mockCodex: flag("mock-codex") });
  for (const l of r.legs) {
    line(
      `[${l.status}] ${l.name} (${l.agent}${l.ms ? `, ${(l.ms / 1000).toFixed(1)}s` : ""}) — ${l.detail}`,
    );
    if (l.fix) line(`       como consertar: ${l.fix}`);
  }
  line(
    `[selftest] nonce=${r.nonce} resultado=${r.overall}${flag("mock-codex") ? " (perna do Codex SIMULADA: não prova o Codex real)" : ""}`,
  );
  return r.overall === "PASS" ? 0 : r.overall === "FAIL" ? 1 : 2;
}

function cmdStatus() {
  const store = new Store(runtimeDir);
  if (!store.exists()) {
    line("sem STATE.json — rode: node tools/orchestrator/orchestrate.mjs init");
    return 1;
  }
  const s = store.state;
  const L = cfg.limits;
  line(`run ${s.runId}: status=${s.status}${s.stopReason ? ` (${s.stopReason})` : ""}`);
  line(`iteração ${s.iteration}/${L.maxIterations}, fase=${s.phase}`);
  line(
    `claude ${s.counters.claudeCalls}/${L.maxClaudeCalls} chamadas · codex ${s.counters.codexCalls}/${L.maxCodexCalls} · custo US$ ${s.counters.costUsd.toFixed(2)}/${L.maxCostUsd}`,
  );
  line(
    `tempo ${(s.counters.wallMs / 60000).toFixed(1)}/${L.maxWallClockMinutes} min · retries ${s.counters.retries} · recuperações ${s.counters.recoveries} · falhas seguidas ${s.counters.consecutiveFailures}/${L.maxConsecutiveFailures} · sem progresso ${s.counters.noProgress}/${L.maxNoProgress}`,
  );
  if (s.humanGate)
    line(
      `GATE HUMANO: ${s.humanGate}\n  depois de fazer: escreva o resultado em ${join(runtimeDir, "human-response.md")} e rode: run --human-done`,
    );
  if (s.lastError) line(`último erro: [${s.lastError.kind}] ${s.lastError.message}`);
  return 0;
}

async function confirmGoal(store) {
  const hash = goalHash();
  if (store.state.goalConfirmedHash === hash) return true;
  line("\n=== MASTER_GOAL.md (resumo) ===");
  line(goalText().split("\n").slice(0, 22).join("\n"));
  line("\n=== Limites (hard stops) ===");
  line(JSON.stringify(cfg.limits));
  line(
    `Branches protegidas: ${cfg.protectedBranches.join(", ")} · agentes: executor=${cfg.roles.executor.agent}, auditor=${cfg.roles.auditor.agent}`,
  );
  let ok = flag("yes");
  if (!ok && process.stdin.isTTY) {
    const rl = createInterface({ input: process.stdin, output: process.stdout });
    ok =
      (
        await rl.question(
          '\nO agente vai agir sozinho neste repositório. Digite "SIM" para confirmar o objetivo: ',
        )
      )
        .trim()
        .toUpperCase() === "SIM";
    rl.close();
  }
  if (!ok) {
    line(
      "objetivo NÃO confirmado — nada foi executado. Confirme com --yes ou rode em um terminal interativo.",
    );
    return false;
  }
  store.state.goalConfirmedHash = hash;
  store.state.goalHash = hash;
  store.save();
  store.log("goal.confirmed", { hash });
  return true;
}

async function cmdRun() {
  if ((await cmdInit()) !== 0) return 1;
  const store = new Store(runtimeDir);
  if (flag("fresh")) {
    const archived = store.archive();
    store.create(goalHash());
    line(`[run] estado anterior arquivado em ${archived ?? "(não havia)"}`);
  }
  if (store.state.status === "done") {
    line("[run] a última execução já terminou com sucesso (done). Use --fresh para recomeçar.");
    return 0;
  }
  if (!(await confirmGoal(store))) return 1;
  if (!flag("skip-doctor")) {
    const r = await doctor({ root, cfg, probeAuth: false });
    const fatal = r.checks.filter((c) => !c.ok && c.severity === "fatal");
    if (fatal.length) {
      for (const c of fatal)
        line(`[FAIL] ${c.name} — ${c.detail}\n       como consertar: ${c.fix}`);
      return 1;
    }
  }
  for (const sig of ["SIGINT", "SIGTERM"])
    process.on(sig, () => {
      store.log("signal", { sig });
      line(
        `\n[run] ${sig}: estado preservado (atômico); rode novamente para retomar de onde parou.`,
      );
      process.exit(130);
    });
  line(
    `[run] iniciando — iteração ${store.state.iteration}, fase ${store.state.phase}. Logs: ${store.logFile}`,
  );
  const result = await run({
    root,
    cfg,
    store,
    adapters: buildAdapters(cfg),
    git: realGit(root),
    humanDone: flag("human-done"),
  });
  line(`\n[run] FIM: status=${result.status}${result.reason ? ` — ${result.reason}` : ""}`);
  cmdStatus();
  return result.exit;
}

function cmdStop() {
  mkdirSync(runtimeDir, { recursive: true });
  writeFileSync(
    join(runtimeDir, cfg.hardStops.stopFile),
    `stop requested ${new Date().toISOString()}\n`,
  );
  line(
    `[stop] criado ${join(runtimeDir, cfg.hardStops.stopFile)} — o laço para antes do próximo passo.`,
  );
  return 0;
}

function cmdReset() {
  const store = new Store(runtimeDir);
  const archived = store.archive();
  const stop = join(runtimeDir, cfg.hardStops.stopFile);
  if (existsSync(stop)) rmSync(stop);
  line(`[reset] estado arquivado em ${archived ?? "(não havia)"}; STOP removido. Rode init/run.`);
  return 0;
}

const HELP = `uso: node tools/orchestrator/orchestrate.mjs <comando>
  init           cria runtime, STATE.json, valida arquivos, ajusta .gitignore (idempotente)
  doctor         verifica Claude CLI, Codex CLI, login, git e branch
  dry-run        valida laço/retry/recuperação/hard stops com agentes simulados (sem CLIs)
  selftest       cadeia real Claude -> Codex -> Claude  (--mock-codex simula só a perna do Codex)
  run            inicia/retoma o laço (--yes confirma o objetivo sem prompt, --human-done, --fresh)
  status | stop | reset | confirm-goal`;

const commands = {
  init: cmdInit,
  doctor: () => cmdDoctor(!flag("no-auth")),
  "dry-run": cmdDryRun,
  selftest: cmdSelftest,
  run: cmdRun,
  status: async () => cmdStatus(),
  stop: async () => cmdStop(),
  reset: async () => cmdReset(),
  "confirm-goal": async () => {
    await cmdInit();
    const store = new Store(runtimeDir);
    store.state.goalConfirmedHash = goalHash();
    store.save();
    line("[confirm-goal] objetivo confirmado");
    return 0;
  },
};

const fn = commands[command];
if (!fn) {
  line(HELP);
  process.exit(command === "help" ? 0 : 1);
}
process.exit(await fn());
