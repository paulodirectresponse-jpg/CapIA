// Biblioteca dos VALIDADORES de evidência humana/hardware da Fase 6 (clean-machine, beta-feedback,
// update, installer). Mesma filosofia de `tools/phase5-acceptance/real-demands/validate.mjs`:
//   - sem arquivo (ou modelo não preenchido) ⇒ `pending_external` — NUNCA aprovado;
//   - arquivo malformado, inconsistente ou que registra falha ⇒ `rejected`;
//   - `partial`: bem formado e sem falhas, mas ainda não cobre a matriz exigida (não é aprovação);
//   - `accepted` só com arquivo real, bem formado e que satisfaz TODAS as regras.
// O validador nunca gera, corrige ou completa resultados. As checagens de plausibilidade (datas não
// futuras, builds do Windows coerentes, hashes bem formados) reduzem erro e fabricação ingênua, mas
// NÃO substituem a confiança no executor humano — por isso exigimos atestados explícitos.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
// CAPIA_P6_OUT redireciona as saídas (os testes usam um diretório temporário).
export const outDir = process.env.CAPIA_P6_OUT ?? join(root, "target/phase6-acceptance");

export const isIso = (s) =>
  typeof s === "string" &&
  /^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$/.test(s) &&
  !Number.isNaN(Date.parse(s));
export const isSha256 = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
export const isSemver = (s) =>
  typeof s === "string" && /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(s);
export const isFilled = (s) =>
  typeof s === "string" &&
  s.trim().length > 0 &&
  !/(TODO|REPLACE|PREENCHER|AAAA|<[^>]+>|\.\.\.)/i.test(s);

/** Acumula problemas com helpers de checagem. */
export function checker() {
  const problems = [];
  return {
    problems,
    must(cond, msg) {
      if (!cond) problems.push(msg);
      return cond;
    },
    filled(v, name) {
      return this.must(isFilled(v), `${name}: ausente ou não preenchido`);
    },
    date(v, name, now) {
      if (!this.must(isIso(v), `${name}: data ISO-8601 inválida`)) return false;
      return this.must(Date.parse(v) <= now + 60_000, `${name}: data no futuro`);
    },
  };
}

/** Avalia o conteúdo já lido. `evaluate(data, ctx)` devolve `{ status, problems, ...detalhes }`. */
export function evaluateData(suite, data, evaluate, now = Date.now()) {
  if (data?.template === true)
    return {
      suite,
      status: "pending_external",
      reason: "o arquivo ainda é o modelo (remova `template` só ao registrar resultados REAIS)",
    };
  const r = evaluate(data, { now });
  const status = r.problems.length ? "rejected" : (r.status ?? "accepted");
  return { suite, ...r, status };
}

export function evaluateFile(suite, file, evaluate, now = Date.now()) {
  if (!existsSync(file))
    return {
      suite,
      status: "pending_external",
      reason: `sem resultados reais em ${file}`,
    };
  let data;
  try {
    data = JSON.parse(readFileSync(file, "utf8"));
  } catch (e) {
    return { suite, status: "rejected", problems: [`JSON inválido: ${e.message}`], file };
  }
  return { ...evaluateData(suite, data, evaluate, now), file };
}

export function writeSummary(summary, dir = outDir) {
  mkdirSync(dir, { recursive: true });
  const s = { ...summary, generated: new Date().toISOString() };
  writeFileSync(join(dir, `${summary.suite}-summary.json`), JSON.stringify(s, null, 2));
  return s;
}

/** CLI padrão dos validadores: `--file <resultados.json>`; imprime o resumo; sai 1 só se `rejected`. */
export function runValidator({ suite, evaluate, argv = process.argv.slice(2) }) {
  const i = argv.indexOf("--file");
  const file = i >= 0 ? resolve(argv[i + 1]) : join(outDir, "evidence", `${suite}-results.json`);
  const summary = writeSummary(evaluateFile(suite, file, evaluate));
  console.log(JSON.stringify(summary, null, 2));
  process.exit(summary.status === "rejected" ? 1 : 0);
}
