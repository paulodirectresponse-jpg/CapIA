#!/usr/bin/env node
// Valida a evidência do pacote de INSTALAÇÃO/ASSINATURA do release candidate: artefatos com hash,
// assinatura Authenticode verificada, SBOM e relatório de licenças, FFmpeg LGPL.
//   node tools/phase6-acceptance/installer/validate.mjs [--file <resultados.json>]
// Sem arquivo ⇒ `pending_external`. Artefatos bem formados mas NÃO assinados (build de teste) ⇒
// `partial` — o gate "instalador assinado" depende de certificado real e continua externo.
import { fileURLToPath } from "node:url";
import { checker, isSemver, isSha256, runValidator } from "../evidence-lib.mjs";

export const KINDS = ["nsis", "msi", "portable", "updater_manifest", "updater_artifact"];
export const SIGNABLE = new Set(["nsis", "msi", "portable", "updater_artifact"]);

export function evaluate(data, { now }) {
  const c = checker();
  c.must(data.schema === "capia.phase6.installer/1", "schema deve ser capia.phase6.installer/1");
  c.must(isSemver(data.release_candidate), "release_candidate inválido");
  c.filled(data.attestation?.performed_by, "attestation.performed_by");
  c.must(data.attestation?.results_are_real === true, "attestation.results_are_real deve ser true");
  c.date(data.built_at, "built_at", now);

  const artifacts = Array.isArray(data.artifacts) ? data.artifacts : [];
  c.must(artifacts.length > 0, "nenhum artefato registrado");
  c.must(
    artifacts.some((a) => ["nsis", "msi"].includes(a.kind)),
    "falta o instalador (nsis|msi)",
  );
  const names = new Set();
  let unsigned = 0;
  artifacts.forEach((a, i) => {
    const at = `artifacts[${i}]`;
    c.filled(a.file, `${at}.file`);
    c.must(!names.has(a.file), `${at}.file repetido`);
    names.add(a.file);
    c.must(KINDS.includes(a.kind), `${at}.kind deve ser ${KINDS.join("|")}`);
    c.must(isSha256(a.sha256), `${at}.sha256 inválido (64 hex minúsculos)`);
    c.must(Number.isInteger(a.size_bytes) && a.size_bytes > 0, `${at}.size_bytes deve ser > 0`);
    if (!SIGNABLE.has(a.kind)) return;
    if (a.signed === true) {
      c.must(
        a.authenticode_status === "Valid",
        `${at}: signed=true exige authenticode_status "Valid"`,
      );
      c.filled(a.signer_subject, `${at}.signer_subject`);
      c.must(
        /^[0-9a-f]{40}$/.test(a.thumbprint ?? ""),
        `${at}.thumbprint deve ser SHA-1 (40 hex minúsculos)`,
      );
      c.must(
        a.timestamped === true,
        `${at}: a assinatura precisa de carimbo de tempo (timestamped=true)`,
      );
      c.filled(a.verified_with, `${at}.verified_with (ex.: "signtool verify /pa /v")`);
    } else {
      c.must(a.signed === false, `${at}.signed deve ser true|false`);
      unsigned++;
    }
  });

  c.must(
    data.checksums?.file === "SHA256SUMS" || /\.sha256$/i.test(data.checksums?.file ?? ""),
    "checksums.file ausente (SHA256SUMS)",
  );
  c.must(
    data.checksums?.verified === true,
    "checksums.verified deve ser true (conferido contra os artefatos)",
  );
  for (const k of ["sbom", "license_report"]) {
    c.filled(data[k]?.file, `${k}.file`);
    c.must(isSha256(data[k]?.sha256), `${k}.sha256 inválido`);
  }
  c.must(["cyclonedx", "spdx"].includes(data.sbom?.format), "sbom.format deve ser cyclonedx|spdx");
  c.must(
    data.ffmpeg?.lgpl_build_verified === true,
    "ffmpeg.lgpl_build_verified deve ser true (build LGPL, dinâmica, sem GPL/nonfree — ADR-032)",
  );
  c.must(
    data.ffmpeg?.license_notice_included === true,
    "ffmpeg.license_notice_included deve ser true",
  );
  c.must(data.third_party_notices_included === true, "third_party_notices_included deve ser true");

  return {
    problems: c.problems,
    status: unsigned > 0 ? "partial" : "accepted",
    release_candidate: data.release_candidate,
    artifacts: artifacts.length,
    unsigned_artifacts: unsigned,
    note:
      unsigned > 0
        ? "Há artefato(s) NÃO assinado(s): build de desenvolvimento/teste. O gate de instalador assinado continua externo (certificado real)."
        : undefined,
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  runValidator({ suite: "installer", evaluate });
