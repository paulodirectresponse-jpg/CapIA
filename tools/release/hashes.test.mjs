import { strict as assert } from "node:assert";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { formatSums, hashAll, verifySums } from "./hashes.mjs";

test("hashes match node's SHA-256, are sorted, skip the sums file, and verify detects tampering", async () => {
  const d = mkdtempSync(join(tmpdir(), "capia-hash-"));
  try {
    mkdirSync(join(d, "sub"));
    writeFileSync(join(d, "b.exe"), "bbb");
    writeFileSync(join(d, "a.txt"), "aaa");
    writeFileSync(join(d, "sub", "c.bin"), Buffer.from([1, 2, 3]));
    writeFileSync(join(d, "SHA256SUMS.txt"), "ignored");
    const entries = await hashAll([d]);
    assert.deepEqual(
      entries.map((e) => e.name),
      ["a.txt", "b.exe", "sub/c.bin"],
    );
    assert.equal(entries[0].sha256, createHash("sha256").update("aaa").digest("hex"));
    assert.equal(entries[2].size, 3);
    const sums = formatSums(entries);
    assert.deepEqual(await verifySums(sums, d), []);
    writeFileSync(join(d, "b.exe"), "bbX");
    const probs = await verifySums(sums, d);
    assert.equal(probs.length, 1);
    assert.match(probs[0], /b\.exe/);
    rmSync(join(d, "a.txt"));
    assert.ok((await verifySums(sums, d)).some((p) => p.includes("ausente")));
    assert.ok((await verifySums("not a hash line", d))[0].includes("inválida"));
  } finally {
    rmSync(d, { recursive: true });
  }
});
