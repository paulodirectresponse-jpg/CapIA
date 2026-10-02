import { chromium } from "/home/user/CapIA/spikes/s5-timeline-canvas/node_modules/playwright-core/index.mjs";
import path from "node:path";
const b = await chromium.launch({ executablePath: "/opt/pw-browsers/chromium-1194/chrome-linux/chrome", args: ["--no-sandbox", "--disable-gpu", "--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const p = await b.newPage(); await p.goto("file://" + path.resolve("proxy_upload.html"));
const r = await p.evaluate(() => window.run()); for (const x of r) console.log(JSON.stringify(x));
console.log("renderer:", await p.evaluate(() => { const g = document.createElement('canvas').getContext('webgl2'); const e = g.getExtension('WEBGL_debug_renderer_info'); return e ? g.getParameter(e.UNMASKED_RENDERER_WEBGL) : 'n/a'; }));
await b.close();
