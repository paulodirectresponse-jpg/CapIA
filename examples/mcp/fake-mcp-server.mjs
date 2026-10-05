// Servidor MCP FALSO (só para testar o cliente de exemplo; não é o capia-server). Fala JSON-RPC 2.0
// por linha em stdin/stdout e implementa initialize, tools/list e tools/call para poucas tools.
import { createInterface } from "node:readline";

const tools = ["server_info", "runs_create", "runs_get"].map((name) => ({
  name,
  description: name,
  inputSchema: { type: "object" },
}));
let polls = 0;
const reply = (id, result) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
const fail = (id, code, message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } })}\n`);
const text = (value, isError = false) => ({
  content: [{ type: "text", text: JSON.stringify(value) }],
  isError,
});

createInterface({ input: process.stdin }).on("line", (line) => {
  const m = JSON.parse(line);
  if (m.id === undefined) return; // notificação
  if (m.method === "initialize")
    return reply(m.id, {
      protocolVersion: "2025-06-18",
      capabilities: { tools: {} },
      serverInfo: { name: "fake-capia", version: "0" },
    });
  if (m.method === "tools/list") return reply(m.id, { tools });
  if (m.method === "tools/call") {
    const { name, arguments: a } = m.params;
    if (name === "server_info") return reply(m.id, text({ version: "fake" }));
    if (name === "runs_create")
      return reply(m.id, text({ run: { id: "run_1", status: "running" } }));
    if (name === "runs_get") {
      polls++;
      return reply(
        m.id,
        text({
          run:
            polls < 2
              ? { id: a.run_id, status: "running", stage: "plan" }
              : { id: a.run_id, status: "waiting_user", pending: { id: "dec_1" } },
        }),
      );
    }
    return reply(m.id, text({ code: "UNKNOWN_TOOL", message: name }, true));
  }
  return fail(m.id, -32601, `método desconhecido: ${m.method}`);
});
