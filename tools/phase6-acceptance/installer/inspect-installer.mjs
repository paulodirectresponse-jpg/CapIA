#!/usr/bin/env node
// Inspeção MÍNIMA e segura de um artefato de instalação: tamanho, SHA-256 (streaming) e assinatura
// de formato (MZ = .exe, D0CF11E0 = .msi). NÃO verifica assinatura digital (isso é Authenticode no
// Windows: `Get-AuthenticodeSignature`/`signtool verify`, registrado na evidência do gate externo) e
// NÃO executa o arquivo.
//   node tools/phase6-acceptance/installer/inspect-installer.mjs --file <instalador> [--expect-sha256 <hex>]
import { createHash } from "node:crypto";
import { createReadStream, statSync } from "node:fs";
import { fileURLToPath } from "node:url";

export async function sha256File(file) {
  const h = createHash("sha256");
  for await (const chunk of createReadStream(file)) h.update(chunk);
  return h.digest("hex");
}

export function detectFormat(head) {
  if (head.length >= 2 && head[0] === 0x4d && head[1] === 0x5a) return "pe_exe";
  if (head.length >= 4 && head.readUInt32BE(0) === 0xd0cf11e0) return "msi_ole";
  return "unknown";
}

export async function inspect(file, expectSha256) {
  const st = statSync(file);
  const fd = createReadStream(file, { start: 0, end: 7 });
  const chunks = [];
  for await (const c of fd) chunks.push(c);
  const sha256 = await sha256File(file);
  const problems = [];
  if (st.size === 0) problems.push("arquivo vazio");
  const format = detectFormat(Buffer.concat(chunks));
  if (format === "unknown") problems.push("formato não reconhecido (esperado .exe PE ou .msi)");
  if (expectSha256 && expectSha256.toLowerCase() !== sha256)
    problems.push("sha256 diferente do esperado");
  return { file, size_bytes: st.size, sha256, format, ok: problems.length === 0, problems };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const arg = (n) => {
    const i = process.argv.indexOf(n);
    return i >= 0 ? process.argv[i + 1] : undefined;
  };
  const file = arg("--file");
  if (!file) {
    console.error("uso: inspect-installer.mjs --file <instalador> [--expect-sha256 <hex>]");
    process.exit(2);
  }
  inspect(file, arg("--expect-sha256")).then((r) => {
    console.log(JSON.stringify(r, null, 2));
    process.exit(r.ok ? 0 : 1);
  });
}
