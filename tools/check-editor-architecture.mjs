#!/usr/bin/env node
// Fronteiras do editor OpenCut integrado (apps/editor/apps/web/src) — Fase 2A.
//  1. AgentSurface (única porta estruturada de escrita além da UI) só enxerga domínio: nunca
//     React/Next/componentes/serviços de renderer/storage.
//  2. Domínio (commands/, timeline/, core/) não importa UI nem AgentSurface.
//  3. Só `app/` (página do editor) liga a ponte; o resto não importa `@/agent-surface`.
//  4. Local-first: nenhum import de serviços SaaS removidos (auth, DB, CMS, analytics, botid).
import { readdirSync, readFileSync, statSync, existsSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const SRC = "apps/editor/apps/web/src";

const SAAS = [
  /^better-auth/, /^drizzle-orm/, /^drizzle-kit/, /^@upstash\//, /^botid/, /^postgres$/, /^pg$/,
  /^@content-collections\//, /^@opennextjs\//, /^feed$/,
];

/** Regras por prefixo de diretório. `deny` = padrões de import proibidos. */
export const RULES = [
  {
    dir: "agent-surface",
    except: [/__tests__\//],
    deny: [/^react/, /^next(\/|$)/, /^@\/components\//, /^@\/services\/(renderer|storage)\//, /^@\/app\//, /^@hugeicons\//],
    why: "AgentSurface só pode depender do domínio (core/commands/timeline/utils/wasm/media types)",
  },
  {
    dir: "commands",
    except: [/__tests__\//],
    deny: [/^react/, /^next(\/|$)/, /^@\/components\//, /^@\/app\//, /^@\/agent-surface/],
    why: "Commands são domínio: sem UI e sem AgentSurface",
  },
  {
    dir: "timeline",
    except: [/components\//, /hooks\//, /controllers\//, /__tests__\//],
    deny: [/^@\/components\//, /^@\/app\//, /^@\/agent-surface/],
    why: "domínio da timeline não importa UI nem AgentSurface",
  },
  {
    dir: "core",
    except: [/__tests__\//],
    deny: [/^@\/components\//, /^@\/app\//, /^@\/agent-surface/],
    why: "EditorCore não importa UI nem AgentSurface",
  },
];

const IMPORT = /(?:from\s+|import\s*\(\s*|import\s+)["']([^"']+)["']/g;

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* walk(full);
    else if (/\.(ts|tsx)$/.test(entry)) yield full;
  }
}

export function importsOf(source) {
  return [...source.matchAll(IMPORT)].map((m) => m[1]);
}

export function checkFile({ rel, source }) {
  const errors = [];
  const imports = importsOf(source);
  for (const rule of RULES) {
    if (!rel.startsWith(`${rule.dir}/`)) continue;
    if (rule.except?.some((re) => re.test(rel))) continue;
    for (const spec of imports) {
      if (rule.deny.some((re) => re.test(spec))) {
        errors.push(`${SRC}/${rel}: import "${spec}" proibido (${rule.why})`);
      }
    }
  }
  for (const spec of imports) {
    if (SAAS.some((re) => re.test(spec))) {
      errors.push(`${SRC}/${rel}: import "${spec}" — serviço SaaS removido (local-first)`);
    }
    if (
      /^@\/agent-surface/.test(spec) &&
      !rel.startsWith("agent-surface/") &&
      !rel.startsWith("app/") &&
      !/__tests__\//.test(rel)
    ) {
      errors.push(`${SRC}/${rel}: só app/ e testes podem importar @/agent-surface`);
    }
  }
  return errors;
}

export function main(root) {
  const base = join(root, SRC);
  if (!existsSync(base)) {
    console.error(`${SRC} não existe`);
    return 1;
  }
  const errors = [];
  for (const file of walk(base)) {
    errors.push(...checkFile({ rel: relative(base, file).replaceAll("\\", "/"), source: readFileSync(file, "utf8") }));
  }
  if (errors.length) {
    console.error("Violações de arquitetura do editor:\n - " + errors.join("\n - "));
    return 1;
  }
  console.log("Arquitetura do editor ok (AgentSurface/commands/timeline/core isolados da UI; sem SaaS).");
  return 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exit(main(process.cwd()));
}
