import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { evaluateData, evaluateFile } from "../evidence-lib.mjs";
import { evaluate } from "./validate.mjs";

const NOW = Date.parse("2026-10-20T00:00:00Z");
const SHA = "c".repeat(64);
const run = (d) => evaluateData("installer", d, evaluate, NOW);

const signed = (over = {}) => ({
  file: "CapIA_setup.exe",
  kind: "nsis",
  sha256: SHA,
  size_bytes: 123456,
  signed: true,
  authenticode_status: "Valid",
  signer_subject: "CN=Exemplo Ltda",
  thumbprint: "d".repeat(40),
  timestamped: true,
  verified_with: "signtool verify /pa /v",
  ...over,
});
const doc = (over = {}) => ({
  schema: "capia.phase6.installer/1",
  release_candidate: "0.6.0-rc.1",
  built_at: "2026-10-19T12:00:00Z",
  attestation: { performed_by: "Release Eng", results_are_real: true },
  artifacts: [
    signed(),
    { file: "latest.json", kind: "updater_manifest", sha256: SHA, size_bytes: 300 },
  ],
  checksums: { file: "SHA256SUMS", verified: true },
  sbom: { file: "sbom.cdx.json", format: "cyclonedx", sha256: SHA },
  license_report: { file: "licenses.txt", sha256: SHA },
  ffmpeg: { lgpl_build_verified: true, license_notice_included: true },
  third_party_notices_included: true,
  ...over,
});

test("tudo assinado, verificado e com SBOM/licenças ⇒ accepted", () => {
  const r = run(doc());
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "accepted");
  assert.equal(r.unsigned_artifacts, 0);
});

test("artefato não assinado (build de teste) ⇒ partial, não accepted", () => {
  const r = run(doc({ artifacts: [signed({ signed: false, authenticode_status: "NotSigned" })] }));
  assert.deepEqual(r.problems, []);
  assert.equal(r.status, "partial");
  assert.equal(r.unsigned_artifacts, 1);
  assert.match(r.note, /externo/);
});

test("alegar assinatura sem prova coerente é recusado", () => {
  const bad = (patch) => run(doc({ artifacts: [signed(patch)] })).status;
  assert.equal(bad({ authenticode_status: "NotSigned" }), "rejected");
  assert.equal(bad({ thumbprint: "abc" }), "rejected");
  assert.equal(bad({ timestamped: false }), "rejected");
  assert.equal(bad({ signer_subject: "" }), "rejected");
  assert.equal(bad({ verified_with: "" }), "rejected");
  assert.equal(bad({ sha256: "zz" }), "rejected");
  assert.equal(bad({ size_bytes: 0 }), "rejected");
  assert.equal(bad({ kind: "zip" }), "rejected");
});

test("falta instalador, checksums, SBOM, licenças ou aviso LGPL ⇒ rejected", () => {
  assert.equal(
    run(
      doc({
        artifacts: [{ file: "latest.json", kind: "updater_manifest", sha256: SHA, size_bytes: 1 }],
      }),
    ).status,
    "rejected",
  );
  assert.equal(run(doc({ checksums: { file: "SHA256SUMS", verified: false } })).status, "rejected");
  assert.equal(
    run(doc({ sbom: { file: "", format: "cyclonedx", sha256: SHA } })).status,
    "rejected",
  );
  assert.equal(run(doc({ sbom: { file: "s", format: "csv", sha256: SHA } })).status, "rejected");
  assert.equal(run(doc({ license_report: { file: "l", sha256: "x" } })).status, "rejected");
  assert.equal(
    run(doc({ ffmpeg: { lgpl_build_verified: false, license_notice_included: true } })).status,
    "rejected",
  );
  assert.equal(run(doc({ third_party_notices_included: false })).status, "rejected");
});

test("atestado, schema, data e arquivos repetidos", () => {
  assert.equal(
    run(doc({ attestation: { performed_by: "x", results_are_real: false } })).status,
    "rejected",
  );
  assert.equal(run(doc({ schema: "x" })).status, "rejected");
  assert.equal(run(doc({ built_at: "2031-01-01T00:00:00Z" })).status, "rejected");
  assert.equal(run(doc({ artifacts: [signed(), signed()] })).status, "rejected");
});

test("sem arquivo e modelo ⇒ pending_external", () => {
  assert.equal(
    evaluateFile("installer", "/nonexistent/i.json", evaluate, NOW).status,
    "pending_external",
  );
  const tpl = JSON.parse(readFileSync(new URL("./template.json", import.meta.url), "utf8"));
  assert.equal(run(tpl).status, "pending_external");
  delete tpl.template;
  assert.equal(run(tpl).status, "rejected");
});
