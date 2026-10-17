// Adaptadores REAIS dos CLIs (claude -p / codex exec). Prompt sempre por stdin (sem limite de argv).
// Nada aqui usa --dangerously-skip-permissions: o executor roda com permission-mode + allowedTools da config.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export class AgentError extends Error {
  constructor(message, { kind = "FAILED", transient = false } = {}) {
    super(message);
    this.name = "AgentError";
    this.kind = kind; // AUTH | CLI_MISSING | TRANSIENT | TIMEOUT | INVALID_OUTPUT | FAILED
    this.transient = transient;
  }
}

const AUTH_RE =
  /not logged in|please (run|log ?in)|\/login|invalid api key|unauthori[sz]ed|authentication (failed|error)|credit balance|billing|\b401\b/i;
const TRANSIENT_RE =
  /rate.?limit|\b429\b|overloaded|\b50[234]\b|ETIMEDOUT|ECONNRESET|EAI_AGAIN|ENOTFOUND|temporar|try again|socket hang up/i;

export function classifyFailure(text) {
  if (AUTH_RE.test(text)) return { kind: "AUTH", transient: false };
  if (TRANSIENT_RE.test(text)) return { kind: "TRANSIENT", transient: true };
  return { kind: "FAILED", transient: true }; // falha desconhecida: tenta de novo, o limite de falhas consecutivas protege
}

function quoteWin(arg) {
  return /[\s"&|<>^()]/.test(arg) ? `"${arg.replace(/"/g, '\\"')}"` : arg;
}

/** Mata o processo E seus filhos (o `codex` é um wrapper Node que lança um binário nativo neto). */
function killTree(child, isWin) {
  try {
    if (isWin) spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true });
    else process.kill(-child.pid, "SIGKILL"); // grupo de processos (spawn com detached: true)
  } catch {
    try {
      child.kill("SIGKILL");
    } catch {}
  }
}

export function runProcess({ bin, args, input, cwd, timeoutMs }) {
  return new Promise((resolve, reject) => {
    const isWin = process.platform === "win32";
    const child = isWin
      ? spawn([bin, ...args.map(quoteWin)].join(" "), { cwd, shell: true, windowsHide: true })
      : spawn(bin, args, { cwd, detached: true });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    let settled = false;
    const settle = (fn, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(grace);
      child.stdout.destroy();
      child.stderr.destroy();
      fn(value);
    };
    let grace;
    const timer = setTimeout(() => {
      timedOut = true;
      killTree(child, isWin);
      // não confie só no evento `close`: um neto vivo pode segurar os pipes para sempre
      grace = setTimeout(
        () => settle(resolve, { code: null, stdout, stderr, timedOut: true }),
        3000,
      );
    }, timeoutMs);
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("error", (e) =>
      settle(
        reject,
        e.code === "ENOENT"
          ? new AgentError(`${bin} não encontrado no PATH`, { kind: "CLI_MISSING" })
          : e,
      ),
    );
    child.on("close", (code) => settle(resolve, { code, stdout, stderr, timedOut }));
    child.stdin.on("error", () => {});
    child.stdin.end(input ?? "");
  });
}

export async function callClaude({ cfg, prompt, cwd, timeoutMs, allowedTools }) {
  const c = cfg.claude;
  const args = [
    "-p",
    "--output-format",
    "json",
    "--permission-mode",
    c.permissionMode,
    "--max-turns",
    String(c.maxTurns),
  ];
  const tools = allowedTools ?? c.allowedTools;
  if (tools?.length) args.push("--allowedTools", tools.join(","));
  if (c.model) args.push("--model", c.model);
  const r = await runProcess({ bin: c.bin, args, input: prompt, cwd, timeoutMs });
  if (r.timedOut)
    throw new AgentError(`claude excedeu ${Math.round(timeoutMs / 60000)} min`, {
      kind: "TIMEOUT",
      transient: true,
    });
  let json = null;
  try {
    json = JSON.parse(r.stdout);
  } catch {}
  const text = json?.result ?? "";
  if (r.code !== 0 || json?.is_error || !json) {
    const detail = `${text} ${r.stderr} ${r.stdout}`.slice(0, 600);
    const { kind, transient } = classifyFailure(detail);
    throw new AgentError(`claude falhou (exit ${r.code}): ${detail.trim().slice(0, 300)}`, {
      kind,
      transient,
    });
  }
  return { text, costUsd: Number(json.total_cost_usd ?? 0), sessionId: json.session_id ?? null };
}

export async function callCodex({ cfg, prompt, cwd, timeoutMs }) {
  const c = cfg.codex;
  const dir = mkdtempSync(join(tmpdir(), "capia-codex-"));
  const outFile = join(dir, "last-message.txt");
  try {
    const args = ["exec", "--sandbox", c.sandbox, "--output-last-message", outFile];
    if (c.model) args.push("--model", c.model);
    args.push("-"); // prompt por stdin
    const r = await runProcess({ bin: c.bin, args, input: prompt, cwd, timeoutMs });
    if (r.timedOut)
      throw new AgentError(`codex excedeu ${Math.round(timeoutMs / 60000)} min`, {
        kind: "TIMEOUT",
        transient: true,
      });
    let text = "";
    try {
      text = readFileSync(outFile, "utf8");
    } catch {}
    if (r.code !== 0 || !text.trim()) {
      const detail = `${r.stderr} ${r.stdout}`.slice(0, 600);
      const { kind, transient } = classifyFailure(detail);
      throw new AgentError(`codex falhou (exit ${r.code}): ${detail.trim().slice(0, 300)}`, {
        kind,
        transient,
      });
    }
    return { text, costUsd: 0 };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** role -> função (ctx) => { text, costUsd } conforme roles.<fase>.agent da config. */
export function buildAdapters(cfg) {
  const byAgent = { claude: callClaude, codex: callCodex };
  const make = (phase) => {
    const fn = byAgent[cfg.roles[phase].agent];
    if (!fn) throw new Error(`agente desconhecido em roles.${phase}: ${cfg.roles[phase].agent}`);
    // Claude no papel de auditor: somente leitura (nunca edita o que está auditando)
    const tools =
      phase === "auditor" && cfg.roles[phase].agent === "claude"
        ? { allowedTools: cfg.claude.auditorAllowedTools }
        : {};
    return (ctx) => fn({ cfg, ...tools, ...ctx });
  };
  return { executor: make("executor"), auditor: make("auditor") };
}
