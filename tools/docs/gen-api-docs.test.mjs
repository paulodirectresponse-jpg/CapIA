import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import {
  MARK_BEGIN,
  MARK_END,
  buildOpenApi,
  curlFor,
  exampleRequest,
  generate,
  injectBlock,
  renderMcpTools,
  renderRestReference,
  renderScopeMatrix,
  splitParams,
  validateCatalog,
} from "./gen-api-docs.mjs";

const here = new URL(".", import.meta.url).pathname;
const catalog = () => JSON.parse(readFileSync(`${here}fixtures/catalog.json`, "utf8"));
const op = (name) => catalog().operations.find((o) => o.name === name);

test("o fixture é um catálogo válido, com nomes e rotas únicos", () => {
  const c = validateCatalog(catalog());
  assert.equal(c.operations.length, 58);
  assert.equal(c.scopes.length, 11);
  assert.equal(c.events.length, 9);
});

test("validateCatalog recusa catálogo malformado (duplicata, scope desconhecido, vazio)", () => {
  const c = catalog();
  assert.throws(() => validateCatalog({}), /inválido/);
  const dup = { ...c, operations: [c.operations[0], c.operations[0]] };
  assert.throws(() => validateCatalog(dup), /duplicad/);
  const bad = structuredClone(c);
  bad.operations[1].scope = "nao:existe";
  assert.throws(() => validateCatalog(bad), /scope desconhecido/);
});

test("a única operação sem scope é o health público", () => {
  const pub = catalog().operations.filter((o) => o.scope === null);
  assert.deepEqual(
    pub.map((o) => o.name),
    ["server.health"],
  );
});

test("splitParams separa caminho, query (GET/DELETE) e corpo (POST/PATCH)", () => {
  const list = splitParams(op("assets.list"));
  assert.deepEqual(list.path, ["project_id"]);
  assert.deepEqual(list.query.sort(), ["after", "limit"]);
  assert.deepEqual(list.body, []);
  const create = splitParams(op("runs.create"));
  assert.deepEqual(create.path, ["project_id"]);
  assert.ok(create.body.includes("brief_text"));
  assert.deepEqual(create.query, []);
});

test("exemplos substituem parâmetros de caminho e respeitam obrigatoriedade", () => {
  const r = exampleRequest(op("runs.approve"));
  assert.equal(r.url, "/v1/projects/prj_example/runs/run_example/approvals");
  assert.deepEqual(r.body, { decision_id: "dec_example", option: "approve" });
  const g = exampleRequest(op("projects.list"));
  assert.equal(g.url, "/v1/projects");
  assert.equal(g.body, undefined);
});

test("curl de operação mutante leva Idempotency-Key; o health público não leva token", () => {
  assert.match(curlFor(op("runs.create")), /Idempotency-Key/);
  assert.match(curlFor(op("runs.create")), /Authorization: Bearer \$CAPIA_TOKEN/);
  assert.doesNotMatch(curlFor(op("server.health")), /Authorization/);
  assert.doesNotMatch(curlFor(op("runs.get")), /Idempotency-Key/);
});

test("nenhum exemplo gerado contém algo que pareça segredo real", () => {
  const text = renderRestReference(catalog());
  assert.doesNotMatch(text, /capia_[A-Za-z0-9]{16,}/);
  assert.doesNotMatch(text, /sk-[A-Za-z0-9]{10,}/);
});

test("a referência REST tem exatamente uma seção por operação", () => {
  const c = catalog();
  const text = renderRestReference(c);
  for (const o of c.operations) {
    assert.equal(text.split(`### \`${o.name}\`\n`).length - 1, 1, o.name);
    assert.ok(text.includes(`\`${o.method} ${o.path}\``), o.path);
  }
});

test("a matriz de scopes cobre todos os scopes e todas as operações", () => {
  const c = catalog();
  const m = renderScopeMatrix(c);
  for (const s of c.scopes) assert.ok(m.includes(`| \`${s}\` |`), s);
  for (const o of c.operations) assert.ok(m.includes(`| \`${o.name}\` |`), o.name);
});

test("openapi: um path+método por operação, operationId único, health sem segurança", () => {
  const c = catalog();
  const api = buildOpenApi(c);
  assert.equal(api.openapi, "3.1.0");
  const ids = [];
  for (const [path, item] of Object.entries(api.paths))
    for (const [method, e] of Object.entries(item)) {
      ids.push(e.operationId);
      assert.ok(c.operations.some((o) => o.path === path && o.method.toLowerCase() === method));
    }
  assert.equal(new Set(ids).size, c.operations.length);
  assert.deepEqual(api.paths["/v1/health"].get.security, []);
  const runs = api.paths["/v1/projects/{project_id}/runs"].post;
  assert.equal(runs["x-capia-scope"], "run:start");
  assert.ok(runs.parameters.some((p) => p.name === "Idempotency-Key"));
  assert.ok(!("project_id" in runs.requestBody.content["application/json"].schema.properties));
  assert.ok(runs.responses["202"]);
});

test("injectBlock substitui só o bloco gerado e é idempotente", () => {
  const base = `# T\n\ntexto manual\n\n${MARK_BEGIN}\n\nvelho\n\n${MARK_END}\n\nrodapé\n`;
  const once = injectBlock(base, "NOVO");
  assert.ok(once.includes("texto manual") && once.includes("rodapé") && once.includes("NOVO"));
  assert.ok(!once.includes("velho"));
  assert.equal(injectBlock(once, "NOVO"), once);
  assert.throws(() => injectBlock(`${MARK_BEGIN} sem fim`, "x"), /desbalanceados/);
  assert.ok(injectBlock("# sem marcadores\n", "B").includes(MARK_END));
});

test("generate é determinístico", () => {
  const a = generate(catalog());
  const b = generate(catalog());
  assert.deepEqual(a, b);
  assert.deepEqual(Object.keys(a).sort(), ["auth-and-scopes.md", "mcp-tools.md", "openapi.json", "rest-reference.md"]);
  JSON.parse(a["openapi.json"]);
});

test("--check passa quando docs/api está em dia com o fixture", () => {
  const r = spawnSync(process.execPath, [`${here}gen-api-docs.mjs`, "--check"], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
});

test("--catalog - lê o catálogo da entrada padrão e --check detecta defasagem", () => {
  const c = catalog();
  c.operations[2].summary = "Resumo alterado para forçar defasagem";
  const r = spawnSync(process.execPath, [`${here}gen-api-docs.mjs`, "--catalog", "-", "--check"], {
    input: JSON.stringify(c),
    encoding: "utf8",
  });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /DESATUALIZADO/);
});

test("a tabela MCP lista as tools com o nome da regra (ponto vira sublinhado) e separa as só-REST", () => {
  const c = catalog();
  const t = renderMcpTools(c);
  assert.ok(t.includes("| `runs_create` | `runs.create`"));
  assert.ok(t.includes("| `uploads_create_inline` |"));
  assert.ok(!t.includes("| `uploads_create` |"));
  assert.match(t, /Só REST[\s\S]*`uploads.create`/);
  for (const o of c.operations.filter((x) => x.surface === "both"))
    assert.ok(t.includes(`| \`${o.tool}\` |`), o.tool);
});
