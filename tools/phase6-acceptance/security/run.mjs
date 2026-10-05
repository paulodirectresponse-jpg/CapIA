#!/usr/bin/env node
// Orquestrador declarativo: lê `steps.json` e reporta cada passo como
// passed | failed | pending_external | not_available (nunca "pulado" em silêncio).
//   node tools/phase6-acceptance/security/run.mjs [--list] [--only id[,id]] [--strict]
// Resumo em target/phase6-acceptance/security-summary.json. Ver README.md desta pasta.
import { fileURLToPath } from "node:url";
import { cli } from "../steps-runner.mjs";

process.exit(cli(fileURLToPath(new URL("./steps.json", import.meta.url))));
