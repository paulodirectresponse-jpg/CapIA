#!/usr/bin/env node
// Compila `capia-timeline-wasm` (wasm32-unknown-unknown) e copia o .wasm para o pacote da timeline
// (ADR-070). O artefato é gerado (gitignored). `--if-missing` pula se já existir.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(root, "packages/ui-timeline/src/generated/capia_timeline.wasm");

if (process.argv.includes("--if-missing") && existsSync(out)) process.exit(0);

execFileSync(
  "cargo",
  ["build", "--release", "--target", "wasm32-unknown-unknown", "-p", "capia-timeline-wasm"],
  { cwd: root, stdio: "inherit" },
);
mkdirSync(dirname(out), { recursive: true });
copyFileSync(join(root, "target/wasm32-unknown-unknown/release/capia_timeline_wasm.wasm"), out);
console.log(`wasm: ${out}`);
