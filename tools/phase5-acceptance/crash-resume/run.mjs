#!/usr/bin/env node
// Kill/restart: failpoints em cada fronteira de stage (em processo) + SIGKILL real de um processo-filho
// estacionado no failpoint, retomando em OUTRO processo sem duplicar edição/download/geração.
import { cargo, runSuite } from "../lib.mjs";

const s = runSuite("crash-resume", [
  {
    req: ["failpoints por stage: sem edição duplicada, timeline = execução limpa"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_crash"]),
  },
  {
    req: ["ACQUIRE: depois do download / depois do submit de geração: nunca duas vezes"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_crash_acquire"]),
  },
  {
    req: ["SIGKILL real + retomada em outro processo"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_kill"]),
  },
  {
    req: ["estágios started→interrupted, nunca completed (store)"],
    cmd: cargo(["-p", "capia-store", "--test", "autonomy_store"]),
  },
]);
process.exit(s.passed ? 0 : 1);
