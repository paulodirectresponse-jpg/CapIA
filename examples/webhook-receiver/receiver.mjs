#!/usr/bin/env node
// EXEMPLO (não é código de produto): receptor local de webhooks do CapIA que VERIFICA a assinatura.
//
//   CAPIA_WEBHOOK_SECRET=<segredo devolvido uma vez pelo webhooks.create> \
//   node examples/webhook-receiver/receiver.mjs [--port 9000]
//
// Contrato (docs/api/webhooks.md):
//   X-CapIA-Signature: v1=<hex HMAC-SHA256(segredo, "<timestamp>.<corpo bruto>")>
//   X-CapIA-Timestamp: Unix em segundos (valores > 1e11 são lidos como milissegundos)
//   X-CapIA-Event-Id : id do evento; o mesmo id reaparece nas tentativas (X-CapIA-Attempt)
// Regras deste receptor: HMAC sobre os BYTES brutos, comparação em tempo constante, janela de
// 5 minutos contra replay, de-duplicação por event id (só DEPOIS de a assinatura ser válida) e corpo
// limitado. Escuta apenas em 127.0.0.1.
import { createHmac, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";

export const TOLERANCE_MS = 5 * 60 * 1000;
export const MAX_BODY_BYTES = 1024 * 1024;

/** Assinatura esperada (hex) para `<timestamp>.<corpo>`; `rawBody` são bytes (Buffer). */
export function computeSignature(secret, timestamp, rawBody) {
  const body = Buffer.isBuffer(rawBody) ? rawBody : Buffer.from(rawBody, "utf8");
  return createHmac("sha256", secret)
    .update(Buffer.from(`${timestamp}.`, "utf8"))
    .update(body)
    .digest("hex");
}

/** Timestamp em ms, ou null se não for um inteiro decimal. */
export function parseTimestampMs(value) {
  if (typeof value !== "string" || !/^\d{1,16}$/.test(value)) return null;
  const n = Number(value);
  return n > 1e11 ? n : n * 1000;
}

/**
 * Verifica um webhook. Devolve `{ ok: true }` ou `{ ok: false, reason }` com reason em:
 * missing_headers | bad_timestamp | stale_timestamp | malformed_signature | bad_signature.
 * O cabeçalho pode trazer várias assinaturas `v1=<hex>` separadas por vírgula (rotação de segredo).
 */
export function verifySignature({
  secret,
  timestamp,
  signature,
  rawBody,
  nowMs = Date.now(),
  toleranceMs = TOLERANCE_MS,
}) {
  if (!secret) throw new Error("secret é obrigatório");
  if (!timestamp || !signature) return { ok: false, reason: "missing_headers" };
  const tsMs = parseTimestampMs(timestamp);
  if (tsMs === null) return { ok: false, reason: "bad_timestamp" };
  if (Math.abs(nowMs - tsMs) > toleranceMs) return { ok: false, reason: "stale_timestamp" };
  const candidates = signature
    .split(",")
    .map((p) => p.trim())
    .filter((p) => p.startsWith("v1="))
    .map((p) => p.slice(3));
  if (candidates.length === 0 || candidates.some((c) => !/^[0-9a-f]{64}$/.test(c)))
    return { ok: false, reason: "malformed_signature" };
  const expected = Buffer.from(computeSignature(secret, timestamp, rawBody), "hex");
  // Sem curto-circuito entre candidatas: o tempo não revela qual (ou se alguma) bateu.
  let match = false;
  for (const c of candidates) {
    if (timingSafeEqual(Buffer.from(c, "hex"), expected)) match = true;
  }
  return match ? { ok: true } : { ok: false, reason: "bad_signature" };
}

/** Conjunto limitado de event ids já processados (os mais antigos saem primeiro). */
export function createDedupe(maxEntries = 10_000) {
  const seen = new Set();
  return {
    /** true se o id já foi visto; senão registra e devolve false. */
    seenBefore(id) {
      if (seen.has(id)) return true;
      seen.add(id);
      if (seen.size > maxEntries) seen.delete(seen.values().next().value);
      return false;
    },
    size: () => seen.size,
  };
}

/** Processa uma entrega: cabeçalhos em minúsculas + corpo bruto → `{ status, body }`. */
export function handleDelivery({ secret, headers, rawBody, dedupe, nowMs, onEvent }) {
  const v = verifySignature({
    secret,
    timestamp: headers["x-capia-timestamp"],
    signature: headers["x-capia-signature"],
    rawBody,
    nowMs,
  });
  if (!v.ok) {
    const status = v.reason === "bad_signature" || v.reason === "stale_timestamp" ? 401 : 400;
    return { status, body: { error: v.reason } };
  }
  let event;
  try {
    event = JSON.parse(Buffer.from(rawBody).toString("utf8"));
  } catch {
    return { status: 400, body: { error: "bad_json" } };
  }
  const eventId = headers["x-capia-event-id"] ?? event.id;
  if (typeof eventId !== "string" || !eventId)
    return { status: 400, body: { error: "no_event_id" } };
  if (dedupe.seenBefore(eventId)) return { status: 200, body: { ok: true, duplicate: true } };
  onEvent?.(event, { attempt: headers["x-capia-attempt"], delivery: headers["x-capia-delivery"] });
  return { status: 200, body: { ok: true, duplicate: false } };
}

export function createReceiver({ secret, onEvent, nowFn = Date.now }) {
  const dedupe = createDedupe();
  return createServer((req, res) => {
    const chunks = [];
    let size = 0;
    let aborted = false;
    req.on("data", (c) => {
      size += c.length;
      if (size > MAX_BODY_BYTES) {
        aborted = true;
        res.writeHead(413).end();
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => {
      if (aborted) return;
      if (req.method !== "POST") {
        res.writeHead(405).end();
        return;
      }
      const out = handleDelivery({
        secret,
        headers: req.headers,
        rawBody: Buffer.concat(chunks),
        dedupe,
        nowMs: nowFn(),
        onEvent,
      });
      res.writeHead(out.status, { "content-type": "application/json" });
      res.end(JSON.stringify(out.body));
    });
  });
}

function main() {
  const secret = process.env.CAPIA_WEBHOOK_SECRET;
  if (!secret) {
    console.error("defina CAPIA_WEBHOOK_SECRET (o segredo é mostrado uma única vez na criação)");
    process.exit(2);
  }
  const i = process.argv.indexOf("--port");
  const port = i >= 0 ? Number(process.argv[i + 1]) : 9000;
  const server = createReceiver({
    secret,
    onEvent: (e, meta) => {
      console.log(
        `evento ${e.type} id=${e.id} projeto=${e.project_id ?? "-"} run=${e.run_id ?? "-"} tentativa=${meta.attempt ?? "?"}`,
      );
    },
  });
  server.listen(port, "127.0.0.1", () => {
    console.log(`receptor em http://127.0.0.1:${port}/ (assinatura verificada, janela de 5 min)`);
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
