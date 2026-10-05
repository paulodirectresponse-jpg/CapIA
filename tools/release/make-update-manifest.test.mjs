import { strict as assert } from "node:assert";
import { generateKeyPairSync } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  buildManifest,
  canonicalJson,
  privateKeyFromSeedHex,
  publicKeyHex,
  signManifest,
  verifyManifest,
} from "./make-update-manifest.mjs";

const SCRIPT = fileURLToPath(new URL("./make-update-manifest.mjs", import.meta.url));
const base = (d) => {
  const art = join(d, "CapIA_0.7.0_x64-setup.exe");
  writeFileSync(art, "installer bytes");
  return art;
};

test("canonical JSON matches the documented algorithm (sorted keys, no spaces)", () => {
  assert.equal(
    canonicalJson({ b: 1, a: { z: [{ y: 2, x: 1 }], c: "é\n" } }),
    '{"a":{"c":"é\\n","z":[{"x":1,"y":2}]},"b":1}',
  );
});

test("sign/verify round trip; any tampering breaks it; wrong key id or key fails", () => {
  const d = mkdtempSync(join(tmpdir(), "capia-man-"));
  try {
    const { privateKey } = generateKeyPairSync("ed25519");
    const pub = publicKeyHex(privateKey);
    const m = buildManifest({
      artifactPath: base(d),
      version: "0.7.0",
      channel: "stable",
      url: "https://example.invalid/x.exe",
      notes: "n",
    });
    const s = signManifest(m, privateKey, "k1");
    assert.equal(verifyManifest(s, pub, "k1"), true);
    assert.equal(verifyManifest(s, pub, "k2"), false);
    for (const t of [
      { ...s, version: "0.7.1" },
      { ...s, notes: "x" },
      { ...s, rollback: true },
      { ...s, artifact: { ...s.artifact, sha256: "0".repeat(64) } },
      { ...s, channel: "beta" },
    ]) {
      assert.equal(verifyManifest(t, pub, "k1"), false);
    }
    const other = generateKeyPairSync("ed25519").privateKey;
    assert.equal(verifyManifest(s, publicKeyHex(other), "k1"), false);
    assert.equal(s.artifact.size, "installer bytes".length);
  } finally {
    rmSync(d, { recursive: true });
  }
});

test("validation: bad version, channel and non-https url are refused", () => {
  const d = mkdtempSync(join(tmpdir(), "capia-man-"));
  try {
    const art = base(d);
    const ok = { artifactPath: art, version: "0.7.0", channel: "stable", url: "https://x.invalid/a" };
    assert.throws(() => buildManifest({ ...ok, version: "7" }));
    assert.throws(() => buildManifest({ ...ok, channel: "nightly" }));
    assert.throws(() => buildManifest({ ...ok, url: "http://x.invalid/a" }));
  } finally {
    rmSync(d, { recursive: true });
  }
});

test("seed from env produces a stable public key; bad seed is rejected", () => {
  const seed = "ab".repeat(32);
  const k1 = publicKeyHex(privateKeyFromSeedHex(seed));
  const k2 = publicKeyHex(privateKeyFromSeedHex(seed));
  assert.equal(k1, k2);
  assert.equal(k1.length, 64);
  assert.throws(() => privateKeyFromSeedHex("zz"));
});

test("CLI: refuses to run without a key; --test-key writes only the PUBLIC key and a verifiable manifest", () => {
  const d = mkdtempSync(join(tmpdir(), "capia-man-"));
  try {
    const art = base(d);
    const out = join(d, "m.json");
    const args = [SCRIPT, "--artifact", art, "--version", "0.7.0", "--channel", "stable", "--url", "https://x.invalid/a", "--out", out];
    const env = { PATH: process.env.PATH };
    const none = spawnSync(process.execPath, args, { env, encoding: "utf8" });
    assert.notEqual(none.status, 0);
    assert.match(none.stderr, /sem chave/);
    execFileSync(process.execPath, [...args, "--test-key"], { env });
    const m = JSON.parse(readFileSync(out, "utf8"));
    const pub = JSON.parse(readFileSync(`${out}.test-pubkey.json`, "utf8"));
    assert.equal(m.signature.key_id, "test-ephemeral");
    assert.equal(verifyManifest(m, pub.public_hex, "test-ephemeral"), true);
    assert.ok(!JSON.stringify(pub).includes("seed") && !JSON.stringify(pub).includes("private"));
    // com a chave "real" vinda do ambiente
    const real = spawnSync(process.execPath, args, {
      env: { ...env, CAPIA_UPDATE_SIGNING_SEED_HEX: "cd".repeat(32), CAPIA_UPDATE_KEY_ID: "prod-1" },
      encoding: "utf8",
    });
    assert.equal(real.status, 0, real.stderr);
    assert.ok(!real.stdout.includes("cdcdcd") && !real.stderr.includes("cdcdcd"), "seed must never be logged");
    const m2 = JSON.parse(readFileSync(out, "utf8"));
    assert.equal(
      verifyManifest(m2, publicKeyHex(privateKeyFromSeedHex("cd".repeat(32))), "prod-1"),
      true,
    );
  } finally {
    rmSync(d, { recursive: true });
  }
});
