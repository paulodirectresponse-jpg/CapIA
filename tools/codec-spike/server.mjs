// Servidor local mínimo (sem dependências): serve spike.html + mediabunny e recebe POST /report.
// Uso: node server.mjs <porta> <dir-saida> <caminho-mediabunny.min.mjs>
import { createServer } from "node:http";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const [port = "8765", outDir = "out", mediabunny = ""] = process.argv.slice(2);
mkdirSync(outDir, { recursive: true });
const here = fileURLToPath(new URL(".", import.meta.url));

const server = createServer((req, res) => {
  if (req.method === "POST" && req.url === "/report") {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const report = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      if (report.mp4Base64) {
        writeFileSync(join(outDir, "out.mp4"), Buffer.from(report.mp4Base64, "base64"));
        delete report.mp4Base64;
      }
      writeFileSync(join(outDir, "report.json"), JSON.stringify(report, null, 2));
      res.writeHead(204).end();
      console.log("REPORT_RECEIVED");
      setTimeout(() => process.exit(0), 200);
    });
    return;
  }
  const path = (req.url ?? "/").split("?")[0];
  if (path === "/" || path === "/spike.html") {
    res.writeHead(200, { "Content-Type": "text/html" }).end(readFileSync(join(here, "spike.html")));
  } else if (path === "/mediabunny.min.mjs") {
    res.writeHead(200, { "Content-Type": "text/javascript" }).end(readFileSync(mediabunny));
  } else {
    res.writeHead(404).end();
  }
});
server.listen(Number(port), "127.0.0.1", () => console.log(`LISTENING ${port}`));
