import { strict as assert } from "node:assert";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { checkVersions, isSemver, workspaceVersion } from "./check-version.mjs";

function repo(v = "1.2.3-rc.1") {
  const root = mkdtempSync(join(tmpdir(), "capia-ver-"));
  const w = (p, s) => {
    mkdirSync(dirname(join(root, p)), { recursive: true });
    writeFileSync(join(root, p), s);
  };
  w(
    "Cargo.toml",
    `[workspace]\nmembers=["crates/a"]\n[workspace.package]\nversion = "${v}"\nedition="2024"\n[workspace.dependencies]\ncapia-a = { path = "crates/a", version = "${v}" }\n`,
  );
  w("crates/a/Cargo.toml", `[package]\nname="capia-a"\nversion.workspace = true\n`);
  w("package.json", JSON.stringify({ name: "x", version: v }));
  w("apps/desktop/package.json", JSON.stringify({ name: "d", version: v }));
  w("apps/desktop/src-tauri/tauri.conf.json", JSON.stringify({ version: v }));
  w("packages/b/fixtures/engine_info.json", JSON.stringify({ version: v }));
  return { root, w };
}

test("semver helper accepts prerelease and rejects junk", () => {
  assert.ok(isSemver("0.6.0-rc.1"));
  assert.ok(isSemver("1.0.0"));
  assert.ok(!isSemver("1.0"));
  assert.ok(!isSemver("v1.0.0"));
  assert.equal(workspaceVersion('[workspace.package]\nversion = "9.9.9"\n'), "9.9.9");
});

test("consistent repo passes", () => {
  const { root } = repo();
  const r = checkVersions(root);
  rmSync(root, { recursive: true });
  assert.deepEqual(r.errors, []);
  assert.equal(r.expected, "1.2.3-rc.1");
  assert.ok(r.checked.length >= 6);
});

for (const [name, path, content] of [
  ["package.json", "apps/desktop/package.json", JSON.stringify({ version: "0.0.0" })],
  [
    "tauri.conf.json",
    "apps/desktop/src-tauri/tauri.conf.json",
    JSON.stringify({ version: "9.0.0" }),
  ],
  ["engine fixture", "packages/b/fixtures/engine_info.json", JSON.stringify({ version: "1.2.3" })],
  ["crate pinned", "crates/a/Cargo.toml", `[package]\nname="capia-a"\nversion = "0.1.0"\n`],
  ["crate without workspace", "crates/a/Cargo.toml", `[package]\nname="capia-a"\n`],
]) {
  test(`divergence in ${name} fails`, () => {
    const { root, w } = repo();
    w(path, content);
    const r = checkVersions(root);
    rmSync(root, { recursive: true });
    assert.ok(r.errors.length >= 1, `esperava erro para ${name}`);
  });
}

test("workspace dependency version drift fails", () => {
  const { root, w } = repo();
  w(
    "Cargo.toml",
    `[workspace.package]\nversion = "1.2.3-rc.1"\n[workspace.dependencies]\ncapia-a = { path = "crates/a", version = "0.0.0" }\n`,
  );
  const r = checkVersions(root);
  rmSync(root, { recursive: true });
  assert.ok(r.errors.some((e) => e.includes("capia-a")));
});
