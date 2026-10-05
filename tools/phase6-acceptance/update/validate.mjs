#!/usr/bin/env node
// Valida a evidência de ATUALIZAÇÃO e ROLLBACK:
//   - atualização ASSINADA entre dois builds RC (quando há certificado de assinatura);
//   - rollback: download interrompido, artefato corrompido, assinatura inválida/adulterada,
//     falha de saúde na inicialização e atualização com Run ativa deixam a instalação anterior
//     recuperável e os projetos do usuário intactos.
//   node tools/phase6-acceptance/update/validate.mjs [--file <resultados.json>]
// Sem arquivo ⇒ `pending_external`. Cenários de rollback completos mas SEM certificado (atualização
// assinada não executada, com motivo) ⇒ `partial` — o gate "update assinado" continua externo.
import { fileURLToPath } from "node:url";
import { checker, isSemver, isSha256, runValidator } from "../evidence-lib.mjs";
import { cmpSemver } from "../beta-feedback/validate.mjs";

export const ROLLBACK_SCENARIOS = [
  "interrupted_download",
  "corrupt_artifact",
  "invalid_signature",
  "startup_health_failure",
  "active_run",
];
export const KINDS = ["physical", "vm", "ci_image"];

export function evaluate(data, { now }) {
  const c = checker();
  c.must(data.schema === "capia.phase6.update/1", "schema deve ser capia.phase6.update/1");
  c.filled(data.attestation?.performed_by, "attestation.performed_by");
  c.must(data.attestation?.results_are_real === true, "attestation.results_are_real deve ser true");
  c.must(data.attestation?.no_simulation === true, "attestation.no_simulation deve ser true");
  c.must(isSemver(data.from_version), "from_version inválida");
  c.must(isSemver(data.to_version), "to_version inválida");
  if (isSemver(data.from_version) && isSemver(data.to_version)) {
    c.must(
      cmpSemver(data.from_version, data.to_version) < 0,
      "to_version deve ser MAIOR que from_version (sem downgrade)",
    );
    c.must(
      /-rc\./.test(data.from_version) && /-rc\./.test(data.to_version),
      "a matriz de atualização é entre dois builds RC (`-rc.N`)",
    );
  }
  c.must(KINDS.includes(data.environment?.kind), `environment.kind deve ser ${KINDS.join("|")}`);
  c.filled(data.environment?.os, "environment.os");

  // ---- atualização assinada -------------------------------------------------------------------------
  const su = data.signed_update ?? {};
  let signedPending = false;
  if (su.performed === true) {
    c.must(
      su.manifest_signature_verified === true,
      "signed_update.manifest_signature_verified deve ser true",
    );
    c.must(
      su.artifact_signature_verified === true,
      "signed_update.artifact_signature_verified deve ser true",
    );
    c.must(isSha256(su.manifest_sha256), "signed_update.manifest_sha256 inválido");
    c.must(isSha256(su.artifact_sha256), "signed_update.artifact_sha256 inválido");
    c.must(
      su.version_after === data.to_version,
      "signed_update.version_after deve ser igual a to_version",
    );
    const a = c.date(su.started_at, "signed_update.started_at", now);
    const b = c.date(su.finished_at, "signed_update.finished_at", now);
    if (a && b)
      c.must(
        Date.parse(su.finished_at) >= Date.parse(su.started_at),
        "signed_update: finished_at antes de started_at",
      );
    c.must(
      su.project_opens_after_update === true,
      "signed_update.project_opens_after_update deve ser true",
    );
    c.must(
      su.settings_and_secret_refs_preserved === true,
      "signed_update.settings_and_secret_refs_preserved deve ser true",
    );
    c.must(su.user_projects_intact === true, "signed_update.user_projects_intact deve ser true");
  } else {
    c.must(su.performed === false, "signed_update.performed deve ser true|false");
    c.filled(
      su.reason,
      "signed_update.reason (por que não foi executada, ex.: certificado indisponível)",
    );
    signedPending = true;
  }

  // ---- rollback -------------------------------------------------------------------------------------
  const scen = Array.isArray(data.rollback?.scenarios) ? data.rollback.scenarios : [];
  for (const id of ROLLBACK_SCENARIOS) {
    const s = scen.find((x) => x.id === id);
    if (!c.must(s, `rollback: cenário \`${id}\` ausente`)) continue;
    if (s.result === "recovered") {
      c.must(
        s.prior_version_launches === true,
        `rollback.${id}: a versão anterior deve abrir (prior_version_launches=true)`,
      );
      c.must(
        s.user_projects_intact === true,
        `rollback.${id}: projetos do usuário devem estar intactos`,
      );
      c.date(s.performed_at, `rollback.${id}.performed_at`, now);
      if (id === "invalid_signature")
        c.must(
          s.update_refused === true,
          "rollback.invalid_signature: o update inválido deve ser RECUSADO (update_refused=true)",
        );
      if (id === "active_run")
        c.must(
          ["deferred", "checkpointed"].includes(s.behavior),
          "rollback.active_run.behavior deve ser deferred|checkpointed",
        );
    } else if (s.result === "failed")
      c.must(false, `FALHA rollback.${id}: ${s.notes ?? "(sem notas)"}`);
    else c.must(false, `rollback.${id}: não executado (result=${s.result})`);
  }
  c.must(
    data.downgrade_blocked === true,
    "downgrade_blocked deve ser true (sem downgrade acidental)",
  );

  return {
    problems: c.problems,
    status: signedPending ? "partial" : "accepted",
    from_version: data.from_version,
    to_version: data.to_version,
    signed_update: su.performed === true ? "performed" : "pending_external",
    rollback_scenarios_recovered: scen.filter((s) => s.result === "recovered").length,
    note: signedPending
      ? "Rollback verificado, mas a atualização ASSINADA não foi executada (sem certificado): o gate de update assinado continua externo."
      : undefined,
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  runValidator({ suite: "update", evaluate });
