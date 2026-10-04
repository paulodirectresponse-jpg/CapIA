#!/usr/bin/env node
// Valida os resultados reais e diz se o critério humano do ROADMAP está atendido.
// Nunca fabrica: sem arquivos de resultado, o veredito é PENDENTE.
// Uso: node validate-results.mjs results/*.json
import { readFileSync } from "node:fs";

const files = process.argv.slice(2);
const problems = [];
const rows = [];
for (const f of files) {
  let r;
  try {
    r = JSON.parse(readFileSync(f, "utf8"));
  } catch (e) {
    problems.push(`${f}: JSON inválido (${e.message})`);
    continue;
  }
  const stepsDone = (r.steps ?? []).filter((s) => s.done).length;
  const blocking = (r.issues ?? []).filter(
    (i) => i.severity === "S1" && !i.resolved && i.summary,
  ).length;
  if (!r.participant || (r.participant === "P1" && !r.date))
    problems.push(`${f}: participante/data ausentes`);
  if (/AAAA/.test(r.date ?? "")) problems.push(`${f}: data não preenchida (modelo não editado)`);
  rows.push({
    file: f,
    participant: r.participant,
    stepsDone,
    total: r.total_seconds,
    blocking,
    exported: r.exported_mp4_validated === true,
  });
}
const completed = rows.filter((r) => r.stepsDone === 10 && r.exported && r.withinTime);
const distinct = new Set(completed.map((r) => r.participant)).size;
const blockingTotal = rows.reduce((a, r) => a + r.blocking, 0);
console.table(rows);
let verdict;
if (files.length === 0) verdict = "PENDENTE — nenhum resultado real fornecido";
else if (problems.length) verdict = "INVÁLIDO — corrija os arquivos";
else if (distinct >= 3 && blockingTotal === 0)
  verdict = "ATENDIDO — ≥ 3 participantes completaram sem bloqueantes";
else verdict = `NÃO ATENDIDO — completaram ${distinct}/3, bloqueantes abertos: ${blockingTotal}`;
for (const p of problems) console.log(`! ${p}`);
console.log(`\nVeredito do critério humano: ${verdict}`);
process.exit(verdict.startsWith("ATENDIDO") ? 0 : 1);
