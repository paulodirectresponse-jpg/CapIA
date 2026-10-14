// Pré-checagem do servidor (usada pelo run-windows.ps1): /projects responde e TODOS os assets do build são servidos.
//   node e2e/preflight.mjs http://127.0.0.1:3100   => exit 0 | 2 (SERVER_BOOT_FAIL)
import { serverPreflight } from "./bootstrap.mjs";
const base = process.argv[2] ?? "http://127.0.0.1:3100";
const r = await serverPreflight(base);
console.log(
	r.ok
		? `[PREFLIGHT] PASS :: ${base}/projects ${r.status}, ${r.assetsChecked} assets ok`
		: `[PREFLIGHT] FAIL (SERVER_BOOT_FAIL) :: ${r.reasons.join(" | ")}`,
);
if (!r.ok && r.failures.length)
	console.log(JSON.stringify(r.failures.slice(0, 5)));
process.exit(r.ok ? 0 : 2);
