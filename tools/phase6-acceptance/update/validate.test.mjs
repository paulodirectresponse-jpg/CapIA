import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { evaluateData, evaluateFile } from "../evidence-lib.mjs";
import { ROLLBACK_SCENARIOS, evaluate } from "./validate.mjs";

const NOW = Date.parse("2026-10-20T00:00:00Z");
const SHA = "b".repeat(64);
const run = (d) => evaluateData("update", d, evaluate, NOW);

const scenario = (id) => ({
  id,
  result: "recovered",
  performed_at: "2026-10-18T10:00:00Z",
  prior_version_launches: true,
  user_projects_intact: true,
  ...(id === "invalid_signature" ? { update_refused: true } : {}),
  ...(id === "active_run" ? { behavior: "deferred" } : {}),
});
const doc = (over = {}) => ({
  schema: "capia.phase6.update/1",
  attestation: { performed_by: "Fulano", results_are_real: true, no_simulation: true },
  from_version: "0.6.0-rc.1",
  to_version: "0.6.0-rc.2",
  environment: { kind: "physical", os: "Windows 11 23H2" },
  signed_update: {
    performed: true,
    manifest_signature_verified: true,
    artifact_signature_verified: true,
    manifest_sha256: SHA,
    artifact_sha256: SHA,
    version_after: "0.6.0-rc.2",
    started_at: "2026-10-18T09:00:00Z",
    finished_at: "2026-10-18T09:05:00Z",
    project_opens_after_update: true,
    settings_and_secret_refs_preserved: true,
    user_projects_intact: true,
  },
  rollback: { scenarios: ROLLBACK_SCENARIOS.map(scenario) },
  downgrade_blocked: true,
  ...over,
});

test("atualização assinada entre dois RCs + todos os rollbacks recuperados ⇒ accepted", () => {
  const r = run(doc());
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "accepted");
  assert.equal(r.signed_update, "performed");
  assert.equal(r.rollback_scenarios_recovered, 5);
});

test("sem certificado: update assinado não executado (com motivo) ⇒ partial, nunca accepted", () => {
  const r = run(
    doc({ signed_update: { performed: false, reason: "certificado de assinatura indisponível" } }),
  );
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "partial");
  assert.equal(r.signed_update, "pending_external");
  assert.match(r.note, /externo/);
  // sem motivo, não vale
  assert.equal(run(doc({ signed_update: { performed: false } })).status, "rejected");
});

test("assinatura não verificada, hash ruim ou versão final errada ⇒ rejected", () => {
  const mutate = (patch) => run(doc({ signed_update: { ...doc().signed_update, ...patch } }));
  assert.equal(mutate({ artifact_signature_verified: false }).status, "rejected");
  assert.equal(mutate({ manifest_signature_verified: false }).status, "rejected");
  assert.equal(mutate({ artifact_sha256: "abc" }).status, "rejected");
  assert.equal(mutate({ version_after: "0.6.0-rc.1" }).status, "rejected");
  assert.equal(mutate({ project_opens_after_update: false }).status, "rejected");
  assert.equal(mutate({ user_projects_intact: false }).status, "rejected");
  assert.equal(mutate({ finished_at: "2026-10-18T08:00:00Z" }).status, "rejected");
});

test("só vale entre dois RCs e sem downgrade", () => {
  assert.equal(run(doc({ to_version: "0.6.0-rc.1" })).status, "rejected");
  assert.equal(
    run(doc({ from_version: "0.6.0-rc.2", to_version: "0.6.0-rc.1" })).status,
    "rejected",
  );
  assert.equal(run(doc({ from_version: "0.5.0", to_version: "0.6.0-rc.1" })).status, "rejected");
  assert.equal(run(doc({ downgrade_blocked: false })).status, "rejected");
});

test("cada cenário de rollback é obrigatório e precisa ter recuperado", () => {
  for (const id of ROLLBACK_SCENARIOS) {
    const without = doc({
      rollback: { scenarios: ROLLBACK_SCENARIOS.filter((s) => s !== id).map(scenario) },
    });
    assert.equal(run(without).status, "rejected", `sem ${id}`);
    const failed = doc({
      rollback: {
        scenarios: ROLLBACK_SCENARIOS.map((s) =>
          s === id ? { ...scenario(s), result: "failed", notes: "app não abriu" } : scenario(s),
        ),
      },
    });
    const r = run(failed);
    assert.equal(r.status, "rejected", `falha em ${id}`);
    assert.ok(r.problems.some((p) => p.startsWith("FALHA")));
    const notRun = doc({
      rollback: {
        scenarios: ROLLBACK_SCENARIOS.map((s) =>
          s === id ? { id: s, result: "not_run" } : scenario(s),
        ),
      },
    });
    assert.equal(run(notRun).status, "rejected", `não executado ${id}`);
  }
});

test("rollback recuperado mas versão anterior não abre ou projeto danificado ⇒ rejected", () => {
  const mk = (patch) =>
    doc({
      rollback: {
        scenarios: ROLLBACK_SCENARIOS.map((s) =>
          s === "corrupt_artifact" ? { ...scenario(s), ...patch } : scenario(s),
        ),
      },
    });
  assert.equal(run(mk({ prior_version_launches: false })).status, "rejected");
  assert.equal(run(mk({ user_projects_intact: false })).status, "rejected");
});

test("update com assinatura inválida precisa ser RECUSADO; Run ativa: adiar ou checkpoint", () => {
  const mk = (id, patch) =>
    doc({
      rollback: {
        scenarios: ROLLBACK_SCENARIOS.map((s) =>
          s === id ? { ...scenario(s), ...patch } : scenario(s),
        ),
      },
    });
  assert.equal(run(mk("invalid_signature", { update_refused: false })).status, "rejected");
  assert.equal(run(mk("active_run", { behavior: "corrupted" })).status, "rejected");
  assert.equal(run(mk("active_run", { behavior: "checkpointed" })).status, "accepted");
});

test("atestados e ambiente", () => {
  const noAtt = doc();
  noAtt.attestation.no_simulation = false;
  assert.equal(run(noAtt).status, "rejected");
  assert.equal(run(doc({ environment: { kind: "emulator", os: "x" } })).status, "rejected");
  assert.equal(run(doc({ schema: "outro" })).status, "rejected");
});

test("sem arquivo e modelo ⇒ pending_external", () => {
  assert.equal(
    evaluateFile("update", "/nonexistent/u.json", evaluate, NOW).status,
    "pending_external",
  );
  const tpl = JSON.parse(readFileSync(new URL("./template.json", import.meta.url), "utf8"));
  assert.equal(run(tpl).status, "pending_external");
  delete tpl.template;
  assert.equal(run(tpl).status, "rejected");
});
