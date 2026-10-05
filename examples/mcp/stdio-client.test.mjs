// Testa o cliente MCP de exemplo contra o servidor MCP falso (JSON-RPC por linha em stdio).
import { test } from "node:test";
import assert from "node:assert/strict";
import { McpStdioClient, startRunAndWatch } from "./stdio-client.mjs";

const fake = new URL("./fake-mcp-server.mjs", import.meta.url).pathname;
const open = () => new McpStdioClient(process.execPath, [fake], { timeoutMs: 5000 });

test("initialize, tools/list e tools/call", async () => {
  const c = open();
  try {
    const init = await c.initialize();
    assert.equal(init.serverInfo.name, "fake-capia");
    const { tools } = await c.listTools();
    assert.deepEqual(
      tools.map((t) => t.name),
      ["server_info", "runs_create", "runs_get"],
    );
    assert.deepEqual(await c.callTool("server_info"), { version: "fake" });
  } finally {
    c.close();
  }
});

test("erro de tool (isError) e erro JSON-RPC viram exceções distintas", async () => {
  const c = open();
  try {
    await c.initialize();
    await assert.rejects(
      () => c.callTool("nao_existe"),
      (e) => e.toolError?.code === "UNKNOWN_TOOL",
    );
    await assert.rejects(
      () => c.request("metodo/inexistente", {}),
      (e) => e.rpc?.code === -32601,
    );
  } finally {
    c.close();
  }
});

test("startRunAndWatch para em waiting_user sem aprovar", async () => {
  const c = open();
  try {
    await c.initialize();
    const lines = [];
    const run = await startRunAndWatch(
      c,
      { project: "prj_1", brief: "b", assets: ["ast_1"] },
      { log: (l) => lines.push(l), sleepFn: async () => {} },
    );
    assert.equal(run.status, "waiting_user");
    assert.ok(lines.some((l) => l.includes("runs_approve")));
  } finally {
    c.close();
  }
});

test("timeout quando o servidor não responde", async () => {
  const c = new McpStdioClient(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
    timeoutMs: 150,
  });
  try {
    await assert.rejects(() => c.request("initialize", {}), /timeout/);
  } finally {
    c.close();
  }
});
