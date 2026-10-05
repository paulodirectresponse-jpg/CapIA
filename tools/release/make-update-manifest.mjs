#!/usr/bin/env node
// Gera e ASSINA o manifesto de atualização (Ed25519 sobre o JSON canônico sem `signature`; idêntico ao
// `capia-updater::manifest::signing_payload`, verificado por um teste entre linguagens).
//
//   node tools/release/make-update-manifest.mjs --artifact <instalador> --version 0.7.0 --channel stable
//        --url https://.../CapIA_0.7.0_x64-setup.exe [--notes "texto"] [--min-version 0.6.0] [--rollback]
//        [--out update-manifest.json] [--test-key]
//
// Chave: SOMENTE de segredos de CI — CAPIA_UPDATE_SIGNING_SEED_HEX (64 hex = semente Ed25519) e
// CAPIA_UPDATE_KEY_ID. Nada de chave no repositório nem em log. `--test-key` cria uma chave descartável
// (key_id `test-ephemeral`, par escrito em <out>.test-pubkey.json só com a PÚBLICA) para provar o pipeline.
import {
  createHash,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  sign,
  verify,
} from "node:crypto";
import { readFileSync, statSync, writeFileSync } from "node:fs";
import { basename } from "node:path";
import { pathToFileURL } from "node:url";

const PKCS8_ED25519_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_ED25519_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

/** JSON canônico: chaves ordenadas em todos os níveis, sem espaços. */
export function canonicalJson(v) {
  if (Array.isArray(v)) return `[${v.map(canonicalJson).join(",")}]`;
  if (v !== null && typeof v === "object") {
    return `{${Object.keys(v)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${canonicalJson(v[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(v);
}

export function signingPayload(manifest) {
  const rest = { ...manifest };
  delete rest.signature;
  return Buffer.from(canonicalJson(rest), "utf8");
}

export function privateKeyFromSeedHex(hex) {
  if (!/^[0-9a-f]{64}$/i.test(hex))
    throw new Error("a semente deve ter 64 caracteres hexadecimais");
  return createPrivateKey({
    key: Buffer.concat([PKCS8_ED25519_PREFIX, Buffer.from(hex, "hex")]),
    format: "der",
    type: "pkcs8",
  });
}

export function publicKeyHex(privateKey) {
  const der = createPublicKey(privateKey).export({ format: "der", type: "spki" });
  return Buffer.from(der).subarray(SPKI_ED25519_PREFIX.length).toString("hex");
}

export function publicKeyFromHex(hex) {
  return createPublicKey({
    key: Buffer.concat([SPKI_ED25519_PREFIX, Buffer.from(hex, "hex")]),
    format: "der",
    type: "spki",
  });
}

export function buildManifest({
  artifactPath,
  version,
  channel,
  url,
  notes = "",
  minVersion,
  rollback = false,
}) {
  if (
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(version)
  ) {
    throw new Error(`versão inválida: ${version}`);
  }
  if (!["stable", "beta"].includes(channel)) throw new Error(`canal inválido: ${channel}`);
  if (!url.startsWith("https://")) throw new Error("a URL do artefato deve ser https");
  const bytes = readFileSync(artifactPath);
  const m = {
    schema: 1,
    product: "capia",
    version,
    channel,
    artifact: {
      name: basename(artifactPath),
      url,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      size: statSync(artifactPath).size,
    },
    notes,
    rollback: Boolean(rollback),
  };
  if (minVersion) m.min_version = minVersion;
  return m;
}

export function signManifest(manifest, privateKey, keyId) {
  const sig = sign(null, signingPayload(manifest), privateKey).toString("hex");
  return { ...manifest, signature: { alg: "ed25519", key_id: keyId, value: sig } };
}

export function verifyManifest(manifest, publicKeyHexValue, keyId) {
  const s = manifest.signature;
  if (!s || s.alg !== "ed25519" || s.key_id !== keyId) return false;
  return verify(
    null,
    signingPayload(manifest),
    publicKeyFromHex(publicKeyHexValue),
    Buffer.from(s.value, "hex"),
  );
}

function parseArgs(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith("--")) continue;
    const k = argv[i].slice(2);
    const n = argv[i + 1];
    if (n === undefined || n.startsWith("--")) o[k] = true;
    else {
      o[k] = n;
      i++;
    }
  }
  return o;
}

function main(argv, env) {
  const a = parseArgs(argv);
  for (const r of ["artifact", "version", "channel", "url"]) {
    if (typeof a[r] !== "string") throw new Error(`--${r} é obrigatório`);
  }
  let key;
  let keyId;
  let testPub = null;
  if (a["test-key"]) {
    const { privateKey } = generateKeyPairSync("ed25519");
    key = privateKey;
    keyId = "test-ephemeral";
    testPub = publicKeyHex(privateKey);
  } else if (env.CAPIA_UPDATE_SIGNING_SEED_HEX) {
    key = privateKeyFromSeedHex(env.CAPIA_UPDATE_SIGNING_SEED_HEX);
    keyId = env.CAPIA_UPDATE_KEY_ID;
    if (!keyId) throw new Error("CAPIA_UPDATE_KEY_ID é obrigatório com a chave real");
  } else {
    throw new Error(
      "sem chave de assinatura: defina CAPIA_UPDATE_SIGNING_SEED_HEX/CAPIA_UPDATE_KEY_ID (segredos de CI) ou use --test-key",
    );
  }
  const m = buildManifest({
    artifactPath: a.artifact,
    version: a.version,
    channel: a.channel,
    url: a.url,
    notes: typeof a.notes === "string" ? a.notes : "",
    minVersion: typeof a["min-version"] === "string" ? a["min-version"] : undefined,
    rollback: a.rollback === true,
  });
  const signed = signManifest(m, key, keyId);
  const out = typeof a.out === "string" ? a.out : "update-manifest.json";
  writeFileSync(out, `${JSON.stringify(signed, null, 2)}\n`);
  if (testPub) {
    writeFileSync(
      `${out}.test-pubkey.json`,
      JSON.stringify(
        { key_id: keyId, public_hex: testPub, label: "TEST-ONLY ephemeral key" },
        null,
        2,
      ),
    );
  }
  console.log(`manifesto escrito em ${out} (key_id=${keyId}${testPub ? ", chave de TESTE" : ""})`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main(process.argv.slice(2), process.env);
  } catch (e) {
    console.error(`make-update-manifest: ${e.message}`);
    process.exit(1);
  }
}
