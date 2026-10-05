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
      /\bclient\s*\.(execute|undo|undoSelective|redo|createProject|openProject|closeProject|importAssets|relink|relinkFolder|startExport|cancelExport|verifyAsset)\(/;
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

/** Código sem comentários (as regras valem para o que executa, não para a documentação). */
const stripComments = (src: string): string =>
  src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");

describe("IA na UI (Fase 4): sem segredos, sem provider, sem escrita direta", () => {
  const aiFiles = prod.filter((f) => /\/ai[A-Z-]|\/Ai[A-Z]/.test(f.replaceAll("\\", "/")));

  it("só `store/aiController.ts` fala com o `ai.*` do cliente; componentes usam o controlador", () => {
    const offenders = prod
      .filter((f) => rel(f) !== "store/aiController.ts")
      .filter((f) => /\bclient\.ai\b|\bAiClient\b/.test(readFileSync(f, "utf8")))
      // o controlador do editor só repassa o cliente ao AiController
      .filter((f) => rel(f) !== "store/controller.ts")
      .map(rel);
    expect(offenders).toEqual([]);
  });

  it("nenhuma chamada de rede ou URL de provider na UI (providers só existem no engine Rust)", () => {
    const hostile =
      /\bfetch\(|XMLHttpRequest|WebSocket\(|EventSource\(|api\.openai\.com|api\.anthropic\.com|generativelanguage\.googleapis|openrouter\.ai|api\.groq\.com/;
    const offenders = prod
      .filter((f) => rel(f) !== "store/controller.ts")
      .filter((f) => hostile.test(readFileSync(f, "utf8")))
      .map(rel);
    expect(offenders).toEqual([]);
  });

  it("a chave de API nunca vai para localStorage/sessionStorage/prefs nem para o estado", () => {
    expect(aiFiles.length).toBeGreaterThan(2);
    for (const f of aiFiles) {
      const src = stripComments(readFileSync(f, "utf8"));
      expect(src, rel(f)).not.toMatch(/localStorage|sessionStorage|indexedDB|document\.cookie/);
    }
    const ctrl = stripComments(readFileSync(join(SRC, "store/aiController.ts"), "utf8"));
    // `apiKey` só aparece como parâmetro repassado ao cliente (nunca atribuído ao store)
    const uses = [...ctrl.matchAll(/apiKey/g)].length;
    expect(uses).toBeLessThanOrEqual(4);
    expect(ctrl).not.toMatch(/store\.set\([^)]*apiKey/);
    const prefs = readFileSync(join(SRC, "lib/prefs.ts"), "utf8");
    expect(prefs).not.toMatch(/api[_-]?key|secret|credential|token/i);
  });

  it("o campo da chave é `type=password`, sem copiar e sem mostrar o valor salvo", () => {
    const src = readFileSync(join(SRC, "components/AiSettingsDialog.tsx"), "utf8");
    expect(src).toMatch(/type="password"/);
    expect(src).not.toMatch(/clipboard|writeText|api_key\s*[:=]\s*p\./);
  });

  it("a UI não monta comandos do engine para a IA: planos voltam como `plan_token`", () => {
    const comps = prod.filter((f) => /Ai(Panel|SettingsDialog)\.tsx$/.test(f));
    for (const f of comps) {
      expect(readFileSync(f, "utf8"), rel(f)).not.toMatch(
        /"insert_clip"|"delete_clip"|"split_clip"|"add_track"|type:\s*"set_/,
      );
    }
  });

  it("Fase 5: nenhum método `ai.run.*`/`ai.memory.*` montado à mão fora dos bindings; Runs não escrevem na timeline", () => {
    const offenders = prod
      .filter((f) => !rel(f).startsWith("store/controller.ts"))
      .filter((f) =>
        /\.call\(\s*"ai\.(run|memory|gateway|generation)\./.test(readFileSync(f, "utf8")),
      )
      .map(rel);
    expect(offenders).toEqual([]);
    const panel = stripComments(readFileSync(join(SRC, "components/AiRunsPanel.tsx"), "utf8"));
    // o painel nunca chama o cliente: só o controlador (undo de Run passa pelo `stepHistory`)
    expect(panel).not.toMatch(/\bclient\b/);
    expect(panel).not.toMatch(/"insert_clip"|"delete_clip"|"split_clip"|"add_track"|type:\s*"set_/);
    expect(panel).toMatch(/c\.undoRun\(/);
  });
});
