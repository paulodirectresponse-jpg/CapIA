// Testa só as partes puras do gerador do projeto de exemplo (não roda ffmpeg nem a CLI).
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";
import {
  BRIEFING,
  cliPlan,
  findOnPath,
  mediaSpecs,
  parseArgs,
  resolveCli,
} from "./make-sample.mjs";

const out = resolve("/tmp/x-sample");

test("toda mídia é sintética (fonte lavfi / color / concat), sem entrada de arquivo externo", () => {
  const specs = mediaSpecs(out);
  assert.ok(specs.length >= 5);
  for (const s of specs) {
    const inputs = s.ffArgs.flatMap((a, i) => (s.ffArgs[i - 1] === "-i" ? [a] : []));
    assert.ok(inputs.length > 0, s.name);
    for (const i of inputs)
      assert.match(i, /^(testsrc2|sine|mandelbrot|gradients|color|anoisesrc)/, `${s.name}: ${i}`);
    assert.ok(s.ffArgs.includes("-f") && s.ffArgs.includes("lavfi"), s.name);
    assert.ok(s.ffArgs.at(-1).startsWith(join(out, "media")), s.name);
  }
});

test("há bruto, referência, música e logo; nomes únicos; papéis corretos", () => {
  const specs = mediaSpecs(out);
  assert.equal(new Set(specs.map((s) => s.name)).size, specs.length);
  assert.ok(specs.some((s) => s.role === "reference" && s.name.startsWith("reference")));
  assert.ok(specs.filter((s) => s.role === "raw").length >= 4);
  assert.ok(specs.some((s) => s.name === "music.wav"));
  assert.ok(specs.some((s) => s.name === "logo.png"));
});

test("o plano da CLI cria o projeto, importa cada mídia e lista os assets (ordem estável)", () => {
  const specs = mediaSpecs(out);
  const plan = cliPlan(out, specs);
  assert.deepEqual(plan[0].args, ["create", join(out, "sample.capia")]);
  assert.equal(plan.length, specs.length + 2);
  for (const [i, s] of specs.entries()) {
    assert.deepEqual(plan[i + 1].args, [
      "asset",
      "import",
      join(out, "sample.capia"),
      join(out, "media", s.name),
    ]);
  }
  assert.deepEqual(plan.at(-1).args.slice(0, 2), ["asset", "list"]);
});

test("briefing é texto sintético com os campos do DemandSpec e sem marca real", () => {
  for (const campo of [
    "Produto:",
    "Público:",
    "Oferta:",
    "Objetivo:",
    "Tom:",
    "Plataforma:",
    "Duração:",
    "Chamada para ação:",
  ])
    assert.ok(BRIEFING.includes(campo), campo);
  assert.match(BRIEFING, /SINTÉTICO/);
});

test("parseArgs", () => {
  assert.equal(parseArgs([]).outDir, resolve("sample-project-out"));
  const a = parseArgs(["pasta", "--no-project", "--dry-run"]);
  assert.equal(a.outDir, resolve("pasta"));
  assert.ok(a.noProject && a.dryRun);
});

test("resolveCli: env > target > cargo > null; findOnPath usa só o exists injetado", () => {
  const none = () => false;
  assert.equal(resolveCli({ env: {}, exists: none, which: () => null }), null);
  assert.deepEqual(resolveCli({ env: {}, exists: none, which: () => "/usr/bin/cargo" }), [
    "/usr/bin/cargo",
    "run",
    "-q",
    "-p",
    "capia-cli",
    "--",
  ]);
  assert.deepEqual(
    resolveCli({
      env: { CAPIA_CLI_BIN: "/x/capia" },
      exists: (p) => p === "/x/capia",
      which: () => null,
    }),
    ["/x/capia"],
  );
  assert.equal(
    findOnPath("ff", "/a:/b", (p) => p === join("/b", "ff")),
    join("/b", "ff"),
  );
  assert.equal(findOnPath("ff", "", none), null);
});

test("sem ffmpeg no PATH o script PULA com mensagem clara e exit 0 (nada gerado)", () => {
  const r = spawnSync(
    process.execPath,
    [new URL("./make-sample.mjs", import.meta.url).pathname, "--dry-run"],
    {
      encoding: "utf8",
      env: { ...process.env, PATH: "" },
    },
  );
  assert.equal(r.status, 0);
  assert.match(r.stdout, /ffmpeg não encontrado/);
  assert.match(r.stdout, /PULADO/);
});
