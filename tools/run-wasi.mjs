#!/usr/bin/env node
// Executa um módulo `wasm32-wasip1` (CLI) com o WASI do Node, repassando argumentos e stdout.
// Uso: node tools/run-wasi.mjs <arquivo.wasm> [args...]
import { readFileSync } from "node:fs";
import { WASI } from "node:wasi";

const [file, ...args] = process.argv.slice(2);
if (!file) {
  console.error("uso: node tools/run-wasi.mjs <arquivo.wasm> [args...]");
  process.exit(2);
}
const wasi = new WASI({ version: "preview1", args: [file, ...args], env: {}, returnOnExit: true });
const module = await WebAssembly.compile(readFileSync(file));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
