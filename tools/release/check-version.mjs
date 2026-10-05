#!/usr/bin/env node
// Fonte única da versão do app (Fase 6, Track C): `[workspace.package] version` do Cargo.toml raiz.
// Falha se qualquer consumidor divergir: tauri.conf.json, package.json (raiz, apps, packages), a fixture
// do contrato engine_info, as versões das dependências de caminho do workspace e crates com versão fixa.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z.-]+)?$/;

export function isSemver(v) {
  return typeof v === "string" && SEMVER.test(v);
}

/** Versão em `[workspace.package]` (primeira ocorrência de `version = "..."` dentro da seção). */
export function workspaceVersion(cargoToml) {
  const m = /\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/.exec(cargoToml);
  if (!m) return null;
  const v = /^\s*version\s*=\s*"([^"]+)"/m.exec(m[1]);
  return v ? v[1] : null;
}

const SKIP = new Set([
  "node_modules",
  "target",
  ".git",
  "dist",
  "spikes",
  "s1-preview-spike",
  "pkg",
]);

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    if (SKIP.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

/** Retorna `{ expected, checked, errors }`. `root` é o diretório raiz do repositório. */
export function checkVersions(root) {
  const errors = [];
  const checked = [];
  const cargoPath = join(root, "Cargo.toml");
  const cargoText = readFileSync(cargoPath, "utf8");
  const expected = workspaceVersion(cargoText);
  if (!expected) {
    return { expected: null, checked, errors: ["Cargo.toml: [workspace.package] version ausente"] };
  }
  if (!isSemver(expected)) errors.push(`Cargo.toml: versão não é semver válido: ${expected}`);
  checked.push("Cargo.toml [workspace.package]");

  for (const m of cargoText.matchAll(/^(capia-[\w-]+)\s*=\s*\{[^}]*version\s*=\s*"([^"]+)"/gm)) {
    checked.push(`Cargo.toml workspace.dependencies.${m[1]}`);
    if (m[2] !== expected) {
      errors.push(`Cargo.toml: dependência ${m[1]} exige ${m[2]} (esperado ${expected})`);
    }
  }

  const jsonVersion = (f, rel) => {
    const v = JSON.parse(readFileSync(f, "utf8")).version;
    checked.push(rel);
    if (v !== expected) errors.push(`${rel}: version ${v} (esperado ${expected})`);
  };

  for (const f of walk(root, [])) {
    const rel = relative(root, f).split("\\").join("/");
    const base = f.split(/[\\/]/).pop();
    if (base === "package.json" || base === "tauri.conf.json") {
      jsonVersion(f, rel);
    } else if (base === "engine_info.json" && rel.includes("fixtures/")) {
      jsonVersion(f, rel);
    } else if (base === "Cargo.toml" && rel !== "Cargo.toml") {
      const pkg = /\[package\]([\s\S]*?)(?:\n\[|$)/.exec(readFileSync(f, "utf8"));
      if (!pkg) continue;
      const own = /^\s*version\s*=\s*"([^"]+)"/m.exec(pkg[1]);
      const ws = /^\s*version\.workspace\s*=\s*true/m.test(pkg[1]);
      if (ws) {
        checked.push(rel);
      } else if (own) {
        checked.push(rel);
        if (own[1] !== expected) {
          errors.push(`${rel}: version fixa ${own[1]} (use version.workspace = true)`);
        }
      } else {
        errors.push(`${rel}: sem version.workspace = true`);
      }
    }
  }
  return { expected, checked, errors };
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  const root = process.argv[2] ?? join(fileURLToPath(import.meta.url), "..", "..", "..");
  if (!existsSync(join(root, "Cargo.toml"))) {
    console.error(`Cargo.toml não encontrado em ${root}`);
    process.exit(2);
  }
  const { expected, checked, errors } = checkVersions(root);
  if (errors.length > 0) {
    console.error(`check:version FALHOU (esperado ${expected ?? "?"}):`);
    for (const e of errors) console.error(`  - ${e}`);
    process.exit(1);
  }
  console.log(`check:version ok — ${expected} (${checked.length} consumidores)`);
}
