#!/usr/bin/env node
// Gera a documentação de API a partir do CATÁLOGO ÚNICO de operações (ADR-102):
//   docs/api/rest-reference.md  — uma seção por operação (rota, scope, schema, exemplo)
//   docs/api/openapi.json       — esqueleto OpenAPI 3.1
//   docs/api/auth-and-scopes.md — só o bloco entre os marcadores GENERATED:scope-matrix
//
// Entrada: JSON do catálogo (`--catalog <arquivo|->`, `-` = stdin). Quando o servidor expuser
// `capia-server catalog`, basta redirecionar a saída para cá; até lá usa-se o fixture
// `tools/docs/fixtures/catalog.json` (transcrito UMA vez do `catalog.rs`, ver fixtures/README.md).
// Não interpreta Rust: o catálogo é dado.
//
//   node tools/docs/gen-api-docs.mjs [--catalog <file|->] [--out-dir docs/api] [--check]
//
// `--check` não escreve: sai 1 se algum arquivo gerado estiver desatualizado (para CI).
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");

export const MARK_BEGIN = "<!-- BEGIN GENERATED:scope-matrix (tools/docs/gen-api-docs.mjs) -->";
export const MARK_END = "<!-- END GENERATED:scope-matrix -->";

// ---- validação do catálogo --------------------------------------------------------------------

/** Falha cedo (com mensagem útil) se o catálogo não tem o formato esperado. */
export function validateCatalog(c) {
  const errs = [];
  if (!c || typeof c !== "object") throw new Error("catálogo inválido: não é um objeto");
  if (!Array.isArray(c.operations) || c.operations.length === 0)
    errs.push("`operations` ausente ou vazio");
  if (!Array.isArray(c.scopes)) errs.push("`scopes` ausente");
  if (!Array.isArray(c.events)) errs.push("`events` ausente");
  const names = new Set();
  const routes = new Set();
  for (const o of c.operations ?? []) {
    for (const k of ["name", "method", "path", "class", "tool", "surface"])
      if (typeof o[k] !== "string" || !o[k]) errs.push(`${o.name ?? "?"}: campo \`${k}\` ausente`);
    if (typeof o.mutating !== "boolean") errs.push(`${o.name}: \`mutating\` ausente`);
    if (!Number.isInteger(o.status)) errs.push(`${o.name}: \`status\` ausente`);
    if (o.scope !== null && !(c.scopes ?? []).includes(o.scope))
      errs.push(`${o.name}: scope desconhecido ${o.scope}`);
    if (names.has(o.name)) errs.push(`${o.name}: nome duplicado`);
    names.add(o.name);
    const r = `${o.method} ${o.path}`;
    if (routes.has(r)) errs.push(`${r}: rota duplicada`);
    routes.add(r);
  }
  if (errs.length) throw new Error(`catálogo inválido:\n- ${errs.join("\n- ")}`);
  return c;
}

// ---- parâmetros e exemplos ---------------------------------------------------------------------

export const pathParams = (op) => [...op.path.matchAll(/\{([a-z_]+)\}/g)].map((m) => m[1]);

/** Divide as propriedades do schema em path / query (GET, DELETE) / body (demais métodos). */
export function splitParams(op) {
  const props = op.schema?.properties ?? {};
  const required = new Set(op.schema?.required ?? []);
  const inPath = new Set(pathParams(op));
  const rest = Object.keys(props).filter((k) => !inPath.has(k));
  const queryLike = op.method === "GET" || op.method === "DELETE";
  return {
    path: [...inPath],
    query: queryLike ? rest : [],
    body: queryLike ? [] : rest,
    props,
    required,
  };
}

const PLACEHOLDER_IDS = {
  project_id: "prj_example",
  run_id: "run_example",
  asset_id: "ast_example",
  upload_id: "upl_example",
  ticket_id: "tkt_example",
  sequence_id: "seq_example",
  export_id: "exp_example",
  webhook_id: "whk_example",
  delivery_id: "dlv_example",
  token_id: "tok_example",
  decision_id: "dec_example",
};

/** Valor de exemplo derivado do schema (mínimo válido; nunca segredo real). */
export function sampleValue(key, schema) {
  if (key in PLACEHOLDER_IDS) return PLACEHOLDER_IDS[key];
  if (!schema || typeof schema !== "object") return "example";
  if (schema.enum) return schema.enum[0];
  switch (schema.type) {
    case "string":
      if (key === "url") return "http://127.0.0.1:9000/capia-webhook";
      if (key === "filename") return "raw.mp4";
      if (key === "name") return "Meu projeto";
      if (key === "content_base64") return "AAAA";
      if (schema.pattern) return `${key.replace(/_id$/, "")}_example`;
      return "example";
    case "integer":
      return Math.max(schema.minimum ?? 0, 1) > (schema.maximum ?? Infinity)
        ? (schema.minimum ?? 0)
        : Math.max(schema.minimum ?? 0, 1);
    case "boolean":
      return false;
    case "array":
      return [sampleValue(`${key}_item`, schema.items)];
    case "object":
      return {};
    default:
      return "example";
  }
}

/** Corpo/consulta de exemplo realistas para as operações centrais; as demais saem do schema. */
const OVERRIDES = {
  "tokens.create": { name: "ci-bot", scopes: ["project:read", "run:read"], expires_in_seconds: 86400 },
  "uploads.create_inline": { filename: "raw.mp4", content_base64: "<BASE64_DO_ARQUIVO>" },
  "assets.import": { upload_id: "upl_example" },
  "commands.preview": {
    label: "Ajustar título",
    expected_revision: 12,
    commands: [{ "...": "comando estruturado do Command Engine (ver docs/COMMAND_SYSTEM.md)" }],
  },
  "commands.apply": { plan_token: "<plan_token devolvido por commands.preview>" },
  "runs.create": {
    brief_text: "Produto: Curso X. Público: iniciantes. Oferta: 30% off. CTA: Inscreva-se.",
    assets: ["ast_example"],
    references: ["ast_reference"],
    start: true,
    budget: { max_cost_micros: 2000000 },
  },
  "runs.approve": { decision_id: "dec_example", option: "approve" },
  "runs.variants": { count: 3, axis: ["hook"] },
  "exports.start": {
    items: [{ sequence: "seq_example", preset: "h264-mp4", name: "vertical-9x16" }],
  },
  "webhooks.create": {
    url: "http://127.0.0.1:9000/capia-webhook",
    events: ["run.completed", "run.failed", "export.completed"],
    description: "receptor local",
  },
  "webhooks.update": { enabled: false },
};

export function exampleRequest(op) {
  const sp = splitParams(op);
  let path = op.path;
  for (const p of sp.path) path = path.replace(`{${p}}`, sampleValue(p, sp.props[p]));
  const data = {};
  const keys = OVERRIDES[op.name] ? [] : [...sp.query, ...sp.body].filter((k) => sp.required.has(k));
  for (const k of keys) data[k] = sampleValue(k, sp.props[k]);
  const body = sp.body.length ? (OVERRIDES[op.name] ?? data) : undefined;
  const query = sp.query.length ? data : {};
  const qs = Object.entries(query)
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`)
    .join("&");
  return { method: op.method, url: path + (qs ? `?${qs}` : ""), body };
}

export function curlFor(op) {
  const r = exampleRequest(op);
  const parts = [`curl -sS -X ${r.method} "http://127.0.0.1:$CAPIA_PORT${r.url}"`];
  if (op.scope) parts.push(`-H "Authorization: Bearer $CAPIA_TOKEN"`);
  if (op.mutating) parts.push(`-H "Idempotency-Key: $(uuidgen)"`);
  if (r.body !== undefined) {
    parts.push(`-H "Content-Type: application/json"`);
    parts.push(`-d '${JSON.stringify(r.body)}'`);
  }
  return parts.join(" \\\n  ");
}

// ---- Markdown ----------------------------------------------------------------------------------

const GROUPS = [
  ["server", "Servidor"],
  ["tokens", "Tokens"],
  ["audit", "Auditoria"],
  ["projects", "Projetos"],
  ["uploads", "Uploads"],
  ["assets", "Assets"],
  ["imports", "Assets"],
  ["sequences", "Sequences e timeline"],
  ["timeline", "Sequences e timeline"],
  ["commands", "Comandos (preview → apply)"],
  ["history", "Comandos (preview → apply)"],
  ["runs", "AI Runs"],
  ["memory", "AI Runs"],
  ["gateway", "AI Runs"],
  ["exports", "Exports e entregáveis"],
  ["deliverables", "Exports e entregáveis"],
  ["webhooks", "Webhooks"],
  ["events", "Eventos"],
];

export function groupOf(op) {
  const prefix = op.name.split(".")[0];
  return GROUPS.find(([p]) => p === prefix)?.[1] ?? "Outros";
}

export function groupedOperations(c) {
  const order = [...new Set(GROUPS.map(([, t]) => t))];
  const map = new Map(order.map((t) => [t, []]));
  for (const op of c.operations) {
    const t = groupOf(op);
    if (!map.has(t)) map.set(t, []);
    map.get(t).push(op);
  }
  return [...map.entries()].filter(([, ops]) => ops.length);
}

const cell = (s) => String(s).replaceAll("|", "\\|");

function paramTable(op) {
  const sp = splitParams(op);
  const rows = [];
  const add = (where, k) => {
    const s = sp.props[k] ?? {};
    const c = [];
    if (s.type) c.push(s.type);
    if (s.enum) c.push(`enum ${s.enum.map((e) => `\`${e}\``).join("/")}`);
    if (s.minimum !== undefined || s.maximum !== undefined) c.push(`${s.minimum ?? ""}..${s.maximum ?? ""}`);
    if (s.minLength !== undefined || s.maxLength !== undefined)
      c.push(`${s.minLength ?? 0}..${s.maxLength ?? "∞"} caracteres`);
    if (s.maxItems !== undefined) c.push(`≤ ${s.maxItems} itens`);
    if (s.maxProperties !== undefined) c.push(`≤ ${s.maxProperties} chaves`);
    if (s.pattern) c.push(`\`${s.pattern}\``);
    rows.push(`| \`${k}\` | ${where} | ${sp.required.has(k) ? "sim" : "não"} | ${cell(c.join(", "))} |`);
  };
  for (const k of sp.path) add("caminho", k);
  for (const k of sp.query) add("query", k);
  for (const k of sp.body) add("corpo", k);
  if (!rows.length) return "_Sem parâmetros._\n";
  return `| Parâmetro | Onde | Obrigatório | Restrições |\n|---|---|---|---|\n${rows.join("\n")}\n`;
}

export function renderOperation(op) {
  const r = exampleRequest(op);
  const surface =
    op.surface === "both"
      ? `REST + MCP (\`${op.tool}\`)`
      : "somente REST (transporte em streaming que o MCP não tem)";
  const lines = [
    `### \`${op.name}\``,
    "",
    op.summary,
    "",
    `- **Rota:** \`${op.method} ${op.path}\``,
    `- **Scope:** ${op.scope ? `\`${op.scope}\`` : "_nenhum (público)_"}`,
    `- **Efeito:** ${op.mutating ? "mutante (aceita `Idempotency-Key`)" : "somente leitura"} · classe de rate limit \`${op.class}\``,
    `- **Sucesso:** HTTP ${op.status}${op.status === 202 ? " (assíncrono: devolve o id; acompanhe por status, SSE ou webhook)" : ""}`,
    `- **Superfície:** ${surface}`,
    `- **Exige o projeto aberto:** ${op.project ? "sim (`{project_id}` precisa ser o projeto aberto no servidor)" : "não"}`,
    "",
    paramTable(op),
    "Exemplo:",
    "",
    "```bash",
    curlFor(op),
    "```",
    "",
  ];
  if (r.body !== undefined) {
    lines.push("Corpo:", "", "```json", JSON.stringify(r.body, null, 2), "```", "");
  }
  return lines.join("\n");
}

export function renderRestReference(c) {
  const out = [
    "# Referência REST (`/v1`)",
    "",
    "> **Gerado** por `node tools/docs/gen-api-docs.mjs` a partir do catálogo único de operações (`crates/capia-server/src/catalog.rs`, ADR-102). **Não edite à mão** — mude o catálogo e regenere. Convenções gerais (envelope de erro, idempotência, paginação, assíncrono) estão em [README.md](README.md); scopes em [auth-and-scopes.md](auth-and-scopes.md).",
    "",
    `O catálogo tem **${c.operations.length} operações** (${c.operations.filter((o) => o.surface === "both").length} também como tools MCP). Toda rota, exceto \`server.health\`, exige \`Authorization: Bearer <token>\`.`,
    "",
    "Os exemplos usam `$CAPIA_PORT` e `$CAPIA_TOKEN` (placeholders; nunca coloque um token real em script versionado). Os ids (`prj_example`, `run_example`…) são ilustrativos. O corpo das respostas não é descrito aqui: o servidor repassa o resultado dos serviços da Engine API e da IA — ver [canonical-flow.md](canonical-flow.md) para os campos que o fluxo usa.",
    "",
    "## Índice",
    "",
  ];
  const groups = groupedOperations(c);
  for (const [title, ops] of groups) {
    out.push(`- **${title}:** ${ops.map((o) => `[\`${o.name}\`](#${anchor(o.name)})`).join(", ")}`);
  }
  out.push("");
  for (const [title, ops] of groups) {
    out.push(`## ${title}`, "");
    for (const op of ops) out.push(renderOperation(op));
  }
  out.push(
    "## Eventos publicados",
    "",
    `Tipos aceitos em \`webhooks.create.events\` (\`*\` assina todos): ${c.events.map((e) => `\`${e}\``).join(", ")}. Ver [webhooks.md](webhooks.md).`,
    "",
  );
  return out.join("\n");
}

const anchor = (name) => name.replaceAll(".", "");

export function renderScopeMatrix(c) {
  const lines = [
    "| Scope | Operações (REST/MCP) | Mutantes |",
    "|---|---|---|",
  ];
  for (const s of c.scopes) {
    const ops = c.operations.filter((o) => o.scope === s);
    const mut = ops.filter((o) => o.mutating).length;
    lines.push(
      `| \`${s}\` | ${ops.map((o) => `\`${o.name}\``).join(", ") || "—"} | ${mut}/${ops.length} |`,
    );
  }
  const pub = c.operations.filter((o) => o.scope === null);
  lines.push(
    `| _(público)_ | ${pub.map((o) => `\`${o.name}\``).join(", ") || "—"} | ${pub.filter((o) => o.mutating).length}/${pub.length} |`,
  );
  lines.push("", "Por rota:", "", "| Operação | Método e rota | Scope | Classe | Mutante |", "|---|---|---|---|---|");
  for (const o of c.operations) {
    lines.push(
      `| \`${o.name}\` | \`${o.method} ${o.path}\` | ${o.scope ? `\`${o.scope}\`` : "—"} | \`${o.class}\` | ${o.mutating ? "sim" : "não"} |`,
    );
  }
  return lines.join("\n");
}

/** Substitui (ou acrescenta ao fim) o bloco gerado dentro de um Markdown escrito à mão. */
export function injectBlock(text, block) {
  const body = `${MARK_BEGIN}\n\n${block}\n\n${MARK_END}`;
  const a = text.indexOf(MARK_BEGIN);
  const b = text.indexOf(MARK_END);
  if (a >= 0 && b > a) return text.slice(0, a) + body + text.slice(b + MARK_END.length);
  if (a >= 0 || b >= 0) throw new Error("marcadores GENERATED:scope-matrix desbalanceados");
  return `${text.replace(/\s*$/, "")}\n\n${body}\n`;
}

// ---- OpenAPI -----------------------------------------------------------------------------------

const ERROR_SCHEMA = {
  type: "object",
  required: ["code", "message", "request_id"],
  properties: {
    code: { type: "string" },
    message: { type: "string" },
    details: {},
    request_id: { type: "string" },
  },
};

export function buildOpenApi(c, version = "0.6.0-rc.1") {
  const paths = {};
  for (const op of c.operations) {
    const sp = splitParams(op);
    const parameters = [];
    for (const p of sp.path)
      parameters.push({ name: p, in: "path", required: true, schema: sp.props[p] ?? { type: "string" } });
    for (const q of sp.query)
      parameters.push({
        name: q,
        in: "query",
        required: sp.required.has(q),
        schema: sp.props[q] ?? { type: "string" },
      });
    if (op.mutating)
      parameters.push({
        name: "Idempotency-Key",
        in: "header",
        required: false,
        schema: { type: "string", minLength: 1, maxLength: 128 },
      });
    const entry = {
      operationId: op.name,
      summary: op.summary,
      tags: [groupOf(op)],
      "x-capia-scope": op.scope,
      "x-capia-mutating": op.mutating,
      "x-capia-class": op.class,
      "x-capia-mcp-tool": op.surface === "both" ? op.tool : null,
      parameters,
      responses: {
        [String(op.status)]: { description: op.status === 202 ? "Aceito (assíncrono)" : "Sucesso" },
        default: {
          description: "Erro",
          content: { "application/json": { schema: { $ref: "#/components/schemas/Error" } } },
        },
      },
    };
    if (op.scope === null) entry.security = [];
    if (sp.body.length) {
      entry.requestBody = {
        required: sp.body.some((k) => sp.required.has(k)),
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: Object.fromEntries(sp.body.map((k) => [k, sp.props[k]])),
              required: sp.body.filter((k) => sp.required.has(k)),
              additionalProperties: false,
            },
          },
        },
      };
    }
    (paths[op.path] ??= {})[op.method.toLowerCase()] = entry;
  }
  return {
    openapi: "3.1.0",
    info: {
      title: "CapIA Local API",
      version,
      description:
        "Esqueleto gerado do catálogo único de operações. Respostas não são descritas campo a campo; ver docs/api/.",
    },
    servers: [{ url: "http://127.0.0.1:{port}", variables: { port: { default: "0" } } }],
    security: [{ bearerAuth: [] }],
    paths,
    components: {
      securitySchemes: { bearerAuth: { type: "http", scheme: "bearer" } },
      schemas: { Error: ERROR_SCHEMA },
    },
  };
}

// ---- CLI ---------------------------------------------------------------------------------------

export function generate(c, { existing = {}, version } = {}) {
  validateCatalog(c);
  const authBase =
    existing["auth-and-scopes.md"] ??
    "# Autenticação e scopes\n\n_(arquivo-base ausente; o bloco abaixo foi gerado)_\n";
  return {
    "rest-reference.md": `${renderRestReference(c)}`,
    "openapi.json": `${JSON.stringify(buildOpenApi(c, version), null, 2)}\n`,
    "auth-and-scopes.md": injectBlock(authBase, renderScopeMatrix(c)),
  };
}

function argOf(flag, def) {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : def;
}

function main() {
  const catalogArg = argOf("--catalog", join(here, "fixtures/catalog.json"));
  const outDir = resolve(root, argOf("--out-dir", "docs/api"));
  const check = process.argv.includes("--check");
  const raw = catalogArg === "-" ? readFileSync(0, "utf8") : readFileSync(catalogArg, "utf8");
  const authPath = join(outDir, "auth-and-scopes.md");
  const existing = existsSync(authPath) ? { "auth-and-scopes.md": readFileSync(authPath, "utf8") } : {};
  const files = generate(JSON.parse(raw), { existing });
  let stale = 0;
  mkdirSync(outDir, { recursive: true });
  for (const [name, content] of Object.entries(files)) {
    const p = join(outDir, name);
    const cur = existsSync(p) ? readFileSync(p, "utf8") : null;
    if (check) {
      if (cur !== content) {
        stale++;
        console.error(`DESATUALIZADO: ${p}`);
      }
    } else if (cur !== content) {
      writeFileSync(p, content);
      console.log(`gerado ${p}`);
    }
  }
  if (check) {
    if (stale) process.exit(1);
    console.log("docs/api gerado está em dia com o catálogo");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
