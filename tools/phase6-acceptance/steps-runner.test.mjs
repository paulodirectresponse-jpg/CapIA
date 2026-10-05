import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  aggregateStatus,
  binEnvName,
  genericEvidenceStatus,
  missingRequirements,
  resolveBin,
  runStep,
  runSteps,
  substitute,
  validateSpec,
} from "./steps-runner.mjs";

const node = process.execPath;
const tmp = () => mkdtempSync(join(tmpdir(), "p6-"));

test("o agregado é o PIOR estado; só passed se tudo passou; vazio não é passed", () => {
  assert.equal(aggregateStatus(["passed", "passed"]), "passed");
  assert.equal(aggregateStatus(["passed", "pending_external"]), "pending_external");
  assert.equal(aggregateStatus(["pending_external", "not_available"]), "not_available");
  assert.equal(aggregateStatus(["not_available", "failed", "passed"]), "failed");
  assert.equal(aggregateStatus([]), "not_available");
});

test("resolução de binário: env → target/release → target/debug → PATH", () => {
  assert.equal(binEnvName("capia-server"), "CAPIA_SERVER_BIN");
  assert.equal(binEnvName("cargo-deny"), "CAPIA_CARGO_DENY_BIN");
  const have = new Set();
  const exists = (p) => have.has(p);
  const ctx = { rootDir: "/r", env: { PATH: "/usr/bin:/bin" }, platform: "linux", exists };
  assert.equal(resolveBin("capia-server", ctx), null);
  have.add("/bin/capia-server");
  assert.equal(resolveBin("capia-server", ctx), "/bin/capia-server");
  have.add("/r/target/debug/capia-server");
  assert.equal(resolveBin("capia-server", ctx), "/r/target/debug/capia-server");
  have.add("/r/target/release/capia-server");
  assert.equal(resolveBin("capia-server", ctx), "/r/target/release/capia-server");
  have.add("/custom/srv");
  assert.equal(
    resolveBin("capia-server", { ...ctx, env: { ...ctx.env, CAPIA_SERVER_BIN: "/custom/srv" } }),
    "/custom/srv",
  );
  // env apontando para arquivo inexistente é ignorado (não finge que existe)
  assert.equal(
    resolveBin("capia-server", { ...ctx, env: { CAPIA_SERVER_BIN: "/nope" } }),
    "/r/target/release/capia-server",
  );
});

test("requisitos faltantes são listados (binário, arquivo, env)", () => {
  const m = missingRequirements(
    { bin: ["zzz-inexistente"], file: ["nao/existe.txt"], env: ["VAR_QUE_NAO_EXISTE_P6"] },
    { rootDir: tmp(), env: {} },
  );
  assert.equal(m.length, 3);
  assert.match(m[0], /zzz-inexistente/);
  assert.deepEqual(missingRequirements({}, {}), []);
});

test("substituição de variáveis; desconhecida e binário ausente falham alto", () => {
  assert.equal(substitute("${root}/x", { rootDir: "/r" }), "/r/x");
  assert.equal(substitute("--f=${evidence}", { evidence: "/e.json" }), "--f=/e.json");
  assert.equal(substitute("${env:ABC}", { env: { ABC: "1" } }), "1");
  assert.equal(substitute("${env:NADA}", { env: {} }), "");
  assert.throws(() => substitute("${desconhecida}"), /desconhecida/);
  assert.throws(
    () => substitute("${bin:zzz-nao-existe}", { env: {}, rootDir: tmp() }),
    /não encontrado/,
  );
});

test("validateSpec recusa spec malformada e passo externo sem evidência/why", () => {
  assert.throws(() => validateSpec({}), /inválido/);
  assert.throws(() => validateSpec({ suite: "s", steps: [] }), /vazio/);
  assert.throws(() => validateSpec({ suite: "s", steps: [{ id: "a", title: "t" }] }), /cmd/);
  assert.throws(
    () => validateSpec({ suite: "s", steps: [{ id: "a", title: "t", external: true }] }),
    /evidence/,
  );
  assert.throws(
    () =>
      validateSpec({
        suite: "s",
        steps: [
          { id: "a", title: "t", cmd: ["x"] },
          { id: "a", title: "t", cmd: ["x"] },
        ],
      }),
    /repetido/,
  );
  assert.ok(validateSpec({ suite: "s", steps: [{ id: "a", title: "t", cmd: ["x"] }] }));
});

test("passo de comando: passed, failed (exit ≠ esperado) e expectExit", () => {
  const ctx = { rootDir: tmp() };
  const ok = runStep({ id: "a", title: "t", cmd: [node, "-e", "process.exit(0)"] }, ctx);
  assert.equal(ok.status, "passed");
  const bad = runStep(
    { id: "b", title: "t", cmd: [node, "-e", "console.error('boom');process.exit(3)"] },
    ctx,
  );
  assert.equal(bad.status, "failed");
  assert.match(bad.tail, /boom/);
  const exp = runStep(
    { id: "c", title: "t", cmd: [node, "-e", "process.exit(3)"], expectExit: 3 },
    ctx,
  );
  assert.equal(exp.status, "passed");
  const to = runStep(
    { id: "d", title: "t", cmd: [node, "-e", "setTimeout(()=>{},5000)"], timeoutSec: 0.2 },
    ctx,
  );
  assert.equal(to.status, "failed");
});

test("requisito ausente ⇒ not_available (stub explícito), NUNCA passed e NUNCA executa", () => {
  const dir = tmp();
  const marker = join(dir, "ran.txt");
  const r = runStep(
    {
      id: "a",
      title: "t",
      requires: { bin: ["zzz-binario-de-outra-frente"] },
      cmd: [node, "-e", `require('fs').writeFileSync(${JSON.stringify(marker)},'x')`],
    },
    { rootDir: dir, env: {} },
  );
  assert.equal(r.status, "not_available");
  assert.match(r.reason, /stub/);
  assert.match(r.reason, /NÃO executado/);
  assert.throws(() => readFileSync(marker)); // o comando não rodou
});

test("passo externo: sem evidência ⇒ pending_external; com validador ⇒ segue o veredito", () => {
  const dir = tmp();
  const step = {
    id: "x",
    title: "t",
    external: true,
    why: "humano",
    evidence: "ev/x.json",
    validate: [node, "-e", "console.log(JSON.stringify({status: process.env.V || 'accepted'}))"],
  };
  assert.equal(runStep(step, { rootDir: dir }).status, "pending_external");
  mkdirSync(join(dir, "ev"));
  writeFileSync(join(dir, "ev/x.json"), "{}");
  assert.equal(runStep(step, { rootDir: dir }).status, "passed");
  const rej = {
    ...step,
    validate: [node, "-e", "console.log(JSON.stringify({status:'rejected'}));process.exit(1)"],
  };
  assert.equal(runStep(rej, { rootDir: dir }).status, "failed");
  const part = {
    ...step,
    validate: [node, "-e", "console.log(JSON.stringify({status:'partial'}))"],
  };
  assert.equal(runStep(part, { rootDir: dir }).status, "pending_external");
  const pend = {
    ...step,
    validate: [node, "-e", "console.log(JSON.stringify({status:'pending_external'}))"],
  };
  assert.equal(runStep(pend, { rootDir: dir }).status, "pending_external");
  const nojson = { ...step, validate: [node, "-e", "console.log('lixo')"] };
  assert.equal(runStep(nojson, { rootDir: dir }).status, "failed"); // sem veredito ⇒ nunca passa
});

test("evidência genérica (sem validador): exige quem, quando e result; modelo é pendente", () => {
  const f = join(tmp(), "e.json");
  const now = Date.parse("2026-10-20T00:00:00Z");
  const w = (o) => writeFileSync(f, JSON.stringify(o));
  w({ performed_by: "Ana", performed_at: "2026-10-10T00:00:00Z", result: "passed" });
  assert.equal(genericEvidenceStatus(f, now).status, "passed");
  w({ performed_by: "Ana", performed_at: "2026-10-10T00:00:00Z", result: "failed" });
  assert.equal(genericEvidenceStatus(f, now).status, "failed");
  w({ performed_by: "", performed_at: "2026-10-10T00:00:00Z", result: "passed" });
  assert.equal(genericEvidenceStatus(f, now).status, "failed");
  w({ performed_by: "Ana", performed_at: "2030-01-01T00:00:00Z", result: "passed" });
  assert.equal(genericEvidenceStatus(f, now).status, "failed");
  w({ performed_by: "Ana", performed_at: "x", result: "passed" });
  assert.equal(genericEvidenceStatus(f, now).status, "failed");
  w({ template: true });
  assert.equal(genericEvidenceStatus(f, now).status, "pending_external");
  writeFileSync(f, "não é json");
  assert.equal(genericEvidenceStatus(f, now).status, "failed");
});

test("runSteps reporta TODOS os passos (nenhum some) e o agregado correto", () => {
  const dir = tmp();
  const spec = {
    suite: "demo",
    steps: [
      { id: "ok", title: "ok", cmd: [node, "-e", "0"] },
      { id: "na", title: "na", requires: { env: ["P6_NAO_DEFINIDA"] }, cmd: [node, "-e", "0"] },
      { id: "ext", title: "ext", external: true, why: "w", evidence: "nao/existe.json" },
    ],
  };
  const s = runSteps(spec, { rootDir: dir, env: {} });
  assert.deepEqual(
    s.steps.map((x) => [x.id, x.status]),
    [
      ["ok", "passed"],
      ["na", "not_available"],
      ["ext", "pending_external"],
    ],
  );
  assert.equal(s.status, "not_available");
  assert.deepEqual(s.counts, { passed: 1, failed: 0, pending_external: 1, not_available: 1 });
  const only = runSteps(spec, { rootDir: dir, env: {} }, { only: ["ok"] });
  assert.equal(only.status, "passed");
  assert.equal(only.steps.length, 1);
});

test("exceção em passo (variável inválida) vira failed, não desaparece", () => {
  const spec = { suite: "d", steps: [{ id: "a", title: "t", cmd: ["${desconhecida}"] }] };
  const s = runSteps(spec, { rootDir: tmp(), env: {} });
  assert.equal(s.steps[0].status, "failed");
  assert.equal(s.status, "failed");
});

test("CLI: --list, exit 0 com pendências, exit 1 com falha, --strict exige passed", () => {
  const dir = tmp();
  const out = join(dir, "out");
  const stepsFile = join(dir, "steps.json");
  const run = (args, steps) => {
    writeFileSync(stepsFile, JSON.stringify(steps));
    const script = `import { cli } from ${JSON.stringify(new URL("./steps-runner.mjs", import.meta.url).href)};
      process.exit(cli(${JSON.stringify(stepsFile)}, ${JSON.stringify(args)}));`;
    return spawnSync(node, ["--input-type=module", "-e", script], {
      encoding: "utf8",
      env: { ...process.env, CAPIA_P6_OUT: out },
    });
  };
  const pending = {
    suite: "cli-demo",
    steps: [{ id: "e", title: "t", external: true, why: "w", evidence: "x/nao.json" }],
  };
  const list = run(["--list"], pending);
  assert.equal(list.status, 0);
  assert.match(list.stdout, /\[externo\] e/);
  const r = run([], pending);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /PENDING_EXTERNAL/);
  assert.match(r.stdout, /NÃO são aprovação/);
  assert.equal(
    JSON.parse(readFileSync(join(out, "cli-demo-summary.json"), "utf8")).status,
    "pending_external",
  );
  assert.equal(run(["--strict"], pending).status, 1);
  const failing = {
    suite: "cli-demo",
    steps: [{ id: "f", title: "t", cmd: [node, "-e", "process.exit(2)"] }],
  };
  assert.equal(run([], failing).status, 1);
  const passing = { suite: "cli-demo", steps: [{ id: "p", title: "t", cmd: [node, "-e", "0"] }] };
  assert.equal(run(["--strict"], passing).status, 0);
});
