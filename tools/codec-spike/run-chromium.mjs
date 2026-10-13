// Controle: roda o MESMO spike no Chromium do Playwright (Linux/CI). Esperado: sem H.264/AAC =>
// o harness precisa REPORTAR falha (prova que um PASS no Windows não é falso-positivo).
// Uso: node run-chromium.mjs <dir-saida> <mediabunny.min.mjs> [playwright-core-dir]
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const [out = "out-chromium", mediabunny, pw = "playwright-core"] = process.argv.slice(2);
const require = createRequire(import.meta.url);
const { chromium } = require(pw);
const here = fileURLToPath(new URL(".", import.meta.url));
const port = 8766;
const server = spawn("node", [`${here}server.mjs`, String(port), out, mediabunny], { stdio: "inherit" });
await new Promise((r) => setTimeout(r, 800));
const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const page = await browser.newPage();
await page.goto(`http://127.0.0.1:${port}/spike.html`);
await page.waitForFunction(() => document.title === "DONE", null, { timeout: 120000 });
await browser.close();
server.kill();
