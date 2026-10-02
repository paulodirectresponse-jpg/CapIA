import { chromium } from "playwright-core";
import path from "node:path";
const exe = "/opt/pw-browsers/chromium-1194/chrome-linux/chrome";
import fs from "node:fs"; const exePath = fs.existsSync(exe) ? exe : undefined;
const browser = await chromium.launch({ executablePath: exePath, args: ["--no-sandbox", "--disable-gpu"] });
const page = await browser.newPage({ viewport: { width: 1920, height: 500 } });
await page.goto("file://" + path.resolve("timeline.html"));
const out = [];
for (const [renderer, scenario, frames] of [["canvas","scroll",240],["canvas","zoom",240],["canvas","drag",240],["canvas","fit",120],["dom","zoom",60],["dom","drag",60],["dom","scroll",120],["dom","fit",30]]) {
  const r = await page.evaluate(([a,b,c]) => window.runScenario(a,b,c), [renderer, scenario, frames]);
  console.log(JSON.stringify(r)); out.push(r);
}
console.log("UA:", await page.evaluate(() => navigator.userAgent), "| cores:", await page.evaluate(() => navigator.hardwareConcurrency));
fs.writeFileSync("/tmp/claude-0/s5-result.json", JSON.stringify(out, null, 2));
await browser.close();
