#!/usr/bin/env node
// EXEMPLO (não é código de produto): cliente MCP mínimo (JSON-RPC 2.0, uma mensagem JSON por linha)
// que fala com `capia-server mcp-stdio` — INTERFACE PREVISTA (docs/api/mcp.md): confirme os flags
// com `capia-server --help` na sua versão.
//
//   export CAPIA_TOKEN=capia_REDACTED
//   node examples/mcp/stdio-client.mjs --data-dir <dir>                       # initialize + tools/list + server_info
//   node examples/mcp/stdio-client.mjs --data-dir <dir> --call projects_list --args '{}'
//   node examples/mcp/stdio-client.mjs --data-dir <dir> --run --project <id> --brief "..." --assets ast_1,ast_2
//
// `--run` inicia uma AI Run (tool `runs_create`) e consulta `runs_get` até um estado terminal ou
// `waiting_user` (nunca aprova sozinho: a aprovação é `runs_approve`, um scope separado).
// `--command`/`--cmd-arg` trocam o executável (usado nos testes com um servidor MCP falso).
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

export const PROTOCOL_VERSION = "2025-06-18";

export class McpStdioClient {
  constructor(command, args, { env = process.env, timeoutMs = 30_000 } = {}) {
    this.timeoutMs = timeoutMs;
    this.nextId = 1;
    this.pending = new Map();
    this.child = spawn(command, args, { env, stdio: ["pipe", "pipe", "inherit"] });
    createInterface({ input: this.child.stdout }).on("line", (line) => this.#onLine(line));
    this.child.on("exit", () => {
      for (const { reject } of this.pending.values()) reject(new Error("servidor MCP encerrou"));
      this.pending.clear();
    });
  }

  #onLine(line) {
    if (!line.trim()) return;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      return; // o stdout do MCP só deve ter JSON-RPC; qualquer outra linha é ignorada
    }
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    clearTimeout(p.timer);
    if (msg.error) p.reject(Object.assign(new Error(msg.error.message), { rpc: msg.error }));
    else p.resolve(msg.result);
  }

  request(method, params) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`timeout em ${method}`));
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }

  notify(method, params) {
    this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  async initialize() {
    const result = await this.request("initialize", {
      protocolVersion: PROTOCOL_VERSION,
      capabilities: {},
      clientInfo: { name: "capia-example-client", version: "0.0.0" },
    });
    this.notify("notifications/initialized", {});
    return result;
  }

  listTools = () => this.request("tools/list", {});

  /** Chama uma tool e devolve o JSON do primeiro bloco de texto; `isError` vira exceção. */
  async callTool(name, args = {}) {
    const res = await this.request("tools/call", { name, arguments: args });
    const text = res?.content?.find((c) => c.type === "text")?.text ?? "null";
    let value;
    try {
      value = JSON.parse(text);
    } catch {
      value = text;
    }
    if (res?.isError) throw Object.assign(new Error(`tool ${name} falhou`), { toolError: value });
    return value;
  }

  close() {
    this.child.stdin.end();
    this.child.kill();
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function startRunAndWatch(
  c,
  { project, brief, assets = [] },
  { log = console.log, sleepFn = sleep } = {},
) {
  const created = await c.callTool("runs_create", {
    project_id: project,
    brief_text: brief,
    assets,
    start: true,
  });
  const runId = created.run?.id ?? created.id;
  log(`run ${runId} iniciada`);
  for (;;) {
    const { run } = await c.callTool("runs_get", { project_id: project, run_id: runId });
    log(`status: ${run.status}${run.stage ? ` (${run.stage})` : ""}`);
    if (["completed", "failed", "cancelled"].includes(run.status)) return run;
    if (run.status === "waiting_user") {
      log(`decisão pendente ${run.pending?.id}: aprove com runs_approve (scope run:approve)`);
      return run;
    }
    await sleepFn(1000);
  }
}

function arg(name, def) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : def;
}

async function main() {
  if (!process.env.CAPIA_TOKEN) {
    console.error("defina CAPIA_TOKEN");
    process.exit(2);
  }
  const dataDir = arg("--data-dir");
  const command = arg("--command", "capia-server");
  const args = process.argv.includes("--command")
    ? process.argv.filter((_, i, a) => a[i - 1] === "--cmd-arg")
    : ["mcp-stdio", "--data-dir", dataDir, "--token-env", "CAPIA_TOKEN"];
  if (!dataDir && !process.argv.includes("--command")) {
    console.error(
      "uso: stdio-client.mjs --data-dir <dir> [--call <tool> --args <json> | --run ...]",
    );
    process.exit(2);
  }
  const c = new McpStdioClient(command, args);
  try {
    const info = await c.initialize();
    console.log(`servidor: ${info.serverInfo?.name} ${info.serverInfo?.version ?? ""}`);
    const tools = (await c.listTools()).tools ?? [];
    console.log(`${tools.length} tools: ${tools.map((t) => t.name).join(", ")}`);
    if (process.argv.includes("--run")) {
      const run = await startRunAndWatch(c, {
        project: arg("--project"),
        brief: arg("--brief"),
        assets: (arg("--assets") ?? "").split(",").filter(Boolean),
      });
      console.log(JSON.stringify({ id: run.id, status: run.status }));
    } else {
      const tool = arg("--call", "server_info");
      console.log(JSON.stringify(await c.callTool(tool, JSON.parse(arg("--args", "{}"))), null, 2));
    }
  } finally {
    c.close();
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((e) => {
    console.error(e.toolError ? `${e.message}: ${JSON.stringify(e.toolError)}` : e.message);
    process.exit(1);
  });
}
