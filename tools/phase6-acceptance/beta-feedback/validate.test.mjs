import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { evaluateData, evaluateFile } from "../evidence-lib.mjs";
import { cmpSemver, evaluate, isOpenForRc } from "./validate.mjs";

const NOW = Date.parse("2026-10-20T00:00:00Z");
const RC = "0.6.0-rc.1";

const participant = (i, over = {}) => ({
  id: `beta-${i}`,
  kind: "external_beta_user",
  started_at: "2026-10-10T10:00:00Z",
  completed_canonical_workflow: true,
  rating: 4,
  ...over,
});
const report = (id, over = {}) => ({
  id,
  reporter: "beta-1",
  received_at: "2026-10-11T10:00:00Z",
  app_version: RC,
  workflow: "export",
  severity: "Minor",
  reproducibility: "sometimes",
  summary: "Texto cortado no botão",
  status: "open",
  rating: null,
  ...over,
});
const doc = (over = {}) => ({
  release_candidate: RC,
  collected_at: "2026-10-15T00:00:00Z",
  attestation: { collected_by: "Equipe CapIA", results_are_real: true, no_fabrication: true },
  participants: [1, 2, 3, 4, 5].map((i) => participant(i)),
  reports: [report("FB-1")],
  ...over,
});
const run = (d, minUsers = 5) =>
  evaluateData("beta-feedback", d, (x, c) => evaluate(x, c, { minUsers }), NOW);

test("5 usuários externos reais e nenhum Blocker/Critical aberto ⇒ accepted, gate passed", () => {
  const r = run(doc());
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "accepted");
  assert.equal(r.gate, "passed");
  assert.equal(r.external_users, 5);
  assert.equal(r.mean_rating, 4);
  assert.equal(r.open_for_rc_by_severity.Minor, 1);
});

test("Blocker ou Critical aberto para o RC viola o gate ⇒ rejected", () => {
  for (const severity of ["Blocker", "Critical"]) {
    const r = run(
      doc({
        reports: [
          report("FB-9", {
            severity,
            steps_to_reproduce: "1) abrir 2) falha",
            logs: { attached: true, bundle_id: "diag-1" },
          }),
        ],
      }),
    );
    assert.equal(r.status, "rejected", severity);
    assert.equal(r.gate, "failed");
    assert.ok(r.problems.some((p) => p.startsWith("GATE")));
  }
});

test("Blocker corrigido NO RC (ou antes) fecha o gate; corrigido em versão posterior não", () => {
  const mk = (fixed_in) =>
    doc({
      reports: [
        report("FB-9", {
          severity: "Blocker",
          status: "fixed",
          fixed_in,
          steps_to_reproduce: "passos",
          logs: { attached: true, bundle_id: "d" },
        }),
      ],
    });
  assert.equal(run(mk("0.6.0-rc.1")).gate, "passed");
  assert.equal(run(mk("0.6.0-beta.9")).gate, "passed");
  assert.equal(run(mk("0.6.0-rc.2")).gate, "failed");
  assert.equal(run(mk("0.6.0")).gate, "failed");
});

test("wontfix de Blocker/Critical NÃO fecha o gate; not_a_bug exige justificativa", () => {
  const w = doc({
    reports: [
      report("FB-9", {
        severity: "Critical",
        status: "wontfix",
        steps_to_reproduce: "p",
        logs: { attached: true },
      }),
    ],
  });
  assert.equal(run(w).gate, "failed");
  const n = doc({
    reports: [
      report("FB-9", {
        severity: "Critical",
        status: "not_a_bug",
        steps_to_reproduce: "p",
        logs: { attached: true },
      }),
    ],
  });
  assert.equal(run(n).status, "rejected"); // sem justification
  n.reports[0].justification = "Comportamento documentado em 08-privacidade";
  assert.equal(run(n).gate, "passed");
  assert.equal(run(n).status, "accepted");
});

test("Blocker/Critical exige logs (ou motivo) e passos para reproduzir", () => {
  const d = doc({
    reports: [report("FB-9", { severity: "Critical", status: "fixed", fixed_in: RC })],
  });
  assert.equal(run(d).status, "rejected");
  d.reports[0].steps_to_reproduce = "passos";
  d.reports[0].logs = { attached: false, unavailable_reason: "app não abre; sem bundle" };
  assert.equal(run(d).status, "accepted");
});

test("poucos usuários externos ⇒ partial; nenhum ⇒ pending_external; internos não contam", () => {
  const few = run(doc({ participants: [participant(1), participant(2)] }));
  assert.equal(few.status, "partial");
  assert.match(few.note, /mínimo documentado/);
  const internal = run(
    doc({
      participants: [1, 2, 3, 4, 5].map((i) => participant(i, { kind: "internal" })),
      reports: [],
    }),
  );
  assert.equal(internal.status, "pending_external");
  assert.equal(run(doc(), 3).status, "accepted");
});

test("validação de formato: enums, reporter desconhecido, duplicatas, datas, notas", () => {
  const bad = (over) => run(doc({ reports: [report("FB-1", over)] }));
  assert.equal(bad({ severity: "Huge" }).status, "rejected");
  assert.equal(bad({ reproducibility: "x" }).status, "rejected");
  assert.equal(bad({ workflow: "x" }).status, "rejected");
  assert.equal(bad({ reporter: "ninguém" }).status, "rejected");
  assert.equal(bad({ summary: "" }).status, "rejected");
  assert.equal(bad({ received_at: "2030-01-01T00:00:00Z" }).status, "rejected");
  assert.equal(bad({ rating: 9 }).status, "rejected");
  assert.equal(bad({ status: "fixed" }).status, "rejected"); // sem fixed_in
  assert.equal(bad({ status: "duplicate", duplicate_of: "FB-1" }).status, "rejected"); // aponta a si
  const dup = doc({ reports: [report("FB-1"), report("FB-1")] });
  assert.equal(run(dup).status, "rejected");
  const noAtt = doc();
  noAtt.attestation.results_are_real = false;
  assert.equal(run(noAtt).status, "rejected");
});

test("affects_rc=false exige motivo e tira o relatório do gate", () => {
  const r = report("FB-9", {
    severity: "Blocker",
    steps_to_reproduce: "p",
    logs: { attached: true },
    affects_rc: false,
  });
  assert.equal(run(doc({ reports: [r] })).status, "rejected"); // sem motivo
  r.affects_rc_reason = "só reproduz no build 0.5.0, já descontinuado";
  const out = run(doc({ reports: [r] }));
  assert.equal(out.gate, "passed");
  assert.equal(isOpenForRc(r, RC), false);
});

test("comparação semver com pré-release", () => {
  assert.equal(cmpSemver("0.6.0-rc.1", "0.6.0-rc.2"), -1);
  assert.equal(cmpSemver("0.6.0-rc.10", "0.6.0-rc.2"), 1);
  assert.equal(cmpSemver("0.6.0", "0.6.0-rc.9"), 1);
  assert.equal(cmpSemver("0.6.0-rc.1", "0.6.0-rc.1"), 0);
  assert.equal(cmpSemver("0.6.1", "0.7.0"), -1);
  assert.equal(cmpSemver("0.6.0-beta.1", "0.6.0-rc.1"), -1);
});

test("sem arquivo / modelo / JSON inválido / CLI", () => {
  assert.equal(
    evaluateFile("beta-feedback", "/nonexistent/fb.json", evaluate, NOW).status,
    "pending_external",
  );
  const tpl = JSON.parse(readFileSync(new URL("./template.json", import.meta.url), "utf8"));
  assert.equal(run(tpl).status, "pending_external");
  delete tpl.template;
  assert.equal(run(tpl).status, "rejected");
  const script = new URL("./validate.mjs", import.meta.url).pathname;
  const out = mkdtempSync(join(tmpdir(), "fb-out-"));
  const env = { ...process.env, CAPIA_P6_OUT: out };
  const a = spawnSync(process.execPath, [script, "--file", "/nonexistent/fb.json"], {
    encoding: "utf8",
    env,
  });
  assert.equal(a.status, 0);
  assert.equal(JSON.parse(a.stdout).status, "pending_external");
  const f = join(mkdtempSync(join(tmpdir(), "fb-")), "r.json");
  const past = new Date(Date.now() - 86_400_000).toISOString(); // o CLI usa o relógio real
  writeFileSync(
    f,
    JSON.stringify(
      doc({
        collected_at: past,
        participants: [participant(1, { started_at: past })],
        reports: [report("FB-1", { received_at: past })],
      }),
    ),
  );
  const b = spawnSync(process.execPath, [script, "--file", f, "--min-users", "1"], {
    encoding: "utf8",
    env,
  });
  assert.equal(b.status, 0);
  assert.equal(JSON.parse(b.stdout).status, "accepted");
  writeFileSync(f, "nope");
  const c = spawnSync(process.execPath, [script, "--file", f], { encoding: "utf8", env });
  assert.equal(c.status, 1);
});
