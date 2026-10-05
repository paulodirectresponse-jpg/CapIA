import { strict as assert } from "node:assert";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { buildInstallerConfig, stage } from "./stage-bundle.mjs";

test("installer config is per-user NSIS, no server unless present, and keeps ffmpeg as resource", () => {
  const c = buildInstallerConfig({ hasServer: false });
  assert.equal(c.bundle.windows.nsis.installMode, "currentUser");
  assert.deepEqual(c.bundle.targets, ["nsis"]);
  assert.equal(c.bundle.externalBin, undefined);
  assert.equal(c.bundle.resources["bundle-staging/ffmpeg/"], "ffmpeg/");
  assert.equal(c.bundle.windows.webviewInstallMode.type, "embedBootstrapper");
  const s = buildInstallerConfig({ hasServer: true, webview: "offline", hasNotices: true });
  assert.deepEqual(s.bundle.externalBin, ["binaries/capia-server"]);
  assert.equal(s.bundle.windows.webviewInstallMode.type, "offlineInstaller");
  assert.ok(s.bundle.resources["bundle-staging/THIRD_PARTY_NOTICES.txt"]);
  assert.throws(() => buildInstallerConfig({ hasServer: false, webview: "nope" }));
});

function fakeFfmpegDir({ gpl }) {
  const d = mkdtempSync(join(tmpdir(), "capia-stage-ff-"));
  const conf = gpl ? "--enable-gpl --enable-libx264" : "--disable-autodetect --disable-gpl";
  const lic = gpl ? "GNU General Public License" : "GNU Lesser General Public License";
  const sh = `#!/bin/sh
case "$*" in *-buildconf*) echo "  ${conf}";; *-L*) echo "${lic}";; *) echo "configuration: ${conf}";; esac
`;
  for (const n of ["ffmpeg", "ffprobe"]) {
    writeFileSync(join(d, n), sh);
    chmodSync(join(d, n), 0o755);
  }
  writeFileSync(join(d, "LICENSE.txt"), "lic");
  writeFileSync(join(d, "ffmpeg-build.json"), JSON.stringify({ source_tag: "n8.1.3" }));
  return d;
}

const opts = (extra) => ({ flags: new Set(extra.flags ?? []), ...extra.kv });

test("stage refuses an unapproved ffmpeg without the dev flag and labels it with the flag", {
  skip: process.platform === "win32",
}, () => {
  const st = mkdtempSync(join(tmpdir(), "capia-stage-tauri-"));
  const gpl = fakeFfmpegDir({ gpl: true });
  try {
    assert.throws(
      () => stage(opts({ kv: { "ffmpeg-dir": gpl, "src-tauri": st }, flags: ["allow-missing-server"] })),
      /FFmpeg não aprovado/,
    );
    const r = stage(
      opts({ kv: { "ffmpeg-dir": gpl, "src-tauri": st }, flags: ["allow-missing-server", "allow-dev-ffmpeg"] }),
    );
    assert.equal(r.ffmpegApproved, false);
    assert.ok(existsSync(join(st, "bundle-staging/ffmpeg/DEV-TEST-ONLY-UNAPPROVED.txt")));
    assert.ok(existsSync(join(st, "bundle-staging/SERVER-PLACEHOLDER.txt")));
    const cfg = JSON.parse(readFileSync(r.configPath, "utf8"));
    assert.equal(cfg.bundle.externalBin, undefined);
  } finally {
    rmSync(st, { recursive: true });
    rmSync(gpl, { recursive: true });
  }
});

test("stage with an approved ffmpeg and a server binary wires the sidecar; missing server is an error without the switch", {
  skip: process.platform === "win32",
}, () => {
  const st = mkdtempSync(join(tmpdir(), "capia-stage-tauri-"));
  const ok = fakeFfmpegDir({ gpl: false });
  const srv = join(ok, "server.exe");
  writeFileSync(srv, "bin");
  try {
    assert.throws(() => stage(opts({ kv: { "ffmpeg-dir": ok, "src-tauri": st } })), /capia-server/);
    mkdirSync(st, { recursive: true });
    const r = stage(opts({ kv: { "ffmpeg-dir": ok, "src-tauri": st, "server-bin": srv } }));
    assert.equal(r.ffmpegApproved, true);
    assert.equal(r.hasServer, true);
    assert.ok(existsSync(join(st, "binaries/capia-server-x86_64-pc-windows-msvc.exe")));
    assert.ok(existsSync(join(st, "bundle-staging/ffmpeg/ffprobe")));
    const cfg = JSON.parse(readFileSync(r.configPath, "utf8"));
    assert.deepEqual(cfg.bundle.externalBin, ["binaries/capia-server"]);
  } finally {
    rmSync(st, { recursive: true });
    rmSync(ok, { recursive: true });
  }
});
