#!/usr/bin/env node
// Manifesto SHA-256 dos artefatos de release.
//   node tools/release/hashes.mjs <dir-ou-arquivo>... [--out SHA256SUMS.txt] [--json hashes.json] [--verify SHA256SUMS.txt]
// Formato `SHA256SUMS` (compatível com `sha256sum -c`): `<hex>  <nome>`. Nomes relativos ao diretório informado.
import { createHash } from "node:crypto";
import { createReadStream, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { pathToFileURL } from "node:url";

export function sha256File(path) {
  return new Promise((resolve, reject) => {
    const h = createHash("sha256");
    createReadStream(path)
      .on("data", (c) => h.update(c))
      .on("error", reject)
      .on("end", () => resolve(h.digest("hex")));
  });
}

function listFiles(p, root = p) {
  const st = statSync(p);
  if (!st.isDirectory()) return [{ path: p, name: basename(p) }];
  return readdirSync(p).flatMap((n) => {
    const q = join(p, n);
    return statSync(q).isDirectory()
      ? listFiles(q, root)
      : [{ path: q, name: relative(root, q).split("\\").join("/") }];
  });
}

/** @returns {Promise<{name:string, sha256:string, size:number}[]>} ordenado por nome. */
export async function hashAll(inputs) {
  const entries = [];
  for (const i of inputs) {
    for (const f of listFiles(i)) {
      if (/(^|\/)SHA256SUMS(\.txt)?$/.test(f.name) || f.name === "hashes.json") continue;
      entries.push({ name: f.name, sha256: await sha256File(f.path), size: statSync(f.path).size });
    }
  }
  return entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

export const formatSums = (entries) => entries.map((e) => `${e.sha256}  ${e.name}\n`).join("");

/** Confere `SHA256SUMS` contra os arquivos em `dir`. Devolve a lista de divergências (vazia = ok). */
export async function verifySums(sumsText, dir) {
  const problems = [];
  for (const line of sumsText.split(/\r?\n/).filter(Boolean)) {
    const m = /^([0-9a-f]{64}) [ *](.+)$/.exec(line);
    if (!m) {
      problems.push(`linha inválida: ${line}`);
      continue;
    }
    try {
      const got = await sha256File(join(dir, m[2]));
      if (got !== m[1]) problems.push(`${m[2]}: hash diferente`);
    } catch {
      problems.push(`${m[2]}: arquivo ausente`);
    }
  }
  return problems;
}

async function main(argv) {
  const inputs = [];
  let out = null;
  let json = null;
  let verify = null;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--out") out = argv[++i];
    else if (argv[i] === "--json") json = argv[++i];
    else if (argv[i] === "--verify") verify = argv[++i];
    else inputs.push(argv[i]);
  }
  if (verify) {
    const probs = await verifySums(readFileSync(verify, "utf8"), inputs[0] ?? ".");
    if (probs.length) {
      console.error(probs.join("\n"));
      return 1;
    }
    console.log("hashes ok");
    return 0;
  }
  if (inputs.length === 0) {
    console.error(
      "uso: hashes.mjs <dir|arquivo>... [--out SHA256SUMS.txt] [--json hashes.json] | --verify <sums> <dir>",
    );
    return 2;
  }
  const entries = await hashAll(inputs);
  if (entries.length === 0) {
    console.error("nenhum arquivo para calcular");
    return 1;
  }
  const text = formatSums(entries);
  if (out) writeFileSync(out, text);
  if (json) writeFileSync(json, JSON.stringify({ algorithm: "sha256", files: entries }, null, 2));
  process.stdout.write(text);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then((c) => process.exit(c));
}
