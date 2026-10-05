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
  // Executor de jobs genérico (ADR-052): threads e relógio, nada de projeto/mídia/SQLite. Fora do WASM.
  "capia-jobs": {
    workspace: [],
    normal: ["serde", "serde_json"],
    build: [],
    dev: [],
  },
  // Render headless determinístico (ADR-063..065): grafo, compositor CPU e mixer PUROS (sem IO, sem
  // FFmpeg); recebem mídia decodificada por um trait. Compila para WASM.
  "capia-render": {
    // `capia-commands` só como dev-dependency: os testes montam documentos pelo Command Engine real
    workspace: ["capia-time", "capia-model", "capia-commands"],
    // `ab_glyph`: rasterização do texto determinístico (Fase 3; Apache-2.0, pura Rust, WASM-ok)
    normal: ["ab_glyph"],
    build: [],
    dev: [],
  },
  // Mídia externa como entrada hostil (ADR-047): probe/ffprobe atrás de um trait. IO de processos;
  // não compila para WASM. Só conhece o tempo (Ticks/Rational).
  "capia-media": {
    workspace: ["capia-time"],
    normal: ["serde", "serde_json", "sha2"],
    build: [],
    dev: [],
  },
  // Preview headless (ADR-066): scheduler/sink sobre o render puro. Sem projeto, mídia nem SQLite.
  "capia-preview": {
    // `capia-commands` só como dev-dependency (testes montam documentos pelo Command Engine real)
    workspace: ["capia-time", "capia-model", "capia-render", "capia-commands"],
    normal: [],
    build: [],
    dev: [],
  },
  // Serviço de decode persistente (ADR-059..061): sessões de ffmpeg + cache de quadros por bytes.
  // Só decodifica: não conhece modelo, assets, projeto nem SQLite. Fora do WASM.
  "capia-decode": {
    workspace: ["capia-time", "capia-media"],
    normal: [],
    build: [],
    dev: [],
  },
  // Segredos (Fase 4, ADR-080): SecretString/SecretStore/redator. Folha: não conhece modelo, projeto,
  // mídia, providers nem rede; só o cofre do SO (Windows Credential Manager) e zeroize.
  "capia-secrets": {
    workspace: [],
    normal: ["zeroize", "keyring"],
    build: [],
    dev: [],
  },
  // Providers de IA (Fase 4, ADR-079/081): contrato canônico, adapters (OpenAI-compatível, Anthropic,
  // Google, Replay, whisper.cpp), registry, router, tool gate. Único crate com HTTP de saída; nunca
  // conhece o documento, o projeto nem a timeline (uma IA só enxerga o motor por tools).
  "capia-ai": {
    workspace: ["capia-secrets"],
    normal: [
      "serde",
      "serde_json",
      "sha2",
      "tokio",
      "reqwest",
      "futures-util",
      "async-trait",
      "bytes",
      "url",
      "base64",
      "jsonschema",
    ],
    build: [],
    dev: [],
  },
  // Inteligência assistida (Fase 4, ADR-082..084): pipelines (transcrição, legendas, silêncio, cenas,
  // Reference Analyzer, Demand Interpreter) e assistente. Cliente do Engine API: escreve só por
  // preview → apply_plan com ator Agent. Roda sem rede (providers off) quando a tarefa é local.
  "capia-intelligence": {
    workspace: [
      "capia-time",
      "capia-model",
      "capia-commands",
      "capia-media",
      "capia-assets",
      "capia-store",
      "capia-project",
      "capia-editor-api",
      "capia-ai",
      "capia-secrets",
    ],
    normal: ["serde", "serde_json", "sha2", "tokio", "zip", "quick-xml", "pdf-extract", "base64"],
    build: [],
    dev: [],
  },
  // Domínio de assets (ADR-046..049): identidade, hash em streaming, import, verify, relink, cache.
  // Sem SQLite e sem Command Engine.
  "capia-assets": {
    workspace: ["capia-time", "capia-model", "capia-media"],
    normal: ["serde", "serde_json", "sha2"],
    build: [],
    dev: [],
  },
  // Única camada com SQLite (ADR-042). Não é "núcleo puro": não compila para WASM.
  "capia-store": {
    workspace: [
      "capia-time",
      "capia-model",
      "capia-commands",
      "capia-assets",
      "capia-media",
      "capia-jobs",
      // só para redigir segredos registrados antes de persistir erros (ADR-080); sem IO próprio
      "capia-secrets",
    ],
    normal: ["serde", "serde_json", "rusqlite", "getrandom"],
    build: [],
    // a auto-dependência de dev habilita a feature `failpoints` só nos testes (ver Cargo.toml)
    dev: [],
  },
  "capia-project": {
    workspace: [
      "capia-time",
      "capia-model",
      "capia-commands",
      "capia-store",
      "capia-assets",
      "capia-media",
      "capia-jobs",
      "capia-decode",
      "capia-render",
      // só dev-dependency: os testes de paridade preview × export usam o scheduler headless
      "capia-preview",
    ],
    normal: ["serde", "serde_json"],
    build: [],
    dev: [],
  },
  // Cliente de linha de comando do engine (headless): só usa a fachada e os tipos do núcleo.
  "capia-cli": {
    workspace: [
      "capia-time",
      "capia-model",
      "capia-commands",
      "capia-store",
      "capia-project",
      "capia-assets",
      "capia-media",
    ],
    normal: ["serde", "serde_json"],
    build: [],
    dev: [],
  },
  // API do editor (Fase 3): fachada JSON transport-agnóstica sobre capia-project; nenhuma regra de
  // edição (tudo passa pelo Command Engine). Consumida pelo shell Tauri e pelo servidor de E2E.
  "capia-editor-api": {
    workspace: [
      "capia-time",
      "capia-model",
      "capia-commands",
      "capia-store",
      "capia-project",
      "capia-assets",
      "capia-media",
    ],
    normal: ["serde", "serde_json"],
    build: [],
    dev: [],
  },
  // Servidor HTTP local SÓ para desenvolvimento/E2E (não distribuído): adaptador fino da editor-api.
  "capia-devserver": {
    workspace: ["capia-editor-api"],
    normal: ["serde_json"],
    build: [],
    dev: [],
  },
  // Funções puras de UX da timeline (snap/grupo/colocação do core) em WASM (ADR-070). Só o núcleo
  // puro; única crate com `unsafe` (borda FFI mínima, sem herdar os lints do workspace).
  "capia-timeline-wasm": {
    workspace: ["capia-time", "capia-model", "capia-commands"],
    normal: ["serde", "serde_json"],
    build: [],
    dev: [],
  },
  // Superfície P2 (SharedBuffer do WebView2, ADR-069): borda COM mínima, só Windows; única crate de
  // produto com `unsafe` além do WASM da timeline. Não conhece engine, projeto nem UI.
  "capia-webview-surface": {
    workspace: [],
    normal: ["tauri", "webview2-com", "windows"],
    build: [],
    dev: [],
  },
  "capia-desktop": {
    workspace: ["capia-project", "capia-editor-api", "capia-webview-surface"],
    normal: ["tauri", "tauri-plugin-dialog", "serde_json"],
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
  // Design system: sem lógica de produto nem dependência do engine.
  "@capia/ui-kit": { workspace: [], forbidden: [/^@tauri-apps\//] },
  // Timeline em canvas + ponte WASM do core (snap/grupo/colocação): só conhece o contrato do engine.
  "@capia/ui-timeline": {
    workspace: ["@capia/engine-bindings"],
    forbidden: [/^@tauri-apps\//],
  },
  "@capia/editor-ui": {
    workspace: ["@capia/engine-bindings", "@capia/ui-kit", "@capia/ui-timeline"],
    forbidden: [/^@tauri-apps\//],
  },
  "@capia/desktop": {
    workspace: ["@capia/editor-ui", "@capia/engine-bindings", "@capia/ui-timeline"],
    forbidden: [],
  },
  // E2E (Playwright) dos fluxos do editor: dirige a app pelo navegador/WebView, sem importar código
  // do produto.
  "@capia/e2e": { workspace: [], forbidden: [] },
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
      // dev-dependency de um crate nele mesmo = truque de features de teste (não é aresta do grafo)
      if (dep.name === pkg.name && kind === "dev") continue;
      const isWorkspace = names.has(dep.name);
      if (isWorkspace) {
        graph[pkg.name].push(dep.name);
        if (!rule.workspace.includes(dep.name)) {
          errors.push(
            `${pkg.name} não pode depender de ${dep.name} (direção: time → model → commands → store → project → cli/desktop)`,
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
