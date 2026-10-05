#!/usr/bin/env node
// Valida a evidência de PARIDADE UI × REST × MCP do fluxo canônico: a MESMA tarefa executada pelas
// três superfícies deve chegar ao mesmo estado autoritativo. Compara ESTADO, não texto de resposta:
// revisão do projeto, sequences, clips, variantes, estado da Run, sondagem do export e webhook.
//   node tools/phase6-acceptance/external-flow/validate-parity.mjs [--file <resultados.json>]
// O validador RECALCULA a igualdade (não confia em `equal: true` do arquivo). Sem arquivo ⇒
// `pending_external`. Resultados reais exigem um servidor real e a UI; nada aqui os fabrica.
import { fileURLToPath } from "node:url";
import { checker, runValidator } from "../evidence-lib.mjs";

export const SURFACES = ["ui", "rest", "mcp"];
// Campos comparados entre as superfícies (estado autoritativo do projeto/Run/export).
export const COMPARED = [
  "project_revision",
  "sequences",
  "clips",
  "variants",
  "run_status",
  "export.codec",
  "export.width",
  "export.height",
  "export.duration_ticks",
];

const get = (o, path) => path.split(".").reduce((a, k) => a?.[k], o);

export function evaluate(data, { now }) {
  const c = checker();
  c.must(data.schema === "capia.phase6.parity/1", "schema deve ser capia.phase6.parity/1");
  c.filled(data.attestation?.performed_by, "attestation.performed_by");
  c.must(data.attestation?.results_are_real === true, "attestation.results_are_real deve ser true");
  c.date(data.performed_at, "performed_at", now);
  c.filled(data.task?.description, "task.description");
  c.must(
    data.task?.same_inputs_for_all_surfaces === true,
    "task.same_inputs_for_all_surfaces deve ser true (mesmo briefing, bruto, referência e política)",
  );

  const diffs = [];
  const state = {};
  for (const s of SURFACES) {
    const v = data.surfaces?.[s];
    if (!c.must(v && typeof v === "object", `surfaces.${s} ausente`)) continue;
    state[s] = v;
    c.must(
      Number.isInteger(v.project_revision) && v.project_revision >= 0,
      `surfaces.${s}.project_revision inválido`,
    );
    for (const k of ["sequences", "clips", "variants"])
      c.must(Number.isInteger(v[k]) && v[k] >= 0, `surfaces.${s}.${k} inválido`);
    c.must(
      v.run_status === "completed",
      `surfaces.${s}.run_status deve ser completed (é ${v.run_status})`,
    );
    c.must(
      Number.isInteger(v.export?.duration_ticks) && v.export.duration_ticks > 0,
      `surfaces.${s}.export.duration_ticks inválido`,
    );
    c.filled(v.export?.codec, `surfaces.${s}.export.codec`);
    c.must(
      Number.isInteger(v.export?.width) && v.export.width > 0,
      `surfaces.${s}.export.width inválido`,
    );
    c.must(
      Number.isInteger(v.export?.height) && v.export.height > 0,
      `surfaces.${s}.export.height inválido`,
    );
  }
  if (Object.keys(state).length === SURFACES.length) {
    for (const field of COMPARED) {
      const vals = SURFACES.map((s) => get(state[s], field));
      if (new Set(vals.map((x) => JSON.stringify(x))).size > 1)
        diffs.push({ field, values: Object.fromEntries(SURFACES.map((s, i) => [s, vals[i]])) });
    }
    for (const d of diffs) c.must(false, `DIVERGÊNCIA em ${d.field}: ${JSON.stringify(d.values)}`);
  }
  if (data.equal === true && diffs.length)
    c.must(false, "o arquivo afirma equal=true mas há divergências");

  c.must(
    data.webhook?.received === true,
    "webhook.received deve ser true (webhook de conclusão recebido)",
  );
  c.must(data.webhook?.signature_verified === true, "webhook.signature_verified deve ser true");
  c.must(
    data.rest_and_mcp_used_same_token_scopes === true,
    "rest_and_mcp_used_same_token_scopes deve ser true (mesmos scopes nas duas superfícies)",
  );
  return {
    problems: c.problems,
    status: "accepted",
    compared_fields: COMPARED.length,
    divergences: diffs,
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  runValidator({ suite: "external-flow-parity", evaluate });
