// PROJECT_BOOTSTRAP — teste pequeno e independente de GPU (roda ANTES do gpu-parity):
//   1 abre /projects · 2 cria projeto real · 3 obtém projectId · 4 chega em /editor/[projectId]
//   5 __capiaAgent existe · 6 __capiaEditor existe · 7 projeto ativo tem o mesmo id
// Veredito: PROJECT_BOOTSTRAP_PASS | HARNESS_BOOTSTRAP_FAIL | SERVER_BOOT_FAIL (exit 0 só no PASS).
//   EDITOR_URL=http://127.0.0.1:3100 node e2e/project-bootstrap.mjs
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch } from "./harness.mjs";
import {
	BootstrapError,
	bootstrapProject,
	evaluateProjectBootstrap,
	serverPreflight,
} from "./bootstrap.mjs";

const BASE = process.env.EDITOR_URL ?? "http://127.0.0.1:3100";
const OUT = process.env.E2E_OUT ?? join(process.cwd(), "gpu-parity-out");
mkdirSync(OUT, { recursive: true });
const report = { base: BASE, verdict: null };

const pre = await serverPreflight(BASE);
report.preflight = pre;
console.log(
	`[PREFLIGHT] ${pre.ok ? "PASS" : "FAIL"} :: ${pre.ok ? `/projects ${pre.status}, ${pre.assetsChecked} assets ok` : pre.reasons.join(" | ")}`,
);
if (!pre.ok) {
	report.verdict = "SERVER_BOOT_FAIL";
} else {
	const { browser, page } = await launch({
		headless: process.env.HEADED !== "1",
	});
	try {
		const boot = await bootstrapProject(page, BASE, { outDir: OUT });
		const result = await evaluateProjectBootstrap(page, boot);
		report.result = result;
		report.mode = boot.mode;
		report.verdict = result.ok
			? "PROJECT_BOOTSTRAP_PASS"
			: "HARNESS_BOOTSTRAP_FAIL";
		console.log(
			`[PROJECT_BOOTSTRAP] ${result.ok ? "PASS" : "FAIL"} :: id=${boot.projectId} mode=${boot.mode} url=${new URL(boot.url).pathname} ` +
				JSON.stringify(result.criteria),
		);
	} catch (e) {
		report.verdict =
			e instanceof BootstrapError ? e.kind : "HARNESS_BOOTSTRAP_FAIL";
		report.error = String(e.message ?? e);
		console.log(`[PROJECT_BOOTSTRAP] FAIL :: ${report.error}`);
		console.log(
			`  evidência: ${join(OUT, "bootstrap-failure.png")} + bootstrap-diagnostics.json`,
		);
	} finally {
		await browser.close();
	}
}
writeFileSync(
	join(OUT, "project-bootstrap.json"),
	JSON.stringify(report, null, 2),
);
console.log(`VERDICT: ${report.verdict}`);
process.exitCode = report.verdict === "PROJECT_BOOTSTRAP_PASS" ? 0 : 1;
