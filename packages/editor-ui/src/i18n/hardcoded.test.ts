import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Heurística (PHASE3 §41): texto visível ao usuário em JSX deve passar por `t(...)`. Procura texto
 * literal entre tags (`>Texto<`) e atributos de acessibilidade/título literais em componentes.
 */
const DIR = join(__dirname, "..", "components");
const files = readdirSync(DIR)
  .filter((f) => f.endsWith(".tsx"))
  .map((f) => join(DIR, f));

// literais permitidos: símbolos, unidades, nomes próprios/técnicos que não se traduzem
const ALLOWED = new Set([
  "CapIA",
  "OpenAI",
  "Anthropic",
  "Google Gemini",
  "OpenRouter",
  "×",
  "…",
  "•",
  "—",
  "dB",
  "fps",
  "ms",
  "px",
  "WAV",
  "RGBA",
  "H.264",
]);

describe("textos fixos em componentes", () => {
  it("não há texto visível literal fora de t(...)", () => {
    const offenders: string[] = [];
    for (const f of files) {
      const src = readFileSync(f, "utf8");
      if (!statSync(f).isFile()) continue;
      for (const m of src.matchAll(/>([^<>{}\n]*[A-Za-zÀ-ÿ]{3,}[^<>{}\n]*)</g)) {
        const text = (m[1] ?? "").trim();
        if (!text || ALLOWED.has(text)) continue;
        // ignora genéricos TS (`Array<string>`), operadores e comparações
        if (/[|&=;()]|=>/.test(text)) continue;
        offenders.push(`${f.split("/").pop() ?? f}: "${text}"`);
      }
      for (const m of src.matchAll(
        /\b(?:aria-label|title|placeholder|label)="([^"{}]*[A-Za-zÀ-ÿ]{3,}[^"{}]*)"/g,
      )) {
        const text = m[1] ?? "";
        if (ALLOWED.has(text) || /^[A-Z#]+$/.test(text) || text === "#RRGGBB" || text === "7/5")
          continue;
        offenders.push(`${f.split("/").pop() ?? f}: attr "${text}"`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
