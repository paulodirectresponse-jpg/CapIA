#!/usr/bin/env node
// DemandSpec em ≥ 10 briefings.
//   (padrão)  modo Replay: roda a avaliação de integridade do pipeline (sem rede, sem chave).
//   --live    provider REAL: --provider <openai|anthropic|google|openrouter|…> --model <id>
//             --briefs <pasta com .docx/.pdf/.txt/.md> --expected <expected.json>
//             (CAPIA_ACCEPT_API_KEY no ambiente; devserver em --server, padrão :5199)
//   expected.json: { "<arquivo>": { "product": "…", "audience": "…", … } }  (campos que o humano espera)
// Sem execução real os números ficam `null` — nunca inventados.
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const out = join(root, "target/phase4-acceptance");
mkdirSync(out, { recursive: true });
const arg = (k) => {
  const i = process.argv.indexOf(k);
  return i >= 0 ? process.argv[i + 1] : undefined;
};

if (!process.argv.includes("--live")) {
  const r = spawnSync(
    "cargo",
    ["test", "-q", "-p", "capia-intelligence", "--test", "demand_eval"],
    { cwd: root, stdio: "inherit" },
  );
  const f = join(out, "demand-eval-replay.json");
  console.log(existsSync(f) ? readFileSync(f, "utf8") : "sem relatório");
  console.log(
    "\nLEMBRETE: modo Replay prova a integridade do pipeline, não a qualidade de um LLM real. Use --live.",
  );
  process.exit(r.status ?? 1);
}

const key = process.env.CAPIA_ACCEPT_API_KEY;
const provider = arg("--provider"),
  model = arg("--model"),
  briefs = arg("--briefs"),
  expectedPath = arg("--expected");
if (!key || !provider || !model || !briefs || !expectedPath) {
  console.error("--live exige CAPIA_ACCEPT_API_KEY, --provider, --model, --briefs e --expected");
  process.exit(2);
}
const base = arg("--server") ?? "http://127.0.0.1:5199";
const call = async (m, p = {}) => {
  const r = await fetch(`${base}/api/${m}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(p),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(`${m}: ${j.code}: ${j.message}`);
  return j;
};
const KIND = {
  openai: "open_ai_compatible",
  openrouter: "open_ai_compatible",
  groq: "open_ai_compatible",
  anthropic: "anthropic",
  google: "google",
};
const st = await call("ai.status");
const preset = st.presets.find((p) => p.key === provider);
await call("ai.provider.save", {
  provider: {
    id: provider,
    kind: KIND[provider] ?? preset?.kind,
    display_name: provider,
    base_url: preset?.base_url ?? null,
    enabled: true,
  },
  api_key: key,
});
await call("ai.model.save", {
  endpoint: {
    id: `${provider}:${model}`,
    provider_id: provider,
    model_id: model,
    display_name: model,
    enabled: true,
    context_window: 128000,
    max_output_tokens: 0,
    capabilities: {
      entries: {
        text_generation: { supported: true, origin: "declared" },
        structured_output: { supported: true, origin: "declared" },
        streaming: { supported: true, origin: "declared" },
      },
    },
  },
});
await call("ai.brain.set", {
  profile: { id: "accept", name: "accept", brain: `${provider}:${model}` },
});
const tmp = mkdtempSync(join(tmpdir(), "capia-demand-"));
await call("project.create", { path: join(tmp, "p.capia") });
const expected = JSON.parse(readFileSync(resolve(expectedPath), "utf8"));
const rows = [];
let hit = 0,
  total = 0,
  unverified = 0,
  dropped = 0;
for (const [file, exp] of Object.entries(expected)) {
  const { task_id } = await call("ai.demand.interpret", {
    documents: [join(resolve(briefs), file)],
    assets: [],
  });
  let spec;
  for (let i = 0; i < 1800 && !spec; i++) {
    const t = await call("ai.task.get", { task_id });
    if (t.state === "done") spec = t.result.spec;
    else if (t.state !== "running") {
      rows.push({ file, error: t.error });
      break;
    } else await new Promise((r) => setTimeout(r, 200));
  }
  if (!spec) continue;
  const norm = (s) =>
    (s ?? "")
      .toLowerCase()
      .replace(/[^\p{L}\p{N}]+/gu, " ")
      .trim();
  let h = 0;
  for (const [field, want] of Object.entries(exp)) {
    total++;
    const got = spec[field]?.value;
    if (got && norm(got).includes(norm(want))) {
      h++;
      hit++;
    }
  }
  unverified += spec.verification.fields_unverified;
  dropped += spec.verification.sources_dropped;
  rows.push({
    file,
    expected_fields: Object.keys(exp).length,
    matched: h,
    sources_dropped: spec.verification.sources_dropped,
    fields_unverified: spec.verification.fields_unverified,
  });
}
const res = {
  kind: "live",
  executed_with: { provider, model, date: new Date().toISOString() },
  briefs: rows.length,
  field_recall: total ? hit / total : null,
  fields_unverified: unverified,
  sources_dropped: dropped,
  rows,
  note: "recall por casamento textual normalizado; a avaliação de mérito do conteúdo é humana",
};
writeFileSync(join(out, "demand-eval-live.json"), JSON.stringify(res, null, 2));
console.log(res);
