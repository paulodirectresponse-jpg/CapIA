#!/usr/bin/env node
// Valida a evidência de máquina limpa (Windows 10 22H2 e Windows 11, perfil novo, SEM ferramentas
// de desenvolvedor). Gerada por `run-clean-machine.ps1` + atestado do executor humano.
//   node tools/phase6-acceptance/clean-machine/validate.mjs [--file <resultados.json>]
// Sem arquivo ⇒ `pending_external`. Um arquivo só com uma das duas versões do Windows ⇒ `partial`
// (não é aprovação). `accepted` exige a matriz completa, todos os passos `passed` e atestados.
import { fileURLToPath } from "node:url";
import { checker, isSemver, isSha256, runValidator } from "../evidence-lib.mjs";

export const REQUIRED_STEPS = [
  "install",
  "launch",
  "create_project",
  "import",
  "edit",
  "export",
  "uninstall",
];
// Ferramentas cuja presença invalida a premissa "sem dependências de desenvolvedor".
export const DEV_TOOLS = [
  "cargo",
  "rustc",
  "node",
  "npm",
  "pnpm",
  "git",
  "python",
  "cmake",
  "cl",
  "ffmpeg_on_path", // um ffmpeg no PATH mascararia a falta do FFmpeg embutido
];
export const KINDS = ["physical", "vm", "ci_image"];

/** "win10" (22H2, build 19045) | "win11" (build ≥ 22000) | null. */
export function osFamily(os) {
  const build = Number(os?.build);
  if (!Number.isInteger(build)) return null;
  if (/Windows 11/i.test(os.caption ?? "")) return build >= 22000 ? "win11" : null;
  if (/Windows 10/i.test(os.caption ?? ""))
    return build === 19045 && /22H2/i.test(os.display_version ?? "") ? "win10" : null;
  return null;
}

function checkMachine(m, i, now, c, rc) {
  const at = `machines[${i}]`;
  c.filled(m.id, `${at}.id`);
  c.must(KINDS.includes(m.kind), `${at}.kind deve ser ${KINDS.join("|")}`);
  const fam = osFamily(m.os);
  c.must(
    fam !== null,
    `${at}.os: precisa ser Windows 10 22H2 (build 19045) ou Windows 11 (build ≥ 22000), com caption coerente`,
  );
  c.must(m.fresh_user_profile === true, `${at}.fresh_user_profile deve ser true (perfil novo)`);
  for (const t of DEV_TOOLS) {
    if (typeof m.dev_tools?.[t] !== "boolean") c.must(false, `${at}.dev_tools.${t}: ausente`);
    else c.must(m.dev_tools[t] === false, `${at}: máquina NÃO limpa — \`${t}\` presente`);
  }
  c.filled(m.installer?.file, `${at}.installer.file`);
  c.must(isSha256(m.installer?.sha256), `${at}.installer.sha256 inválido (64 hex minúsculos)`);
  if (m.installer?.signed === true)
    c.must(
      m.installer.authenticode_status === "Valid",
      `${at}: installer.signed=true exige authenticode_status "Valid"`,
    );
  else c.must(m.installer?.signed === false, `${at}.installer.signed deve ser true|false`);
  c.must(isSemver(m.app_version), `${at}.app_version inválida`);
  c.must(m.app_version === rc, `${at}.app_version (${m.app_version}) ≠ release_candidate (${rc})`);
  const okStart = c.date(m.started_at, `${at}.started_at`, now);
  const okEnd = c.date(m.finished_at, `${at}.finished_at`, now);
  if (okStart && okEnd)
    c.must(
      Date.parse(m.finished_at) >= Date.parse(m.started_at),
      `${at}: finished_at antes de started_at`,
    );
  const steps = Array.isArray(m.steps) ? m.steps : [];
  for (const id of REQUIRED_STEPS) {
    const s = steps.find((x) => x.id === id);
    if (!c.must(s, `${at}: passo \`${id}\` ausente`)) continue;
    if (s.result === "passed") {
      c.date(s.completed_at, `${at}.${id}.completed_at`, now);
      if (
        okStart &&
        okEnd &&
        typeof s.completed_at === "string" &&
        !Number.isNaN(Date.parse(s.completed_at))
      )
        c.must(
          Date.parse(s.completed_at) >= Date.parse(m.started_at) - 1000 &&
            Date.parse(s.completed_at) <= Date.parse(m.finished_at) + 1000,
          `${at}.${id}.completed_at fora do intervalo da execução`,
        );
    } else if (s.result === "failed") {
      c.must(false, `FALHA ${at} passo \`${id}\`: ${s.notes ?? "(sem notas)"}`);
    } else c.must(false, `${at}.${id}: não executado (result=${s.result})`);
  }
  c.must(
    Number.isInteger(m.export?.file_size_bytes) && m.export.file_size_bytes > 0,
    `${at}.export.file_size_bytes deve ser > 0 (arquivo exportado real)`,
  );
  c.must(
    m.user_project_preserved_after_uninstall === true,
    `${at}: o desinstalador não pode apagar projetos do usuário (user_project_preserved_after_uninstall=true)`,
  );
  return fam;
}

export function evaluate(data, { now }) {
  const c = checker();
  c.must(isSemver(data.release_candidate), "release_candidate inválido");
  c.filled(data.attestation?.performed_by, "attestation.performed_by");
  c.must(data.attestation?.results_are_real === true, "attestation.results_are_real deve ser true");
  c.must(data.attestation?.no_simulation === true, "attestation.no_simulation deve ser true");
  const machines = Array.isArray(data.machines) ? data.machines : [];
  c.must(machines.length > 0, "nenhuma máquina registrada");
  const ids = new Set();
  const families = new Set();
  let physical = 0;
  let unsigned = 0;
  machines.forEach((m, i) => {
    if (ids.has(m.id)) c.must(false, `machines[${i}].id repetido (${m.id})`);
    ids.add(m.id);
    const fam = checkMachine(m, i, now, c, data.release_candidate);
    if (fam) families.add(fam);
    if (m.kind === "physical") physical++;
    if (m.installer?.signed === false) unsigned++;
  });
  const missing = ["win10", "win11"].filter((f) => !families.has(f));
  return {
    problems: c.problems,
    status: missing.length ? "partial" : "accepted",
    machines: machines.length,
    physical_machines: physical,
    os_covered: [...families],
    os_missing: missing,
    unsigned_installers: unsigned,
    note:
      (unsigned
        ? "Há instalador NÃO assinado: isto não atende o critério de instalador assinado. "
        : "") + (missing.length ? `Matriz incompleta: falta ${missing.join(", ")}.` : ""),
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  runValidator({ suite: "clean-machine", evaluate });
