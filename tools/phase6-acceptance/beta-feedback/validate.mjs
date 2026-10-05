#!/usr/bin/env node
// Valida o feedback REAL de beta e calcula o GATE DE SAÍDA da Fase 6:
//   "nenhum Blocker/Critical conhecido em aberto para o release candidate".
//   node tools/phase6-acceptance/beta-feedback/validate.mjs [--file <feedback.json>] [--min-users N]
// Veredito: `pending_external` (sem arquivo/modelo) · `rejected` (malformado OU gate violado) ·
// `partial` (bem formado, gate sem bloqueadores, mas poucos usuários externos reais) · `accepted`.
// Nunca cria, corrige ou "fecha" relatórios. Usuários internos não contam como beta externo.
import { fileURLToPath } from "node:url";
import { checker, isSemver, runValidator } from "../evidence-lib.mjs";

export const SEVERITIES = ["Blocker", "Critical", "Major", "Minor", "Cosmetic"];
export const REPRODUCIBILITY = ["always", "often", "sometimes", "rare", "once", "unable"];
export const STATUSES = ["open", "fixed", "wontfix", "duplicate", "not_a_bug"];
export const WORKFLOWS = [
  "install",
  "first_project",
  "import",
  "edit",
  "ai_setup",
  "run",
  "approvals",
  "export",
  "api",
  "mcp",
  "webhook",
  "update",
  "uninstall",
  "other",
];
export const PARTICIPANT_KINDS = ["external_beta_user", "internal"];
export const DEFAULT_MIN_USERS = 5;

const BLOCKING = new Set(["Blocker", "Critical"]);

/** -1 | 0 | 1 comparando duas versões semver (pré-release: `-rc.N` < sem sufixo). */
export function cmpSemver(a, b) {
  const parse = (v) => {
    const [core, pre] = v.split("+")[0].split("-");
    return { n: core.split(".").map(Number), pre: pre ?? null };
  };
  const x = parse(a);
  const y = parse(b);
  for (let i = 0; i < 3; i++) if (x.n[i] !== y.n[i]) return x.n[i] < y.n[i] ? -1 : 1;
  if (x.pre === y.pre) return 0;
  if (x.pre === null) return 1;
  if (y.pre === null) return -1;
  const px = x.pre.split(".");
  const py = y.pre.split(".");
  for (let i = 0; i < Math.max(px.length, py.length); i++) {
    if (px[i] === undefined) return -1;
    if (py[i] === undefined) return 1;
    const nx = /^\d+$/.test(px[i]);
    const ny = /^\d+$/.test(py[i]);
    if (nx && ny && Number(px[i]) !== Number(py[i])) return Number(px[i]) < Number(py[i]) ? -1 : 1;
    if (nx !== ny) return nx ? -1 : 1;
    if (!nx && px[i] !== py[i]) return px[i] < py[i] ? -1 : 1;
  }
  return 0;
}

/** O relatório ainda vale como problema aberto para o RC? */
export function isOpenForRc(r, rc) {
  if (r.affects_rc === false) return false;
  if (r.status === "open") return true;
  if (r.status === "wontfix") return true; // Blocker/Critical não se resolvem por "não vamos corrigir"
  if (r.status === "fixed") return !isSemver(r.fixed_in) || cmpSemver(r.fixed_in, rc) > 0;
  return false; // duplicate / not_a_bug (validados à parte)
}

export function evaluate(data, { now }, { minUsers = DEFAULT_MIN_USERS } = {}) {
  const c = checker();
  const rc = data.release_candidate;
  c.must(isSemver(rc), "release_candidate inválido");
  c.filled(data.attestation?.collected_by, "attestation.collected_by");
  c.must(data.attestation?.results_are_real === true, "attestation.results_are_real deve ser true");
  c.must(data.attestation?.no_fabrication === true, "attestation.no_fabrication deve ser true");
  c.date(data.collected_at, "collected_at", now);

  const participants = Array.isArray(data.participants) ? data.participants : [];
  const reports = Array.isArray(data.reports) ? data.reports : [];
  const pIds = new Set();
  participants.forEach((p, i) => {
    const at = `participants[${i}]`;
    c.filled(p.id, `${at}.id`);
    c.must(!pIds.has(p.id), `${at}.id repetido`);
    pIds.add(p.id);
    c.must(
      PARTICIPANT_KINDS.includes(p.kind),
      `${at}.kind deve ser ${PARTICIPANT_KINDS.join("|")}`,
    );
    c.date(p.started_at, `${at}.started_at`, now);
    c.must(
      typeof p.completed_canonical_workflow === "boolean",
      `${at}.completed_canonical_workflow ausente`,
    );
    c.must(
      p.rating === null || (Number.isInteger(p.rating) && p.rating >= 1 && p.rating <= 5),
      `${at}.rating deve ser inteiro 1–5 ou null`,
    );
  });

  const rIds = new Set(reports.map((r) => r.id));
  const seen = new Set();
  reports.forEach((r, i) => {
    const at = `reports[${i}]`;
    c.filled(r.id, `${at}.id`);
    c.must(!seen.has(r.id), `${at}.id repetido`);
    seen.add(r.id);
    c.must(pIds.has(r.reporter), `${at}.reporter não consta em participants`);
    c.date(r.received_at, `${at}.received_at`, now);
    c.must(isSemver(r.app_version), `${at}.app_version inválida`);
    c.must(SEVERITIES.includes(r.severity), `${at}.severity deve ser ${SEVERITIES.join("|")}`);
    c.must(REPRODUCIBILITY.includes(r.reproducibility), `${at}.reproducibility inválida`);
    c.must(WORKFLOWS.includes(r.workflow), `${at}.workflow deve ser ${WORKFLOWS.join("|")}`);
    c.filled(r.summary, `${at}.summary`);
    c.must(STATUSES.includes(r.status), `${at}.status deve ser ${STATUSES.join("|")}`);
    c.must(
      r.rating === null ||
        r.rating === undefined ||
        (Number.isInteger(r.rating) && r.rating >= 1 && r.rating <= 5),
      `${at}.rating deve ser 1–5 ou null`,
    );
    if (BLOCKING.has(r.severity)) {
      if (r.logs?.attached !== true)
        c.filled(
          r.logs?.unavailable_reason,
          `${at}: Blocker/Critical exige logs anexados (bundle) ou logs.unavailable_reason`,
        );
      c.filled(r.steps_to_reproduce, `${at}.steps_to_reproduce`);
    }
    if (r.status === "fixed")
      c.must(isSemver(r.fixed_in), `${at}: status fixed exige fixed_in (versão)`);
    if (r.status === "duplicate")
      c.must(
        rIds.has(r.duplicate_of) && r.duplicate_of !== r.id,
        `${at}: duplicate_of deve apontar outro relatório`,
      );
    if (r.status === "not_a_bug") c.filled(r.justification, `${at}.justification`);
    if (r.affects_rc === false) c.filled(r.affects_rc_reason, `${at}.affects_rc_reason`);
  });

  // ---- gate de saída ---------------------------------------------------------------------------------
  const blockers = reports.filter((r) => BLOCKING.has(r.severity) && isOpenForRc(r, rc));
  // duplicata de um bloqueador aberto continua bloqueando (o original é o que conta, mas não pode sumir)
  for (const r of blockers)
    c.must(false, `GATE: ${r.severity} em aberto para ${rc}: ${r.id} — ${r.summary}`);

  const external = participants.filter((p) => p.kind === "external_beta_user");
  const rated = participants.filter((p) => Number.isInteger(p.rating));
  const bySev = Object.fromEntries(
    SEVERITIES.map((s) => [s, reports.filter((r) => r.severity === s).length]),
  );
  const openBySev = Object.fromEntries(
    SEVERITIES.map((s) => [
      s,
      reports.filter((r) => r.severity === s && isOpenForRc(r, rc)).length,
    ]),
  );
  const enough = external.length >= minUsers;
  return {
    problems: c.problems,
    status: external.length === 0 ? "pending_external" : enough ? "accepted" : "partial",
    gate: blockers.length === 0 ? "passed" : "failed",
    release_candidate: rc,
    external_users: external.length,
    min_external_users: minUsers,
    completed_canonical_workflow: external.filter((p) => p.completed_canonical_workflow).length,
    reports: reports.length,
    by_severity: bySev,
    open_for_rc_by_severity: openBySev,
    mean_rating: rated.length ? rated.reduce((a, p) => a + p.rating, 0) / rated.length : null,
    note: enough
      ? undefined
      : `Só ${external.length} usuário(s) externo(s) real(is); o mínimo documentado é ${minUsers}. O gate de bloqueadores abaixo é informativo até haver beta suficiente.`,
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const i = process.argv.indexOf("--min-users");
  const minUsers = i >= 0 ? Number(process.argv[i + 1]) : DEFAULT_MIN_USERS;
  runValidator({ suite: "beta-feedback", evaluate: (d, ctx) => evaluate(d, ctx, { minUsers }) });
}
