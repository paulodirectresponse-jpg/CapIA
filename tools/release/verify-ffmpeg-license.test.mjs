import { strict as assert } from "node:assert";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { configurationFlags, evaluateFfmpegText, inspectDir } from "./verify-ffmpeg-license.mjs";

const LGPL_CONF =
  "configuration: --disable-autodetect --disable-network --disable-programs --enable-shared --disable-static --disable-gpl";
const LGPL_LIC =
  "ffmpeg is free software; under the terms of the GNU Lesser General Public License as published";
const GPL_CONF =
  "configuration: --enable-gpl --enable-version3 --enable-static --enable-libx264 --enable-libx265 --disable-w32threads";
const GPL_LIC =
  "ffmpeg is free software; under the terms of the GNU General Public License as published";

test("LGPL build without forbidden flags is approved", () => {
  const r = evaluateFfmpegText({ version: LGPL_CONF, license: LGPL_LIC });
  assert.deepEqual(r.reasons, []);
  assert.equal(r.approved, true);
  assert.equal(r.license, "LGPL");
});

test("every forbidden flag is rejected", () => {
  for (const flag of [
    "--enable-gpl",
    "--enable-nonfree",
    "--enable-libx264",
    "--enable-libx265",
    "--enable-libfdk-aac",
  ]) {
    const r = evaluateFfmpegText({
      version: `configuration: --disable-autodetect ${flag}`,
      license: LGPL_LIC,
    });
    assert.equal(r.approved, false, flag);
    assert.ok(
      r.reasons.some((x) => x.includes(flag)),
      flag,
    );
  }
});

test("a typical GPL essentials build is rejected on flags and on license text", () => {
  const r = evaluateFfmpegText({ version: GPL_CONF, license: GPL_LIC });
  assert.equal(r.approved, false);
  assert.ok(r.reasons.length >= 3);
  assert.equal(r.license, "GPL");
  // só o texto de licença GPL, sem flags, também reprova
  const r2 = evaluateFfmpegText({ version: LGPL_CONF, license: GPL_LIC });
  assert.equal(r2.approved, false);
});

test("missing configuration is rejected (never approves what it cannot read)", () => {
  const r = evaluateFfmpegText({ version: "ffmpeg version 8", license: LGPL_LIC });
  assert.equal(r.approved, false);
});

test("version3 is a warning, not an approval blocker", () => {
  const r = evaluateFfmpegText({
    version: `${LGPL_CONF} --enable-version3`,
    license: LGPL_LIC,
  });
  assert.equal(r.approved, true);
  assert.ok(r.warnings.some((w) => w.includes("version3")));
});

test("configurationFlags reads -buildconf style output", () => {
  const f = configurationFlags("  --enable-shared\n  --disable-gpl\nnoise");
  assert.ok(f.has("--enable-shared") && f.has("--disable-gpl"));
});

test(
  "inspectDir with a fake GPL ffmpeg is not approved; with a fake LGPL it is (needs license file)",
  {
    skip: process.platform === "win32",
  },
  () => {
    const mk = (conf, lic, withLicenseFile) => {
      const d = mkdtempSync(join(tmpdir(), "capia-ff-"));
      const script = `#!/bin/sh
case "$*" in
  *-buildconf*) echo "  ${conf.replace("configuration: ", "")}";;
  *-L*) echo "${lic}";;
  *) echo "ffmpeg version test"; echo "${conf}";;
esac
`;
      for (const n of ["ffmpeg", "ffprobe"]) {
        writeFileSync(join(d, n), script);
        chmodSync(join(d, n), 0o755);
      }
      if (withLicenseFile) writeFileSync(join(d, "LICENSE.md"), "x");
      return d;
    };
    const gpl = mk(GPL_CONF, GPL_LIC, true);
    const lgpl = mk(LGPL_CONF, LGPL_LIC, true);
    const noLic = mk(LGPL_CONF, LGPL_LIC, false);
    try {
      assert.equal(inspectDir(gpl).approved, false);
      assert.equal(inspectDir(lgpl).approved, true);
      assert.equal(inspectDir(noLic).approved, false);
      assert.equal(inspectDir(lgpl, { requireMetadata: true }).approved, false);
      mkdirSync(join(tmpdir(), "capia-ff-missing"), { recursive: true });
      assert.equal(inspectDir(join(tmpdir(), "capia-ff-missing")).approved, false);
    } finally {
      for (const d of [gpl, lgpl, noLic]) rmSync(d, { recursive: true });
    }
  },
);
