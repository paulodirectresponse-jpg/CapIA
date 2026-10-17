// Utilitários sem dependências: escrita atômica, hash, redação de segredos, sleep.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
export const sha256 = (text) => createHash("sha256").update(text).digest("hex").slice(0, 16);
export const nowIso = () => new Date().toISOString();

export function atomicWriteJson(file, value) {
  mkdirSync(dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(tmp, file); // substitui de forma atômica (inclusive no Windows)
}

export function readJson(file, fallback = null) {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return fallback;
  }
}

const SECRET_PATTERNS = [
  /sk-[A-Za-z0-9_-]{16,}/g,
  /gh[pousr]_[A-Za-z0-9]{20,}/g,
  /(Bearer\s+)[A-Za-z0-9._~+/-]{16,}/gi,
  /((?:API_?KEY|TOKEN|SECRET|PASSWORD)[A-Z_]*\s*[=:]\s*)\S+/gi,
];

/** Segredos nunca vão para log/artefato (princípio 6 do CapIA). */
export function redact(text) {
  let out = String(text);
  for (const pattern of SECRET_PATTERNS)
    out = out.replace(pattern, (m, g1) => `${g1 ?? ""}[REDACTED]`);
  return out;
}
