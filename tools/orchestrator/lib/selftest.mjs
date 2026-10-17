// Teste de ponta a ponta com os CLIs REAIS: Claude -> Codex -> Claude, passando um nonce pela cadeia.
import { randomBytes } from "node:crypto";
import { AgentError, callClaude, callCodex, runProcess } from "./agents.mjs";

export async function selftest({ cfg, root, mockCodex = false }) {
  const nonce = randomBytes(4).toString("hex").toUpperCase();
  const lean = { ...cfg, claude: { ...cfg.claude, maxTurns: 1, allowedTools: [] } };
  const legs = [];
  const leg = async (name, agent, fn, expect, fix) => {
    const t0 = Date.now();
    try {
      const res = await fn();
      const ok = res.text.includes(expect);
      legs.push({
        name,
        agent,
        status: ok ? "PASS" : "FAIL",
        ms: Date.now() - t0,
        detail: ok
          ? `devolveu ${expect}`
          : `esperado ${expect}, veio: ${res.text.trim().slice(0, 120)}`,
        text: res.text,
      });
      return ok ? res.text : null;
    } catch (e) {
      const blocked = e instanceof AgentError && (e.kind === "CLI_MISSING" || e.kind === "AUTH");
      legs.push({
        name,
        agent,
        status: blocked ? "BLOCKED" : "FAIL",
        ms: Date.now() - t0,
        detail: String(e.message).slice(0, 220),
        fix: blocked ? fix : null,
      });
      return null;
    }
  };
  const timeoutMs = 180_000;

  const a = await leg(
    "1/3 Claude gera",
    "claude",
    () =>
      callClaude({
        cfg: lean,
        cwd: root,
        timeoutMs,
        prompt: `Responda SOMENTE com esta linha, sem mais nada: PING-${nonce}`,
      }),
    `PING-${nonce}`,
    "npm i -g @anthropic-ai/claude-code && claude (/login)",
  );

  const codexPrompt = `Você recebeu a mensagem "PING-${nonce}". Responda SOMENTE com esta linha, sem mais nada: PONG-${nonce}`;
  // falha rápida e sem gastar tokens: `codex login status` (imprime "Not logged in" mas sai com 0)
  const codexGuarded = async () => {
    const st = await runProcess({
      bin: cfg.codex.bin,
      args: ["login", "status"],
      input: "",
      cwd: root,
      timeoutMs: 30_000,
    });
    if (/not logged in|no (credentials|api key)|logged out/i.test(`${st.stdout}${st.stderr}`))
      throw new AgentError(
        `codex não autenticado: ${`${st.stdout}${st.stderr}`.trim().slice(0, 80)}`,
        { kind: "AUTH" },
      );
    return callCodex({ cfg, cwd: root, timeoutMs, prompt: codexPrompt });
  };
  const codexFn = mockCodex ? async () => ({ text: `PONG-${nonce}` }) : codexGuarded;
  const b = a
    ? await leg(
        `2/3 Codex audita${mockCodex ? " (SIMULADO)" : ""}`,
        "codex",
        codexFn,
        `PONG-${nonce}`,
        "npm i -g @openai/codex && codex login",
      )
    : null;
  if (!a)
    legs.push({
      name: "2/3 Codex audita",
      agent: "codex",
      status: "SKIPPED",
      detail: "perna 1 falhou",
    });

  if (b)
    await leg(
      "3/3 Claude fecha",
      "claude",
      () =>
        callClaude({
          cfg: lean,
          cwd: root,
          timeoutMs,
          prompt: `O auditor respondeu "PONG-${nonce}". Responda SOMENTE com esta linha, sem mais nada: FINAL-${nonce}`,
        }),
      `FINAL-${nonce}`,
      "npm i -g @anthropic-ai/claude-code",
    );
  else
    legs.push({
      name: "3/3 Claude fecha",
      agent: "claude",
      status: "SKIPPED",
      detail: "perna 2 não passou",
    });

  const real = legs.every((l) => l.status === "PASS") && !mockCodex;
  const overall = real
    ? "PASS"
    : legs.some((l) => l.status === "FAIL")
      ? "FAIL"
      : legs.some((l) => l.status === "BLOCKED")
        ? "BLOCKED"
        : "PARTIAL";
  return { nonce, overall, legs };
}
