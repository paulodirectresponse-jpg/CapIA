import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { evaluateData, evaluateFile } from "../evidence-lib.mjs";
import { DEV_TOOLS, REQUIRED_STEPS, evaluate, osFamily } from "./validate.mjs";

const NOW = Date.parse("2026-10-10T00:00:00Z");
const SHA = "a".repeat(64);

const machine = (id, os, overrides = {}) => ({
  id,
  kind: "physical",
  os,
  fresh_user_profile: true,
  dev_tools: Object.fromEntries(DEV_TOOLS.map((t) => [t, false])),
  installer: {
    file: "CapIA_setup.exe",
    sha256: SHA,
    signed: false,
    authenticode_status: "NotSigned",
  },
  app_version: "0.6.0-rc.1",
  started_at: "2026-10-09T10:00:00Z",
  finished_at: "2026-10-09T11:00:00Z",
  steps: REQUIRED_STEPS.map((s) => ({
    id: s,
    result: "passed",
    completed_at: "2026-10-09T10:30:00Z",
    notes: "",
  })),
  export: { file_size_bytes: 4096 },
  user_project_preserved_after_uninstall: true,
  ...overrides,
});
const WIN10 = {
  caption: "Microsoft Windows 10 Pro",
  version: "10.0.19045",
  build: 19045,
  display_version: "22H2",
};
const WIN11 = {
  caption: "Microsoft Windows 11 Pro",
  version: "10.0.22631",
  build: 22631,
  display_version: "23H2",
};
const doc = (machines) => ({
  release_candidate: "0.6.0-rc.1",
  attestation: { performed_by: "Maria Testadora", results_are_real: true, no_simulation: true },
  machines,
});
const run = (d) => evaluateData("clean-machine", d, evaluate, NOW);

test("matriz completa (Win10 22H2 + Win11), tudo passed ⇒ accepted", () => {
  const r = run(doc([machine("w10", WIN10), machine("w11", WIN11)]));
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "accepted");
  assert.deepEqual(r.os_covered.sort(), ["win10", "win11"]);
  assert.equal(r.physical_machines, 2);
  assert.equal(r.unsigned_installers, 2);
  assert.match(r.note, /NÃO assinado/);
});

test("só uma versão do Windows ⇒ partial (não é aprovação)", () => {
  const r = run(doc([machine("w11", WIN11)]));
  assert.equal(r.status, "partial");
  assert.deepEqual(r.os_missing, ["win10"]);
});

test("máquina com ferramenta de desenvolvedor NÃO é limpa ⇒ rejected", () => {
  for (const tool of DEV_TOOLS) {
    const m = machine("w10", WIN10);
    m.dev_tools[tool] = true;
    const r = run(doc([m, machine("w11", WIN11)]));
    assert.equal(r.status, "rejected", tool);
    assert.ok(
      r.problems.some((p) => p.includes(tool)),
      tool,
    );
  }
});

test("flag de dev tool ausente também é problema (não presume limpo)", () => {
  const m = machine("w10", WIN10);
  delete m.dev_tools.cargo;
  assert.equal(run(doc([m, machine("w11", WIN11)])).status, "rejected");
});

test("passo falho registra falha honesta ⇒ rejected; passo não executado ⇒ rejected", () => {
  const f = machine("w10", WIN10);
  f.steps[5] = { id: "export", result: "failed", notes: "sem encoder H.264 aprovado" };
  const r = run(doc([f, machine("w11", WIN11)]));
  assert.equal(r.status, "rejected");
  assert.ok(r.problems.some((p) => p.startsWith("FALHA") && p.includes("export")));
  const n = machine("w10", WIN10);
  n.steps[2] = { id: "create_project", result: "not_run" };
  assert.equal(run(doc([n, machine("w11", WIN11)])).status, "rejected");
});

test("SO inconsistente é recusado (build errado, caption desconhecida)", () => {
  assert.equal(osFamily(WIN10), "win10");
  assert.equal(osFamily(WIN11), "win11");
  assert.equal(
    osFamily({ caption: "Microsoft Windows 10 Pro", build: 19044, display_version: "21H2" }),
    null,
  );
  assert.equal(
    osFamily({ caption: "Microsoft Windows 11 Pro", build: 19045, display_version: "22H2" }),
    null,
  );
  assert.equal(osFamily({ caption: "Ubuntu", build: 22631 }), null);
  const bad = machine("w10", { ...WIN10, build: 19044 });
  assert.equal(run(doc([bad, machine("w11", WIN11)])).status, "rejected");
});

test("atestados, datas e hashes são exigidos", () => {
  const base = () => doc([machine("w10", WIN10), machine("w11", WIN11)]);
  let d = base();
  d.attestation.results_are_real = false;
  assert.equal(run(d).status, "rejected");
  d = base();
  d.attestation.performed_by = "";
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[0].installer.sha256 = "xyz";
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[0].finished_at = "2030-01-01T00:00:00Z"; // futuro
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[0].finished_at = "2026-10-09T09:00:00Z"; // antes do início
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[0].steps[0].completed_at = "2026-10-08T00:00:00Z"; // fora do intervalo
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[0].fresh_user_profile = false;
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[1].id = d.machines[0].id;
  assert.equal(run(d).status, "rejected");
  d = base();
  d.machines[0].app_version = "0.5.0";
  assert.equal(run(d).status, "rejected");
});

test("projeto do usuário apagado pelo desinstalador ou export vazio ⇒ rejected", () => {
  const a = machine("w10", WIN10, { user_project_preserved_after_uninstall: false });
  assert.equal(run(doc([a, machine("w11", WIN11)])).status, "rejected");
  const b = machine("w10", WIN10, { export: { file_size_bytes: 0 } });
  assert.equal(run(doc([b, machine("w11", WIN11)])).status, "rejected");
});

test("instalador marcado signed exige Authenticode Valid", () => {
  const m = machine("w10", WIN10, {
    installer: { file: "x.exe", sha256: SHA, signed: true, authenticode_status: "NotSigned" },
  });
  assert.equal(run(doc([m, machine("w11", WIN11)])).status, "rejected");
  const ok = machine("w10", WIN10, {
    installer: { file: "x.exe", sha256: SHA, signed: true, authenticode_status: "Valid" },
  });
  const r = run(doc([ok, machine("w11", WIN11)]));
  assert.equal(r.status, "accepted");
  assert.equal(r.unsigned_installers, 1);
});

test("sem arquivo e modelo não preenchido ⇒ pending_external, nunca aprovado", () => {
  const r = evaluateFile("clean-machine", "/nonexistent/cm.json", evaluate, NOW);
  assert.equal(r.status, "pending_external");
  const tpl = JSON.parse(readFileSync(new URL("./template.json", import.meta.url), "utf8"));
  assert.equal(run(tpl).status, "pending_external");
  // o modelo sem a marca `template` NÃO passa (atestados falsos, passos não executados)
  delete tpl.template;
  assert.equal(run(tpl).status, "rejected");
});

test("CLI: sem arquivo exit 0 + pending_external; JSON inválido exit 1 + rejected", () => {
  const script = new URL("./validate.mjs", import.meta.url).pathname;
  const tmp = mkdtempSync(join(tmpdir(), "cm-out-"));
  const a = spawnSync(process.execPath, [script, "--file", "/nonexistent/cm.json"], {
    encoding: "utf8",
    env: { ...process.env, CAPIA_P6_OUT: tmp },
  });
  assert.equal(a.status, 0);
  assert.equal(JSON.parse(a.stdout).status, "pending_external");
  const f = join(mkdtempSync(join(tmpdir(), "cm-")), "r.json");
  writeFileSync(f, "{ não é json");
  const b = spawnSync(process.execPath, [script, "--file", f], {
    encoding: "utf8",
    env: { ...process.env, CAPIA_P6_OUT: tmp },
  });
  assert.equal(b.status, 1);
  assert.equal(JSON.parse(b.stdout).status, "rejected");
});
