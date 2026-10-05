#!/usr/bin/env node
// Memória em 4 escopos: proposta ≠ ativa, promoção só com aprovação humana, precedência, isolamento,
// rejeitado não ressuscita, exclusão sem conteúdo no log, sinais de correção.
import { cargo, pnpm, runSuite } from "../lib.mjs";

const s = runSuite("memory", [
  {
    req: ["escopos, promoção, precedência, isolamento, exclusão"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_memory"]),
  },
  {
    req: ["propriedade: User/Client ativos só com aprovação humana"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_properties", "--", "memory"]),
  },
  {
    req: ["UI: proposta inativa até clique explícito"],
    cmd: pnpm([
      "--filter",
      "@capia/editor-ui",
      "exec",
      "vitest",
      "run",
      "src/components/AiRunsPanel.test.tsx",
    ]),
  },
]);
process.exit(s.passed ? 0 : 1);
