#!/usr/bin/env node
// Segurança da autonomia: injeção (brief/metadados/modelo), canário de segredo, aprovações/gasto,
// promoção de memória, Run aninhada, integridade preview/apply, AI Off, fronteira de rede (SafeFetcher),
// arquitetura (único ponto de rede) e UI sem escrita direta.
import { cargo, pnpm, runSuite } from "../lib.mjs";

const s = runSuite("security", [
  {
    req: [
      "injeção no brief e no modelo, canário, aprovações, integridade preview/apply, cancelamento, AI Off",
    ],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_security"]),
  },
  {
    req: ["SafeFetcher: SSRF, redirect, tamanho, tipo, allow-list"],
    cmd: cargo(["-p", "capia-ai", "--test", "fetch"]),
  },
  {
    req: ["fronteira de rede / tools fechadas (fase 4 mantida)"],
    cmd: cargo(["-p", "capia-ai", "--test", "security"]),
  },
  {
    req: ["canário do serviço ai.* (logs, projeto, IPC, AppDb)"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "service"]),
  },
  { req: ["arquitetura: dependências e único ponto de rede"], cmd: pnpm(["check:arch"]) },
  {
    req: ["UI: sem segredo, sem provider, sem escrita direta; Runs só via controlador"],
    cmd: pnpm([
      "--filter",
      "@capia/editor-ui",
      "exec",
      "vitest",
      "run",
      "src/architecture.test.ts",
      "src/components/AiRunsPanel.test.tsx",
    ]),
  },
]);
process.exit(s.passed ? 0 : 1);
