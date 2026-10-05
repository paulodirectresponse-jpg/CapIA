// Executa os testes do cliente Python (unittest) dentro de `pnpm test:tools`. Sem python3 o teste é
// PULADO (o motivo aparece na saída do runner), nunca aprovado em silêncio.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const dir = new URL(".", import.meta.url).pathname;
const hasPython = spawnSync("python3", ["--version"], { encoding: "utf8" }).status === 0;

test(
  "cliente REST Python: unittest contra servidor falso",
  { skip: hasPython ? false : "python3 indisponível neste ambiente" },
  () => {
    const r = spawnSync("python3", ["-B", "-m", "unittest", "discover", "-s", dir], {
      encoding: "utf8",
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    });
    assert.equal(r.status, 0, `${r.stdout}\n${r.stderr}`);
  },
);
