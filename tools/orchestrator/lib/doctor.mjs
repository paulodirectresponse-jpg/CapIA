// Diagnóstico do ambiente: cada checagem diz o que está errado E como consertar.
import { existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { runProcess } from "./agents.mjs";

const quick = (bin, args) =>
  runProcess({ bin, args, input: "", cwd: process.cwd(), timeoutMs: 30_000 }).then(
    (r) => ({ ok: r.code === 0, out: `${r.stdout}${r.stderr}`.trim() }),
    (e) => ({ ok: false, out: String(e.message) }),
  );

export async function doctor({ root, cfg, probeAuth = true }) {
  const checks = [];
  const add = (name, ok, detail, fix = null, severity = "fatal") =>
    checks.push({ name, ok, detail, fix, severity });

  const major = Number(process.versions.node.split(".")[0]);
  add(
    "node >= 20",
    major >= 20,
    `node ${process.versions.node}`,
    "instale Node 22+ (https://nodejs.org)",
  );

  let branch = "";
  try {
    branch = execFileSync("git", ["rev-parse", "--abbrev-ref", "HEAD"], {
      cwd: root,
      encoding: "utf8",
    }).trim();
    add("repositório git", true, `branch ${branch}`);
    add(
      "branch não protegida",
      !cfg.protectedBranches.includes(branch),
      `branch atual: ${branch}; protegidas: ${cfg.protectedBranches.join(", ")}`,
      "troque de branch: git checkout -b integration/<nome> (o orquestrador nunca roda em branch protegida)",
    );
  } catch {
    add(
      "repositório git",
      false,
      "git não encontrado ou fora de um repositório",
      "rode dentro do clone do CapIA",
    );
  }

  for (const agent of new Set(Object.values(cfg.roles).map((r) => r.agent))) {
    const bin = cfg[agent].bin;
    const v = await quick(bin, ["--version"]);
    add(
      `${agent} CLI instalado`,
      v.ok,
      v.ok ? v.out.split("\n")[0] : v.out.slice(0, 160),
      agent === "claude" ? "npm i -g @anthropic-ai/claude-code" : "npm i -g @openai/codex",
    );
    if (!v.ok) continue;
    if (agent === "codex") {
      const a = await quick(bin, ["login", "status"]);
      // `codex login status` imprime "Not logged in" e sai com 0: o texto decide, não o exit code.
      const loggedIn = a.ok && !/not logged in|no (credentials|api key)|logged out/i.test(a.out);
      add(
        `${agent} autenticado`,
        loggedIn,
        a.out.slice(0, 160) || "ok",
        "rode: codex login (ou defina OPENAI_API_KEY)",
      );
    } else if (probeAuth) {
      const a = await runProcess({
        bin,
        args: ["-p", "--output-format", "json", "--max-turns", "1"],
        input: "Responda apenas: OK",
        cwd: root,
        timeoutMs: 90_000,
      }).catch((e) => ({ code: 1, stdout: "", stderr: String(e.message) }));
      let ok = false;
      try {
        ok = a.code === 0 && !JSON.parse(a.stdout).is_error;
      } catch {}
      add(
        `${agent} autenticado (chamada real mínima)`,
        ok,
        ok ? "ok" : `${a.stderr}${a.stdout}`.slice(0, 160),
        "rode: claude e faça /login (ou defina ANTHROPIC_API_KEY)",
      );
    }
  }
  const fatal = checks.filter((c) => !c.ok && c.severity === "fatal");
  return { ok: fatal.length === 0, checks };
}
