#!/usr/bin/env node
// Paridade nativo × WASM do núcleo (ADR-016, ROADMAP Fase 2): o exemplo `parity` executa sequências
// aleatórias determinísticas pelo Command Engine e imprime o digest do documento final. Rodamos o
// mesmo programa nativamente e em wasm32-wasip1 (WASI do Node) e exigimos saída **idêntica**.
import { execFileSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const cases = process.argv[2] ?? "150";
const sh = (cmd, args) =>
  execFileSync(cmd, args, { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });

/** Primeira linha que difere entre duas saídas (ou `null` se idênticas). */
export function firstDifference(a, b) {
  const [la, lb] = [a.split("\n"), b.split("\n")];
  for (let i = 0; i < Math.max(la.length, lb.length); i++) {
    if (la[i] !== lb[i]) return { line: i + 1, native: la[i] ?? "<fim>", wasm: lb[i] ?? "<fim>" };
  }
  return null;
}

export function main() {
  const native = sh("cargo", [
    "run",
    "-q",
    "-p",
    "capia-commands",
    "--example",
    "parity",
    "--",
    cases,
  ]);
  sh("cargo", [
    "build",
    "-q",
    "-p",
    "capia-commands",
    "--example",
    "parity",
    "--target",
    "wasm32-wasip1",
  ]);
  const wasmFile = join(root, "target/wasm32-wasip1/debug/examples/parity.wasm");
  const wasm = sh("node", [join(root, "tools/run-wasi.mjs"), wasmFile, cases]);
  const diff = firstDifference(native, wasm);
  if (diff) {
    console.error(
      `Paridade QUEBRADA na linha ${diff.line}:\n  nativo: ${diff.native}\n  wasm:   ${diff.wasm}`,
    );
    process.exit(1);
  }
  const combined = native.trim().split("\n").at(-1);
  console.log(`Paridade nativo × WASM ok: ${cases} sequências, ${combined}`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
