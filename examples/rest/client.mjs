#!/usr/bin/env node
// EXEMPLO (não é código de produto): fluxo canônico externo do CapIA por REST, em Node ≥ 22 (sem
// dependências). Ver docs/api/canonical-flow.md.
//
//   CAPIA_PORT=8731 CAPIA_TOKEN=capia_REDACTED \
//   node examples/rest/client.mjs --raw raw.mp4 --reference ref.mp4 \
//        --brief "Produto: ... Público: ... Oferta: ... CTA: ..." [--approve plan_approval] [--variants 3]
//
// Scopes do token: project:read project:write media:read media:write run:read run:start run:approve
// export:read export:start (+ webhook:manage com --webhook-url). Só fala com 127.0.0.1.
//
// Forma das respostas: o servidor repassa o resultado dos serviços da Engine API/IA (ex.: Run =
// `{ run: { id, status, stage, pending, sequences } }`). `extractId` aceita `{ <tipo>: { id } }` ou
// `{ id }` para não depender do envelope exato. Ajuste se a sua versão do servidor diferir.
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { basename } from "node:path";
import { fileURLToPath } from "node:url";

export class ApiError extends Error {
  constructor(status, body) {
    super(`HTTP ${status} ${body?.code ?? ""}: ${body?.message ?? ""}`);
    this.status = status;
    this.code = body?.code;
    this.body = body;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export class CapiaClient {
  constructor({ port, token, fetchImpl = fetch }) {
    if (!port || !token) throw new Error("defina CAPIA_PORT e CAPIA_TOKEN");
    this.base = `http://127.0.0.1:${port}`;
    this.token = token;
    this.fetch = fetchImpl;
  }

  /** `idempotencyKey`: use a MESMA chave ao repetir um POST após timeout (evita efeito duplicado). */
  async request(method, path, { body, query, idempotencyKey } = {}) {
    const qs = query
      ? `?${new URLSearchParams(Object.entries(query).map(([k, v]) => [k, String(v)]))}`
      : "";
    const headers = { Authorization: `Bearer ${this.token}`, Accept: "application/json" };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (method !== "GET" && method !== "DELETE")
      headers["Idempotency-Key"] = idempotencyKey ?? randomUUID();
    const res = await this.fetch(`${this.base}${path}${qs}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    const json = text ? JSON.parse(text) : null;
    if (!res.ok) throw new ApiError(res.status, json);
    return json;
  }

  get = (p, query) => this.request("GET", p, { query });
  post = (p, body, idempotencyKey) => this.request("POST", p, { body, idempotencyKey });
}

export function extractId(res, kind) {
  const id = res?.[kind]?.id ?? res?.id ?? res?.[`${kind}_id`];
  if (!id) throw new Error(`resposta sem id de ${kind}: ${JSON.stringify(res)}`);
  return id;
}

export async function poll(
  fn,
  done,
  { intervalMs = 1000, timeoutMs = 600_000, sleepFn = sleep } = {},
) {
  const t0 = Date.now();
  for (;;) {
    const v = await fn();
    if (done(v)) return v;
    if (Date.now() - t0 > timeoutMs) throw new Error("tempo esgotado esperando a operação");
    await sleepFn(intervalMs);
  }
}

const TERMINAL_RUN = new Set(["completed", "failed", "cancelled"]);

/** Faz upload (base64 inline, para arquivos pequenos) e importa como asset. Devolve o asset_id. */
export async function importFile(c, projectId, file, log = () => {}, sleepFn = sleep) {
  const bytes = readFileSync(file);
  const up = await c.post("/v1/uploads/inline", {
    filename: basename(file),
    content_base64: bytes.toString("base64"),
  });
  const uploadId = extractId(up, "upload");
  const started = await c.post(`/v1/projects/${projectId}/assets`, { upload_id: uploadId });
  const ticketId = extractId(started, "ticket");
  log(`importando ${basename(file)} (ticket ${ticketId})`);
  const ticket = await poll(
    () => c.get(`/v1/projects/${projectId}/imports/${ticketId}`),
    (t) =>
      ["completed", "failed", "cancelled"].includes(
        (t.ticket ?? t).state ?? (t.ticket ?? t).status,
      ),
    { intervalMs: 500, sleepFn },
  );
  const tk = ticket.ticket ?? ticket;
  if ((tk.state ?? tk.status) !== "completed")
    throw new Error(`importação falhou: ${JSON.stringify(tk)}`);
  return tk.asset_id ?? tk.asset?.id ?? extractId(tk, "asset");
}

/**
 * Passos 1–11 do fluxo canônico. `autoApprove`: tipos de decisão que o script aprova sozinho
 * (padrão: nenhum — gasto, geração e licença nunca são aprovados sem um humano pedir).
 */
export async function runCanonicalFlow(c, opts, log = console.log) {
  const { raw, reference, brief, autoApprove = [], variants = 0, exportPreset = "h264-mp4" } = opts;
  const sleepFn = opts.sleepFn ?? sleep;
  if (opts.webhookUrl) {
    const wh = await c.post("/v1/webhooks", {
      url: opts.webhookUrl,
      events: [
        "run.completed",
        "run.failed",
        "run.waiting_user",
        "export.completed",
        "export.failed",
      ],
      description: "exemplo canônico",
    });
    log(`webhook ${extractId(wh, "webhook")} criado (o segredo foi mostrado só agora; guarde-o)`);
  }
  // 1. projeto
  const project = await c.post("/v1/projects", { name: opts.projectName ?? "Fluxo canônico" });
  const pid = extractId(project, "project");
  log(`1. projeto ${pid}`);
  // 2–3. bruto e referência
  const rawAsset = await importFile(c, pid, raw, log, sleepFn);
  const refAsset = await importFile(c, pid, reference, log, sleepFn);
  log(`2-3. assets ${rawAsset} (bruto) e ${refAsset} (referência)`);
  // 4–5. briefing + Run (uma chave de idempotência estável por intenção)
  const created = await c.post(
    `/v1/projects/${pid}/runs`,
    { brief_text: brief, assets: [rawAsset], references: [refAsset], start: true },
    opts.runIdempotencyKey ?? randomUUID(),
  );
  const runId = extractId(created, "run");
  log(`4-5. Run ${runId} iniciada`);
  // 6–8. acompanhar, aprovar, esperar
  let run;
  for (;;) {
    run = await poll(
      async () => (await c.get(`/v1/projects/${pid}/runs/${runId}`)).run,
      (r) => TERMINAL_RUN.has(r.status) || r.status === "waiting_user",
      { sleepFn },
    );
    if (run.status !== "waiting_user") break;
    const d = run.pending;
    log(`7. decisão ${d.id} (${d.kind}): ${d.question}`);
    if (!autoApprove.includes(d.kind) || !d.options.some((o) => o.id === "approve")) {
      log(`   exige um humano (use --approve ${d.kind} só se for seguro). Parando aqui.`);
      return { projectId: pid, runId, status: "waiting_user", decision: d };
    }
    await c.post(`/v1/projects/${pid}/runs/${runId}/approvals`, {
      decision_id: d.id,
      option: "approve",
    });
  }
  log(`8. Run ${run.status}`);
  if (run.status !== "completed") return { projectId: pid, runId, status: run.status, run };
  // 9. variantes
  if (variants > 0) {
    await c.post(`/v1/projects/${pid}/runs/${runId}/variants`, { count: variants });
    log(`9. ${variants} variantes pedidas (Runs filhas; acompanhe por GET /runs)`);
  }
  // 10. export
  const sequences = run.sequences?.length
    ? run.sequences
    : ((await c.get(`/v1/projects/${pid}/sequences`)).sequences ?? []).map((s) => s.id);
  const first = sequences[0]?.id ?? sequences[0];
  if (!first) throw new Error("a Run não produziu nenhuma sequence para exportar");
  const exp = await c.post(`/v1/projects/${pid}/exports`, {
    items: [{ sequence: first, preset: exportPreset }],
  });
  const expId = extractId(exp, "export");
  const done = await poll(
    () => c.get(`/v1/projects/${pid}/exports/${expId}`),
    (e) => ["completed", "failed", "cancelled"].includes((e.export ?? e).state),
    { sleepFn },
  );
  log(`10. export ${expId}: ${(done.export ?? done).state}`);
  // 11. o webhook chega ao receptor local; 12. confira no app/summary
  const summary = await c.get(`/v1/projects/${pid}/summary`);
  log("12. confira o resultado no CapIA (abra o projeto) ou em /summary");
  return { projectId: pid, runId, status: "completed", exportId: expId, summary };
}

function arg(name, def) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : def;
}

async function main() {
  const c = new CapiaClient({ port: process.env.CAPIA_PORT, token: process.env.CAPIA_TOKEN });
  const brief =
    arg("--brief") ?? (arg("--brief-file") ? readFileSync(arg("--brief-file"), "utf8") : null);
  if (!arg("--raw") || !arg("--reference") || !brief) {
    console.error(
      "uso: client.mjs --raw <video> --reference <video> --brief <texto>|--brief-file <arq>",
    );
    process.exit(2);
  }
  const result = await runCanonicalFlow(c, {
    raw: arg("--raw"),
    reference: arg("--reference"),
    brief,
    autoApprove: (arg("--approve") ?? "").split(",").filter(Boolean),
    variants: Number(arg("--variants", "0")),
    webhookUrl: arg("--webhook-url"),
  });
  console.log(JSON.stringify({ ...result, summary: undefined }, null, 2));
  process.exit(result.status === "completed" ? 0 : 3);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((e) => {
    console.error(e instanceof ApiError ? `${e.message} (request_id=${e.body?.request_id})` : e);
    process.exit(1);
  });
}
