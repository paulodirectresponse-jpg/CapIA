// STATE.json (fonte única do progresso) + log JSONL. Toda escrita é atômica.
import { appendFileSync, mkdirSync, existsSync, renameSync } from "node:fs";
import { join } from "node:path";
import { atomicWriteJson, nowIso, readJson, redact } from "./util.mjs";

export const STATE_SCHEMA_VERSION = 1;

export function newState({ goalHash }) {
  return {
    schemaVersion: STATE_SCHEMA_VERSION,
    runId: `run-${Date.now().toString(36)}`,
    status: "idle", // idle | running | done | paused_human_gate | paused_auth | blocked | hard_stop | failed | stopped
    stopReason: null,
    goalHash,
    goalConfirmedHash: null,
    iteration: 1,
    phase: "executor", // executor | auditor
    steps: {}, // "<iter>:<phase>" -> { done, artifact, costUsd, attempts }
    lastFindings: [],
    lastFindingsHash: null,
    lastExecutorSummary: null,
    counters: {
      claudeCalls: 0,
      codexCalls: 0,
      costUsd: 0,
      wallMs: 0,
      consecutiveFailures: 0,
      noProgress: 0,
      retries: 0,
      recoveries: 0,
    },
    humanGate: null,
    lastError: null,
    createdAt: nowIso(),
    updatedAt: nowIso(),
  };
}

export class Store {
  constructor(dir) {
    this.dir = dir;
    this.stateFile = join(dir, "STATE.json");
    this.logFile = join(dir, "logs", "orchestrator.jsonl");
    mkdirSync(join(dir, "logs"), { recursive: true });
    mkdirSync(join(dir, "runs"), { recursive: true });
    this.state = readJson(this.stateFile);
  }
  exists() {
    return this.state !== null;
  }
  create(goalHash) {
    this.state = newState({ goalHash });
    this.save();
    return this.state;
  }
  save() {
    this.state.updatedAt = nowIso();
    atomicWriteJson(this.stateFile, this.state);
  }
  /** Arquiva STATE.json atual (nunca apaga) para recomeçar do zero. */
  archive() {
    if (!existsSync(this.stateFile)) return null;
    const target = join(this.dir, `STATE.${Date.now()}.archived.json`);
    renameSync(this.stateFile, target);
    this.state = null;
    return target;
  }
  log(event, data = {}) {
    const line = JSON.stringify({ at: nowIso(), runId: this.state?.runId, event, ...data });
    appendFileSync(this.logFile, `${redact(line)}\n`);
  }
  runDir(iteration) {
    const dir = join(this.dir, "runs", `iter-${String(iteration).padStart(2, "0")}`);
    mkdirSync(dir, { recursive: true });
    return dir;
  }
}
