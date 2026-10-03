#!/usr/bin/env node
// Verifica as fronteiras arquiteturais aprovadas (docs/ARCHITECTURE.md §4, ADR-002, ADR-038).
// Falha o CI se: um crate/pacote não está na matriz, depende do que não pode, há ciclo, ou o núcleo
// importa Tauri/IA/render/IO. Adicionar um crate/pacote exige atualizar a matriz abaixo de propósito.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, statSync, existsSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

/** Matriz Rust: dependências permitidas por crate (normal / build / dev). Tudo fora disso é violação. */
export const RUST_RULES = {
  "capia-time": { workspace: [], normal: ["serde"], build: [], dev: ["serde_json"] },
  "capia-model": { workspace: ["capia-time"], normal: ["serde"], build: [], dev: ["serde_json"] },
  "capia-commands": {
    workspace: ["capia-time", "capia-model"],
    normal: ["serde", "serde_json"],
    build: [],
    dev: [],
  },
  "capia-project": {
    workspace: ["capia-time", "capia-model", "capia-commands"],
    normal: ["serde"],
    build: [],
    dev: ["serde_json"],
  },
  "capia-desktop": {
    workspace: ["capia-project"],
    normal: ["tauri"],
    build: ["tauri-build"],
    dev: [],
  },
};

/** Nunca no núcleo (time/model/commands/project): shell, GPU, mídia, banco, rede, provedores de IA. */
export const FORBIDDEN_IN_CORE =
  /^(tauri(-.*)?|wry|tao|wgpu(-.*)?|ffmpeg(-.*)?|libav.*|rusqlite|sqlx|diesel|reqwest|hyper|ureq|openai.*|async-openai|anthropic.*|genai|rig-core|llm(-.*)?)$/;
export const CORE_CRATES = ["capia-time", "capia-model", "capia-commands", "capia-project"];

/** Matriz JS. `external` = prefixos de dependências de terceiros permitidas além das listadas. */
export const JS_RULES = {
  "@capia/engine-bindings": { workspace: [], forbidden: [/^react/, /^@tauri-apps\//] },
  "@capia/editor-ui": { workspace: ["@capia/engine-bindings"], forbidden: [/^@tauri-apps\//] },
  "@capia/desktop": {
    workspace: ["@capia/editor-ui", "@capia/engine-bindings"],
    forbidden: [],
  },
};

export function findCycles(graph) {
  const cycles = [];
  const state = new Map();
  const stack = [];
  const visit = (node) => {
    state.set(node, 1);
    stack.push(node);
    for (const next of graph[node] ?? []) {
      if (state.get(next) === 1) cycles.push([...stack.slice(stack.indexOf(next)), next]);
      else if (!state.has(next)) visit(next);
    }
    stack.pop();
    state.set(node, 2);
  };
  for (const node of Object.keys(graph)) if (!state.has(node)) visit(node);
  return cycles;
}

/** `metadata` = saída de `cargo metadata --no-deps`. Retorna lista de violações (strings). */
export function checkRust(metadata, rules = RUST_RULES) {
  const errors = [];
  const names = new Set(metadata.packages.map((p) => p.name));
  const graph = {};
  for (const pkg of metadata.packages) {
    const rule = rules[pkg.name];
    if (!rule) {
      errors.push(
        `crate "${pkg.name}" não está na matriz de arquitetura (tools/check-architecture.mjs)`,
      );
      continue;
    }
    graph[pkg.name] = [];
    for (const dep of pkg.dependencies) {
      const kind = dep.kind ?? "normal";
      const isWorkspace = names.has(dep.name);
      if (isWorkspace) {
        graph[pkg.name].push(dep.name);
        if (!rule.workspace.includes(dep.name)) {
          errors.push(
            `${pkg.name} não pode depender de ${dep.name} (direção: time → model → commands → project → desktop)`,
          );
        }
      } else if (!(rule[kind] ?? []).includes(dep.name)) {
        const why =
          CORE_CRATES.includes(pkg.name) && FORBIDDEN_IN_CORE.test(dep.name)
            ? ` — "${dep.name}" é shell/GPU/mídia/banco/rede/IA e não pode estar no núcleo (ADR-002)`
            : " — adicione à matriz de propósito, se for decisão aprovada";
        errors.push(`${pkg.name} tem dependência ${kind} não permitida: ${dep.name}${why}`);
      }
    }
  }
  for (const cycle of findCycles(graph)) errors.push(`ciclo de dependência: ${cycle.join(" → ")}`);
  for (const name of Object.keys(rules)) {
    if (!names.has(name)) errors.push(`matriz lista "${name}" mas o crate não existe no workspace`);
  }
  return errors;
}

/** `packages` = [{ name, dependencies, devDependencies }] lidos dos package.json do workspace. */
export function checkJs(packages, rules = JS_RULES) {
  const errors = [];
  const names = new Set(packages.map((p) => p.name));
  const graph = {};
  for (const pkg of packages) {
    const rule = rules[pkg.name];
    if (!rule) {
      errors.push(
        `pacote "${pkg.name}" não está na matriz de arquitetura (tools/check-architecture.mjs)`,
      );
      continue;
    }
    graph[pkg.name] = [];
    const all = { ...pkg.dependencies, ...pkg.devDependencies };
    for (const dep of Object.keys(all)) {
      if (names.has(dep)) {
        graph[pkg.name].push(dep);
        if (!rule.workspace.includes(dep)) errors.push(`${pkg.name} não pode depender de ${dep}`);
      } else if (rule.forbidden.some((re) => re.test(dep))) {
        errors.push(
          `${pkg.name} não pode depender de ${dep} (UI/bindings não conhecem o shell; ADR-002)`,
        );
      }
    }
  }
  for (const cycle of findCycles(graph))
    errors.push(`ciclo de dependência JS: ${cycle.join(" → ")}`);
  return errors;
}

/** Procura imports proibidos nos fontes (pega o que não está declarado em package.json/Cargo.toml). */
export function findForbiddenSourceImports(root) {
  const errors = [];
  const walk = (dir, visitor) => {
    if (!existsSync(dir)) return;
    for (const entry of readdirSync(dir)) {
      if (["node_modules", "dist", "target"].includes(entry)) continue;
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) walk(full, visitor);
      else visitor(full);
    }
  };
  for (const pkgDir of ["packages/engine-bindings/src", "packages/editor-ui/src"]) {
    walk(join(root, pkgDir), (file) => {
      if (!/\.(ts|tsx)$/.test(file)) return;
      if (/from\s+["']@tauri-apps\//.test(readFileSync(file, "utf8"))) {
        errors.push(`${relative(root, file)} importa @tauri-apps (só apps/desktop pode)`);
      }
    });
  }
  for (const crate of CORE_CRATES) {
    walk(join(root, "crates", crate, "src"), (file) => {
      if (!file.endsWith(".rs")) return;
      if (/\b(use|extern crate)\s+(tauri|wgpu|ffmpeg)/.test(readFileSync(file, "utf8"))) {
        errors.push(`${relative(root, file)} usa tauri/wgpu/ffmpeg no núcleo`);
      }
    });
  }
  return errors;
}

function main() {
  const root = join(fileURLToPath(import.meta.url), "..", "..");
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  const jsPackages = [];
  for (const group of ["packages", "apps"]) {
    const base = join(root, group);
    if (!existsSync(base)) continue;
    for (const dir of readdirSync(base)) {
      const manifest = join(base, dir, "package.json");
      if (existsSync(manifest)) jsPackages.push(JSON.parse(readFileSync(manifest, "utf8")));
    }
  }
  const errors = [
    ...checkRust(metadata),
    ...checkJs(jsPackages),
    ...findForbiddenSourceImports(root),
  ];
  if (errors.length > 0) {
    console.error("Violações de arquitetura:\n - " + errors.join("\n - "));
    process.exit(1);
  }
  console.log(
    `Arquitetura ok: ${String(metadata.packages.length)} crates Rust e ${String(jsPackages.length)} pacotes JS dentro das fronteiras, sem ciclos.`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
