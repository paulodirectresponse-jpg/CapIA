// Executor declarativo dos pacotes de aceitação da Fase 6 (external-flow, installer, update, security).
//
// Lê um `steps.json` e reporta CADA passo como `passed | failed | pending_external | not_available` —
// nunca "pulado" em silêncio e nunca "passou" sem ter rodado/validado:
//   - passo de comando: roda se os requisitos existem; sem eles ⇒ `not_available` (diz o que falta:
//     binário de outra frente, arquivo de teste, variável de ambiente). Saída ≠ esperada ⇒ `failed`.
//   - passo `external` (humano/hardware/certificado/jurídico): sem arquivo de evidência ⇒
//     `pending_external`; com evidência ⇒ valida (comando `validate` ou checagem genérica mínima).
//
// Formato do steps.json:
//   { "suite": "nome", "description": "…", "steps": [
//     { "id": "x", "title": "…",
//       "requires": { "bin": ["capia-server"], "file": ["crates/…"], "env": ["CAPIA_PORT"] },
//       "cmd": ["${bin:capia-server}", "--version"], "expectExit": 0, "timeoutSec": 600 },
//     { "id": "y", "title": "…", "external": true, "evidence": "target/phase6-acceptance/evidence/y.json",
//       "validate": ["node", "tools/…/validate.mjs", "--file", "${evidence}"], "why": "…" } ] }
//
// Variáveis em `cmd`/`validate`: ${root}, ${evidence}, ${bin:NOME} (resolvido), ${env:NOME}.
// Resolução de binário: variável CAPIA_<NOME>_BIN (sem o prefixo `capia-`; traços → sublinhado, maiúsculas;
// ex.: capia-server → CAPIA_SERVER_BIN, cargo-deny → CAPIA_CARGO_DENY_BIN), depois
// target/release e target/debug, depois PATH.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
// CAPIA_P6_OUT redireciona as saídas (os testes usam um diretório temporário).
export const outDir = process.env.CAPIA_P6_OUT ?? join(root, "target/phase6-acceptance");

export const STATUS = ["passed", "failed", "pending_external", "not_available"];
// Severidade crescente para o agregado: o pior estado domina; só "passed" se TUDO passou.
const RANK = { passed: 0, pending_external: 1, not_available: 2, failed: 3 };

export function aggregateStatus(statuses) {
  if (statuses.length === 0) return "not_available";
  return statuses.reduce((w, s) => (RANK[s] > RANK[w] ? s : w), "passed");
}

export function binEnvName(name) {
  return `CAPIA_${name
    .replace(/^capia-/, "")
    .replace(/[^A-Za-z0-9]/g, "_")
    .toUpperCase()}_BIN`;
}

export function resolveBin(name, ctx = {}) {
  const {
    rootDir = root,
    env = process.env,
    platform = process.platform,
    exists = existsSync,
  } = ctx;
  const fromEnv = env[binEnvName(name)];
  if (fromEnv && exists(fromEnv)) return fromEnv;
  const exts = platform === "win32" ? [".exe", ".cmd", ".bat", ""] : [""];
  for (const dir of ["target/release", "target/debug"])
    for (const e of exts) {
      const p = join(rootDir, dir, name + e);
      if (exists(p)) return p;
    }
  const sep = platform === "win32" ? ";" : delimiter;
  for (const dir of (env.PATH ?? env.Path ?? "").split(sep).filter(Boolean))
    for (const e of exts) {
      const p = join(dir, name + e);
      if (exists(p)) return p;
    }
  return null;
}

/** Quais requisitos faltam (lista legível). Vazio ⇒ pode rodar. */
export function missingRequirements(requires = {}, ctx = {}) {
  const { rootDir = root, env = process.env, exists = existsSync } = ctx;
  const missing = [];
  for (const b of requires.bin ?? [])
    if (!resolveBin(b, ctx))
      missing.push(`binário \`${b}\` (defina ${binEnvName(b)} ou compile em target/)`);
  for (const f of requires.file ?? [])
    if (!exists(resolve(rootDir, f))) missing.push(`arquivo \`${f}\``);
  for (const e of requires.env ?? []) if (!env[e]) missing.push(`variável de ambiente \`${e}\``);
  return missing;
}

export function substitute(arg, ctx = {}) {
  const { rootDir = root, env = process.env, evidence = "" } = ctx;
  return arg.replace(/\$\{([^}]+)\}/g, (_, key) => {
    if (key === "root") return rootDir;
    if (key === "evidence") return evidence;
    if (key.startsWith("bin:")) {
      const p = resolveBin(key.slice(4), ctx);
      if (!p) throw new Error(`binário ${key.slice(4)} não encontrado`);
      return p;
    }
    if (key.startsWith("env:")) return env[key.slice(4)] ?? "";
    throw new Error(`variável desconhecida em cmd: \${${key}}`);
  });
}

export function validateSpec(spec) {
  const errs = [];
  if (!spec || typeof spec !== "object") throw new Error("steps.json inválido");
  if (!spec.suite) errs.push("`suite` ausente");
  if (!Array.isArray(spec.steps) || spec.steps.length === 0) errs.push("`steps` vazio");
  const ids = new Set();
  for (const s of spec.steps ?? []) {
    if (!s.id || !s.title) errs.push(`passo sem id/title: ${JSON.stringify(s).slice(0, 60)}`);
    if (ids.has(s.id)) errs.push(`${s.id}: id repetido`);
    ids.add(s.id);
    if (s.external) {
      if (!s.evidence) errs.push(`${s.id}: passo externo exige \`evidence\``);
      if (!s.why) errs.push(`${s.id}: passo externo exige \`why\` (por que é externo)`);
    } else if (!Array.isArray(s.cmd) || s.cmd.length === 0) errs.push(`${s.id}: \`cmd\` ausente`);
  }
  if (errs.length) throw new Error(`steps.json inválido:\n- ${errs.join("\n- ")}`);
  return spec;
}

/**
 * Checagem genérica mínima de evidência de passo externo (quando não há `validate`):
 * JSON com `result: "passed"|"failed"`, `performed_by` e `performed_at` (ISO, não futuro).
 */
export function genericEvidenceStatus(file, now = Date.now()) {
  let j;
  try {
    j = JSON.parse(readFileSync(file, "utf8"));
  } catch (e) {
    return { status: "failed", reason: `evidência ilegível: ${e.message}` };
  }
  if (j?.template === true)
    return { status: "pending_external", reason: "evidência é um modelo não preenchido" };
  const problems = [];
  if (!j.performed_by || typeof j.performed_by !== "string") problems.push("performed_by ausente");
  const t = Date.parse(j.performed_at ?? "");
  if (Number.isNaN(t)) problems.push("performed_at inválido");
  else if (t > now + 60_000) problems.push("performed_at no futuro");
  if (j.result !== "passed" && j.result !== "failed")
    problems.push("result deve ser passed|failed");
  if (problems.length) return { status: "failed", reason: problems.join("; ") };
  return j.result === "passed"
    ? { status: "passed" }
    : { status: "failed", reason: "a evidência registra falha" };
}

function runCmd(cmd, ctx, timeoutSec) {
  const argv = cmd.map((a) => substitute(a, ctx));
  const t0 = Date.now();
  const r = spawnSync(argv[0], argv.slice(1), {
    cwd: ctx.rootDir ?? root,
    encoding: "utf8",
    shell: false,
    timeout: (timeoutSec ?? 1800) * 1000,
    env: { ...process.env, CARGO_INCREMENTAL: "0" },
  });
  return {
    argv,
    status: r.status,
    stdout: r.stdout ?? "",
    stderr: `${r.stderr ?? ""}${r.error ? `\n${r.error.message}` : ""}`,
    seconds: Math.round((Date.now() - t0) / 100) / 10,
  };
}

const tail = (r) => `${r.stdout}${r.stderr}`.split("\n").slice(-25).join("\n");

export function runStep(step, ctx = {}) {
  const rec = { id: step.id, title: step.title };
  const rootDir = ctx.rootDir ?? root;
  if (step.external) {
    const evidence = resolve(rootDir, step.evidence);
    rec.evidence = step.evidence;
    rec.why = step.why;
    if (!existsSync(evidence))
      return {
        ...rec,
        status: "pending_external",
        reason: `sem evidência real em ${step.evidence}`,
      };
    if (!step.validate) {
      const g = genericEvidenceStatus(evidence, ctx.now);
      return { ...rec, ...g };
    }
    const miss = missingRequirements(step.requires, ctx);
    if (miss.length)
      return { ...rec, status: "not_available", reason: `falta: ${miss.join(", ")}` };
    const r = runCmd(step.validate, { ...ctx, evidence }, step.timeoutSec ?? 120);
    let verdict = null;
    try {
      verdict = JSON.parse(r.stdout).status;
    } catch {
      /* sem JSON */
    }
    const map = {
      accepted: "passed",
      passed: "passed",
      rejected: "failed",
      pending_external: "pending_external",
      partial: "pending_external",
    };
    const status = map[verdict] ?? "failed";
    return {
      ...rec,
      status,
      reason:
        status === "passed"
          ? undefined
          : `validador respondeu \`${verdict ?? "sem veredito"}\` (exit ${r.status})`,
      command: r.argv.join(" "),
      seconds: r.seconds,
      tail: status === "failed" ? tail(r) : undefined,
    };
  }
  const miss = missingRequirements(step.requires, ctx);
  if (miss.length)
    return {
      ...rec,
      status: "not_available",
      reason: `stub: falta ${miss.join(", ")}. Passo NÃO executado e NÃO contado como aprovado.`,
    };
  const r = runCmd(step.cmd, ctx, step.timeoutSec);
  const ok = r.status === (step.expectExit ?? 0);
  return {
    ...rec,
    status: ok ? "passed" : "failed",
    command: r.argv.join(" "),
    seconds: r.seconds,
    tail: ok ? undefined : tail(r),
  };
}

export function runSteps(spec, ctx = {}, { only } = {}) {
  validateSpec(spec);
  const steps = spec.steps.filter((s) => !only || only.includes(s.id));
  const results = steps.map((s) => {
    let rec;
    try {
      rec = runStep(s, ctx);
    } catch (e) {
      rec = { id: s.id, title: s.title, status: "failed", reason: e.message };
    }
    return rec;
  });
  const counts = Object.fromEntries(
    STATUS.map((k) => [k, results.filter((r) => r.status === k).length]),
  );
  return {
    suite: spec.suite,
    description: spec.description,
    generated: new Date().toISOString(),
    status: aggregateStatus(results.map((r) => r.status)),
    counts,
    steps: results,
  };
}

export function writeSummary(summary, dir = outDir) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${summary.suite}-summary.json`), JSON.stringify(summary, null, 2));
}

/** CLI comum dos `run.mjs`: `--list`, `--only id[,id]`, `--strict` (sai 1 se não for `passed`). */
export function cli(stepsFile, argv = process.argv.slice(2)) {
  const spec = JSON.parse(readFileSync(stepsFile, "utf8"));
  validateSpec(spec);
  if (argv.includes("--list")) {
    for (const s of spec.steps)
      console.log(`${s.external ? "[externo]" : "[comando]"} ${s.id}: ${s.title}`);
    return 0;
  }
  const oi = argv.indexOf("--only");
  const only = oi >= 0 ? argv[oi + 1].split(",") : undefined;
  const summary = runSteps(spec, {}, { only });
  writeSummary(summary);
  for (const s of summary.steps)
    console.log(`${s.status.toUpperCase().padEnd(16)} ${s.id}${s.reason ? `  — ${s.reason}` : ""}`);
  console.log(`\n${summary.suite}: ${summary.status}  ${JSON.stringify(summary.counts)}`);
  if (summary.status !== "passed")
    console.log("(só `passed` significa aprovado; os demais estados NÃO são aprovação)");
  const failed = summary.status === "failed";
  return failed || (argv.includes("--strict") && summary.status !== "passed") ? 1 : 0;
}
