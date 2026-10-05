// Revisão estática dos scripts/workflows que só rodam no Windows (não podemos executá-los em Linux).
// Garante as propriedades de segurança por inspeção do texto: certificado só de segredos, nada de segredo
// em log, rótulos dev/test, instalador sem e2e-testkit e desinstalação que não apaga projetos.
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const read = (rel) => readFileSync(fileURLToPath(new URL(`../../${rel}`, import.meta.url)), "utf8");
const sign = read("tools/release/sign.ps1");
const verify = read("tools/release/verify-signature.ps1");
const smoke = read("tools/release/installer-smoke.ps1");
const installerYml = read(".github/workflows/installer.yml");
const releaseYml = read(".github/workflows/release.yml");
const hooks = read("apps/desktop/src-tauri/installer-hooks.nsh");

test("sign.ps1 takes the certificate only from env vars and never prints secrets", () => {
  assert.match(sign, /CAPIA_SIGN_PFX_BASE64/);
  assert.match(sign, /CAPIA_SIGN_PFX_PASSWORD/);
  for (const line of sign.split("\n")) {
    if (/Write-Host|Write-Output|echo/i.test(line)) {
      assert.doesNotMatch(line, /PFX_PASSWORD|PFX_BASE64|\$tmpPfx/, `secret in log line: ${line}`);
    }
  }
  assert.match(sign, /UNSIGNED-DEV-TEST-ONLY/);
  assert.match(sign, /external_pending/);
  assert.match(sign, /Remove-Item \$tmpPfx/, "temp PFX must be deleted");
  assert.match(sign, /New-SelfSignedCertificate/, "test mode uses a throw-away certificate");
  assert.doesNotMatch(sign, /\.pfx['"]\s*$/m, "no PFX path in repo");
});

test("verify-signature.ps1 never reports external_pending as valid and can require real signatures", () => {
  assert.match(verify, /NotSigned' \{ \$state = 'external_pending'/);
  assert.match(verify, /\$Require -eq 'valid' -and \$state -ne 'valid'/);
});

test("installer workflow builds without e2e-testkit, runs the smoke, and does not touch ci.yml", () => {
  assert.doesNotMatch(installerYml, /--features/);
  assert.match(installerYml, /installer-smoke\.ps1/);
  assert.match(installerYml, /pnpm check:version/);
  assert.match(installerYml, /cargo test -p capia-updater -p capia-support/);
  assert.match(installerYml, /--bundles nsis/);
  assert.match(installerYml, /dev-test/);
  assert.doesNotMatch(
    installerYml,
    /CAPIA_UPDATE_SIGNING_SEED_HEX/,
    "no update key in the CI workflow",
  );
});

test("secrets appear only in env mappings, never in run lines", () => {
  for (const [name, text] of [
    ["installer", installerYml],
    ["release", releaseYml],
  ]) {
    for (const line of text.split("\n")) {
      if (line.includes("secrets.")) {
        assert.match(line, /^\s+[A-Z0-9_]+: \${{ secrets\.[A-Z0-9_]+ }}\s*$/, `${name}: ${line}`);
      }
    }
  }
});

test("release workflow requires the approved FFmpeg (no dev override) and signs only with secrets", () => {
  assert.doesNotMatch(releaseYml, /allow-dev/);
  assert.match(releaseYml, /--require-metadata/);
  assert.match(releaseYml, /Require valid/);
  assert.match(releaseYml, /external_pending/);
  assert.match(releaseYml, /HAS_UPDATE_KEY == 'True' && env\.HAS_CERT == 'True'/);
});

test("smoke proves user projects survive uninstall and runs the self-test from the installed location", () => {
  assert.match(smoke, /--self-test/);
  assert.match(smoke, /--require-media/);
  assert.match(smoke, /PROJETO DO USUÁRIO FOI APAGADO/);
  assert.match(smoke, /uninstall\*\.exe/);
  assert.match(smoke, /Step 'reinstall'/);
});

test("NSIS hooks never delete .capia files", () => {
  assert.doesNotMatch(hooks, /Delete\s+.*\.capia/i);
  assert.doesNotMatch(hooks, /RMDir\s+\/r\s+"\$DOCUMENTS/i);
  assert.match(hooks, /CopyFiles/);
});
