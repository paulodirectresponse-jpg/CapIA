#!/usr/bin/env node
// Valida o pacote HUMANO de demandas reais: `--file <results.json>` (formato de template.json).
// Sem arquivo ⇒ `pending_external` (nunca aprovado). Regras: ≥ 10 demandas distintas, nota inteira 1–5,
// run_id e brief_file presentes, avaliador/provider/modelo/data preenchidos, média ≥ 4,0.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const out = join(root, "target/phase5-acceptance");
mkdirSync(out, { recursive: true });
const i = process.argv.indexOf("--file");
const file =
  i >= 0 ? process.argv[i + 1] : join(root, "target/phase5-acceptance/real-demands-results.json");

export function evaluate(data) {
  const problems = [];
  for (const k of ["provider", "model", "evaluated_at", "rater"])
    if (!data[k]) problems.push(`campo obrigatório ausente: ${k}`);
  const ds = Array.isArray(data.demands) ? data.demands : [];
  const rated = ds.filter((d) => d.rating !== null && d.rating !== undefined);
  const ids = new Set();
  for (const d of rated) {
    if (!Number.isInteger(d.rating) || d.rating < 1 || d.rating > 5)
      problems.push(`${d.id}: nota inválida`);
    if (!d.run_id) problems.push(`${d.id}: run_id ausente`);
    if (!d.brief_file) problems.push(`${d.id}: brief_file ausente`);
    if (ids.has(d.id)) problems.push(`${d.id}: id repetido`);
    ids.add(d.id);
  }
  if (rated.length < 10) problems.push(`mínimo de 10 demandas avaliadas (há ${rated.length})`);
  const mean = rated.length ? rated.reduce((a, d) => a + d.rating, 0) / rated.length : null;
  if (mean !== null && mean < 4.0) problems.push(`média ${mean.toFixed(2)} < 4,0`);
  return { rated: rated.length, mean, problems };
}

let summary;
if (!existsSync(file)) {
  summary = {
    suite: "real-demands",
    status: "pending_external",
    reason: `sem resultados humanos em ${file}`,
    mean: null,
    rated: 0,
  };
} else {
  const r = evaluate(JSON.parse(readFileSync(file, "utf8")));
  summary = {
    suite: "real-demands",
    status: r.problems.length ? "rejected" : "accepted",
    ...r,
    file,
  };
}
summary.generated = new Date().toISOString();
writeFileSync(join(out, "real-demands-summary.json"), JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === "rejected" ? 1 : 0);
