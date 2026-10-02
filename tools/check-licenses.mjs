#!/usr/bin/env node
// Verificação inicial de licenças das dependências JS (par do `cargo deny` para Rust; deny.toml).
// Política: docs/PROVENANCE.md §1–§2.6. Detecta GPL/AGPL/non-commercial/sem licença (DENY) e marca
// copyleft fraco/atípico para REVISÃO humana. NÃO substitui auditoria humana nem parecer jurídico.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ALLOW = new Set([
  "MIT",
  "MIT-0",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Apache-2.0",
  "0BSD",
  "CC0-1.0",
  "Unlicense",
  "Zlib",
  "BlueOak-1.0.0",
  "Python-2.0",
  "CC-BY-4.0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "WTFPL",
]);
// Copyleft fraco/por arquivo ou licenças atípicas: permitidas só após olhar (REVIEW).
const REVIEW = /^(MPL-|LGPL-|EPL-|CDDL-|CC-BY-SA|OFL-|Artistic-|SEE LICENSE|CUSTOM)/i;
// Bloqueio: copyleft forte, non-commercial, source-available restritivo, sem licença.
const DENY =
  /(^|[^L])GPL|AGPL|SSPL|BUSL|PolyForm|Non-?Commercial|NC-|Commons-Clause|Elastic-|UNLICENSED|^UNKNOWN$|^NONE$|^$/i;

/** Retorna "allow" | "review" | "deny" para uma expressão SPDX (OR = podemos eleger; AND = ambas). */
export function classifyLicense(expression) {
  const text = String(expression ?? "").trim();
  if (text === "") return "deny";
  const tokens = text.replace(/\(/g, " ( ").replace(/\)/g, " ) ").split(/\s+/).filter(Boolean);
  let i = 0;
  const rank = { allow: 0, review: 1, deny: 2 };
  const worst = (a, b) => (rank[a] >= rank[b] ? a : b);
  const best = (a, b) => (rank[a] <= rank[b] ? a : b);
  const parseOr = () => {
    let result = parseAnd();
    while (tokens[i]?.toUpperCase() === "OR") {
      i += 1;
      result = best(result, parseAnd());
    }
    return result;
  };
  const parseAnd = () => {
    let result = parseAtom();
    while (tokens[i]?.toUpperCase() === "AND") {
      i += 1;
      result = worst(result, parseAtom());
    }
    return result;
  };
  const parseAtom = () => {
    const token = tokens[i];
    i += 1;
    if (token === "(") {
      const inner = parseOr();
      i += 1; // ")"
      return inner;
    }
    let id = token ?? "";
    if (tokens[i]?.toUpperCase() === "WITH") i += 2; // "X WITH exception": a exceção não piora a base
    id = id.replace(/\+$/, "");
    if (DENY.test(id)) return "deny";
    if (ALLOW.has(id)) return "allow";
    if (REVIEW.test(id)) return "review";
    return "review"; // desconhecido não é aprovado automaticamente
  };
  return parseOr();
}

/** `licensesJson` = saída de `pnpm licenses list --json` ({ licença: [{ name, versions }] }). */
export function evaluate(licensesJson, exceptions = {}) {
  const result = { allow: [], review: [], deny: [], excepted: [] };
  for (const [license, packages] of Object.entries(licensesJson)) {
    for (const pkg of packages) {
      for (const version of pkg.versions ?? ["?"]) {
        const id = `${pkg.name}@${version}`;
        const entry = { id, license };
        if (exceptions[id]) result.excepted.push({ ...entry, exception: exceptions[id] });
        else result[classifyLicense(license)].push(entry);
      }
    }
  }
  return result;
}

function main() {
  const root = join(fileURLToPath(import.meta.url), "..", "..");
  const raw = execFileSync("pnpm", ["licenses", "list", "--json"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  const exceptions = JSON.parse(
    readFileSync(join(root, "tools", "license-exceptions.json"), "utf8"),
  ).exceptions;
  const result = evaluate(JSON.parse(raw), exceptions);
  const total =
    result.allow.length + result.review.length + result.deny.length + result.excepted.length;
  console.log(
    `Licenças JS: ${String(total)} pacotes — ${String(result.allow.length)} permitidos, ${String(result.review.length)} para revisão, ${String(result.deny.length)} bloqueados, ${String(result.excepted.length)} com exceção humana.`,
  );
  for (const e of result.review) console.warn(`  REVISAR: ${e.id} (${e.license})`);
  for (const e of result.deny) console.error(`  BLOQUEADO: ${e.id} (${e.license})`);
  if (result.deny.length > 0) {
    console.error(
      "Dependências bloqueadas pela política (PROVENANCE.md). Revisão humana obrigatória.",
    );
    process.exit(1);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
