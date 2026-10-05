import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { RELEASE_DOCS, checkDocs, checkVersions, readVersions } from "./check-release.mjs";
import { detectFormat, inspect, sha256File } from "./inspect-installer.mjs";

function repo(files) {
  const root = mkdtempSync(join(tmpdir(), "rel-"));
  for (const [rel, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), content);
  }
  return root;
}
const versions = (c, p, t) => ({
  "Cargo.toml": `[workspace]\nmembers=[]\n[workspace.package]\nedition="2024"\nversion = "${c}"\n`,
  "package.json": JSON.stringify({ name: "x", version: p }),
  "apps/desktop/src-tauri/tauri.conf.json": JSON.stringify({ version: t }),
});

test("versão única nas três fontes", () => {
  const ok = repo(versions("0.6.0-rc.1", "0.6.0-rc.1", "0.6.0-rc.1"));
  assert.equal(checkVersions(ok).ok, true);
  assert.equal(checkVersions(ok, "0.6.0-rc.1").ok, true);
  assert.equal(checkVersions(ok, "0.6.0-rc.2").ok, false);
});

test("versões divergentes ou ausentes são problema", () => {
  const diff = repo(versions("0.6.0-rc.1", "0.0.0", "0.6.0-rc.1"));
  const r = checkVersions(diff);
  assert.equal(r.ok, false);
  assert.match(r.problems[0], /divergentes/);
  const missing = repo({ "package.json": JSON.stringify({ version: "1.0.0" }) });
  assert.equal(checkVersions(missing).ok, false);
  assert.equal(readVersions(missing)["Cargo.toml (workspace.package)"], null);
});

const longText = (extra = "") => `${"conteúdo ".repeat(40)}${extra}`;
const releaseMd = `${Array.from({ length: 12 }, (_, i) => `- [ ] item ${i}`).join("\n")}\nassinatura SBOM rollback migração\n${longText()}`;
const docs = (over = {}) => ({
  "CHANGELOG.md": `## [Unreleased]\n\n## [0.6.0-rc.1] - 2026-10-05\n${longText()}`,
  "docs/RELEASE.md": releaseMd,
  "docs/KNOWN_ISSUES.md": `pendente externo ${longText()}`,
  "docs/MIGRATION_COMPAT.md": longText(),
  "docs/phase6/IMPL_DOCS_ACCEPTANCE.md": longText(),
  ...over,
});

test("documentos de release completos e coerentes", () => {
  const root = repo(docs());
  assert.deepEqual(checkDocs(root, "0.6.0-rc.1"), { ok: true, problems: [] });
});

test("documento ausente, curto, sem seção da versão ou sem checklist falha", () => {
  const miss = docs();
  delete miss["docs/KNOWN_ISSUES.md"];
  assert.ok(checkDocs(repo(miss)).problems.some((p) => p.includes("KNOWN_ISSUES")));
  assert.ok(
    checkDocs(repo(docs({ "docs/MIGRATION_COMPAT.md": "curto" }))).problems.some((p) =>
      p.includes("curto"),
    ),
  );
  assert.ok(checkDocs(repo(docs()), "0.7.0").problems.some((p) => p.includes('"## [0.7.0]"')));
  assert.ok(
    checkDocs(
      repo(docs({ "docs/RELEASE.md": longText("assinatura SBOM rollback migra") })),
    ).problems.some((p) => p.includes("checklist")),
  );
  assert.ok(
    checkDocs(repo(docs({ "CHANGELOG.md": `## [0.6.0-rc.1]\n${longText()}` }))).problems.some((p) =>
      p.includes("Unreleased"),
    ),
  );
  assert.equal(RELEASE_DOCS.length, 5);
});

test("CLI: sai 1 quando algo falha e 2 sem argumentos", () => {
  const script = new URL("./check-release.mjs", import.meta.url).pathname;
  assert.equal(spawnSync(process.execPath, [script], { encoding: "utf8" }).status, 2);
  const r = spawnSync(process.execPath, [script, "--versions", "--expect", "9.9.9"], {
    encoding: "utf8",
  });
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).versions.ok, false);
});

test("inspect-installer: hash em streaming, formato e comparação", async () => {
  const dir = mkdtempSync(join(tmpdir(), "inst-"));
  const exe = join(dir, "setup.exe");
  const body = Buffer.concat([Buffer.from("MZ"), Buffer.alloc(1000, 7)]);
  writeFileSync(exe, body);
  const expected = createHash("sha256").update(body).digest("hex");
  assert.equal(await sha256File(exe), expected);
  const r = await inspect(exe, expected);
  assert.equal(r.ok, true);
  assert.equal(r.format, "pe_exe");
  assert.equal(r.size_bytes, 1002);
  assert.equal((await inspect(exe, "0".repeat(64))).ok, false);
  const msi = join(dir, "a.msi");
  writeFileSync(msi, Buffer.from([0xd0, 0xcf, 0x11, 0xe0, 1, 2, 3, 4]));
  assert.equal((await inspect(msi)).format, "msi_ole");
  const bad = join(dir, "x.bin");
  writeFileSync(bad, "texto");
  const b = await inspect(bad);
  assert.equal(b.ok, false);
  assert.equal(b.format, "unknown");
  const empty = join(dir, "e.exe");
  writeFileSync(empty, "");
  assert.equal((await inspect(empty)).ok, false);
  assert.equal(detectFormat(Buffer.alloc(0)), "unknown");
});
