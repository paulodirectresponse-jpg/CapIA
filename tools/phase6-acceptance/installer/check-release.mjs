#!/usr/bin/env node
// Verificações AUTOMÁTICAS do pacote de release (não dependem de máquina limpa nem de certificado):
//   --versions [--expect X.Y.Z[-rc.N]]  versão única propagada (Cargo workspace, package.json raiz,
//                                       tauri.conf.json); com --expect, igual à versão esperada;
//   --docs [--version X]                documentos de release presentes, não vazios e coerentes
//                                       (CHANGELOG com a seção da versão, checklist do RELEASE.md…).
// Sai 0 se tudo ok, 1 se alguma verificação falhou (JSON em stdout). Sem efeitos colaterais.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

const read = (root, rel) =>
  existsSync(join(root, rel)) ? readFileSync(join(root, rel), "utf8") : null;

/** Versões declaradas nas três fontes. */
export function readVersions(root = defaultRoot) {
  const out = {};
  const cargo = read(root, "Cargo.toml");
  const m = cargo?.match(/\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/);
  out["Cargo.toml (workspace.package)"] = m?.[1] ?? null;
  try {
    out["package.json"] = JSON.parse(read(root, "package.json") ?? "{}").version ?? null;
  } catch {
    out["package.json"] = null;
  }
  try {
    out["apps/desktop/src-tauri/tauri.conf.json"] =
      JSON.parse(read(root, "apps/desktop/src-tauri/tauri.conf.json") ?? "{}").version ?? null;
  } catch {
    out["apps/desktop/src-tauri/tauri.conf.json"] = null;
  }
  return out;
}

export function checkVersions(root = defaultRoot, expect) {
  const versions = readVersions(root);
  const problems = [];
  for (const [src, v] of Object.entries(versions)) if (!v) problems.push(`${src}: versão ausente`);
  const set = new Set(Object.values(versions).filter(Boolean));
  if (set.size > 1) problems.push(`versões divergentes: ${JSON.stringify(versions)}`);
  if (expect && [...set].some((v) => v !== expect))
    problems.push(`esperado ${expect}, encontrado ${JSON.stringify(versions)}`);
  return { ok: problems.length === 0, versions, problems };
}

export const RELEASE_DOCS = [
  "CHANGELOG.md",
  "docs/RELEASE.md",
  "docs/KNOWN_ISSUES.md",
  "docs/MIGRATION_COMPAT.md",
  "docs/phase6/IMPL_DOCS_ACCEPTANCE.md",
];

export function checkDocs(root = defaultRoot, version) {
  const problems = [];
  for (const d of RELEASE_DOCS) {
    const t = read(root, d);
    if (t === null) problems.push(`${d}: ausente`);
    else if (t.trim().length < 200) problems.push(`${d}: vazio ou curto demais`);
  }
  const changelog = read(root, "CHANGELOG.md");
  if (changelog && version) {
    const esc = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    if (!new RegExp(`^## \\[${esc}\\]`, "m").test(changelog))
      problems.push(`CHANGELOG.md: sem seção "## [${version}]"`);
  }
  if (changelog && !/^## \[Unreleased\]/m.test(changelog))
    problems.push("CHANGELOG.md: sem seção [Unreleased]");
  const rel = read(root, "docs/RELEASE.md");
  if (rel) {
    const boxes = (rel.match(/^- \[[ x]\]/gm) ?? []).length;
    if (boxes < 10) problems.push(`docs/RELEASE.md: checklist com ${boxes} itens (mínimo 10)`);
    for (const word of ["assinatura", "SBOM", "rollback", "migra"])
      if (!new RegExp(word, "i").test(rel))
        problems.push(`docs/RELEASE.md: não menciona "${word}"`);
  }
  const ki = read(root, "docs/KNOWN_ISSUES.md");
  if (ki && !/pendente|externo/i.test(ki))
    problems.push("docs/KNOWN_ISSUES.md: não lista pendências externas");
  return { ok: problems.length === 0, problems };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const arg = (n) => {
    const i = process.argv.indexOf(n);
    return i >= 0 ? process.argv[i + 1] : undefined;
  };
  const results = {};
  if (process.argv.includes("--versions"))
    results.versions = checkVersions(defaultRoot, arg("--expect"));
  if (process.argv.includes("--docs")) {
    const v = arg("--version") ?? arg("--expect");
    results.docs = checkDocs(defaultRoot, v);
  }
  if (!Object.keys(results).length) {
    console.error("uso: check-release.mjs --versions [--expect X] | --docs [--version X]");
    process.exit(2);
  }
  console.log(JSON.stringify(results, null, 2));
  process.exit(Object.values(results).every((r) => r.ok) ? 0 : 1);
}
