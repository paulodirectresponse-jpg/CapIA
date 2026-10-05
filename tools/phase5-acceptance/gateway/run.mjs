#!/usr/bin/env node
// Asset Gateway + geração: aprovação de licença desconhecida, proveniência, orçamento, desligado ≠ quebra,
// dedup por hash, retry/429, SafeFetcher.
import { cargo, runSuite } from "../lib.mjs";

const s = runSuite("gateway", [
  {
    req: ["gateway/geração: aprovação, proveniência, orçamento, desligado, fallback"],
    cmd: cargo([
      "-p",
      "capia-intelligence",
      "--test",
      "autonomy_scenarios",
      "--",
      "gateway",
      "generation",
      "known_license",
      "rate_limited",
    ]),
  },
  {
    req: ["adapters, licenças, geração (unitários)"],
    cmd: cargo(["-p", "capia-intelligence", "--lib", "autonomy::gateway", "autonomy::generation"]),
  },
  {
    req: ["ACQUIRE e crash"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_crash_acquire"]),
  },
  { req: ["SafeFetcher"], cmd: cargo(["-p", "capia-ai", "--test", "fetch"]) },
]);
process.exit(s.passed ? 0 : 1);
