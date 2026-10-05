// Executa os testes do receptor Python (unittest) dentro de `pnpm test:tools`. Sem python3 no PATH
// o teste é PULADO (nunca "passa" em silêncio): o motivo aparece na saída do runner.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const dir = new URL(".", import.meta.url).pathname;
const probe = spawnSync("python3", ["--version"], { encoding: "utf8" });
const hasPython = probe.status === 0;

test(
  "receptor Python: unittest (vetores independentes, janela, dedupe)",
  {
    skip: hasPython ? false : "python3 indisponível neste ambiente",
  },
  () => {
    const r = spawnSync("python3", ["-B", "-m", "unittest", "discover", "-s", dir, "-v"], {
      encoding: "utf8",
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    });
    assert.equal(r.status, 0, `${r.stdout}\n${r.stderr}`);
  },
);
