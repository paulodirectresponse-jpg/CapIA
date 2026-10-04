import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Regra inegociável (PHASE3 §46, CLAUDE.md #3): TODA mutação do documento a partir da UI é um
 * comando/transação do Command Engine. Estes testes varrem o código-fonte e falham se um
 * componente/hook falar com o engine por outro caminho.
 */
const SRC = __dirname;

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory()
      ? files(p)
      : p.endsWith(".ts") || p.endsWith(".tsx")
        ? [p]
        : [];
  });
}

const isTest = (f: string) => /\.test\.tsx?$/.test(f);
const prod = files(SRC).filter((f) => !isTest(f));
const rel = (f: string) => f.slice(SRC.length + 1).replaceAll("\\", "/");

describe("UI → engine boundary", () => {
  it("só o controlador chama métodos do cliente que escrevem (execute/undo/redo/projeto/import/relink/export)", () => {
    const writers =
      /\bclient\s*\.(execute|undo|redo|createProject|openProject|closeProject|importAssets|relink|relinkFolder|startExport|cancelExport|verifyAsset)\(/;
    const offenders = prod
      .filter((f) => rel(f) !== "store/controller.ts")
      .filter((f) => /client\b/.test(readFileSync(f, "utf8")))
      .filter((f) => writers.test(readFileSync(f, "utf8")))
      .map(rel);
    expect(offenders).toEqual([]);
  });

  it("componentes só leem do cliente (quadros, codificadores): nenhum comando direto", () => {
    const allowedReads = new Set(["renderFrame", "encoders", "thumbnail", "peaks"]);
    const used: string[] = [];
    for (const f of prod.filter((p) => rel(p).startsWith("components/"))) {
      for (const m of readFileSync(f, "utf8").matchAll(/\bclient\.(\w+)\(/g)) {
        used.push(`${rel(f)}:${m[1] ?? ""}`);
      }
    }
    const bad = used.filter((u) => !allowedReads.has(u.split(":")[1] ?? ""));
    expect(bad).toEqual([]);
  });

  it("nada na UI fala com o transporte nem monta método de engine à mão", () => {
    const offenders = prod
      .filter((f) => !rel(f).startsWith("store/controller.ts"))
      .filter((f) =>
        /transport\.call|callBinary|"command\.(execute|undo|redo)"/.test(readFileSync(f, "utf8")),
      )
      .map(rel);
    expect(offenders).toEqual([]);
  });
});
