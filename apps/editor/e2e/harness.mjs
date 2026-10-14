// Harness E2E do editor OpenCut integrado (Playwright + ponte HTTP loopback para a AgentSurface).
import { execFile, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright-core";

export function findChromium() {
	if (process.env.CHROMIUM_PATH) return process.env.CHROMIUM_PATH;
	const roots = [
		process.env.PLAYWRIGHT_BROWSERS_PATH,
		"/opt/pw-browsers",
		join(
			process.env.HOME ?? process.env.USERPROFILE ?? "",
			".cache/ms-playwright",
		),
		join(process.env.LOCALAPPDATA ?? "", "ms-playwright"),
	].filter(Boolean);
	for (const root of roots) {
		if (!existsSync(root)) continue;
		for (const dir of readdirSync(root).filter((d) =>
			d.startsWith("chromium-"),
		)) {
			for (const rel of [
				"chrome-linux/chrome",
				"chrome-linux64/chrome",
				"chrome-win/chrome.exe",
				"chrome-win64/chrome.exe",
			]) {
				const p = join(root, dir, rel);
				if (existsSync(p)) return p;
			}
		}
	}
	return undefined; // deixa o Playwright resolver (channel do sistema)
}

// WebGPU de SwiftShader entrega canvas vazio; o compositor cai no WebGL2 (sem --enable-unsafe-webgpu).
export const SOFTWARE_GL_ARGS = [
	"--use-gl=angle",
	"--use-angle=swiftshader",
	"--enable-unsafe-swiftshader",
	"--autoplay-policy=no-user-gesture-required",
];

export async function launch({ headless = true } = {}) {
	const real = process.env.GPU_REAL === "1";
	// GPU real: Chrome/Edge INSTALADO (tem H.264 e GPU de verdade), janela visível (WebGPU em headless
	// costuma cair em software). BROWSER_CHANNEL=chrome|msedge.
	const browser = await chromium.launch({
		executablePath: real ? process.env.CHROMIUM_PATH : findChromium(),
		channel:
			real && !process.env.CHROMIUM_PATH
				? (process.env.BROWSER_CHANNEL ?? "chrome")
				: undefined,
		headless: real ? false : headless,
		args: real
			? [
					"--autoplay-policy=no-user-gesture-required",
					"--enable-unsafe-webgpu",
					"--ignore-gpu-blocklist",
				]
			: SOFTWARE_GL_ARGS,
	});
	const context = await browser.newContext({
		viewport: { width: 1700, height: 950 },
		acceptDownloads: true,
	});
	const page = await context.newPage();
	return { browser, context, page };
}

export function makeMedia(dir = join(tmpdir(), "capia-e2e-media")) {
	mkdirSync(dir, { recursive: true });
	const main = join(dir, "main.webm");
	if (!existsSync(main)) {
		execFileSync("ffmpeg", [
			"-loglevel",
			"error",
			"-y",
			"-f",
			"lavfi",
			"-i",
			"testsrc2=size=1280x720:rate=30:duration=12",
			"-f",
			"lavfi",
			"-i",
			"sine=frequency=440:duration=12",
			"-c:v",
			"libvpx-vp9",
			"-b:v",
			"1M",
			"-c:a",
			"libopus",
			"-shortest",
			main,
		]);
	}
	return { dir, main };
}

/** Ponte HTTP loopback → `window.__capiaAgent.call` da página viva (a MESMA instância da UI). */
export async function startBridge(page) {
	const server = createServer((req, res) => {
		let body = "";
		req.on("data", (c) => (body += c));
		req.on("end", async () => {
			try {
				const { method, params } = JSON.parse(body || "{}");
				const result = await page.evaluate(
					({ m, p }) => {
						if (!window.__capiaAgent)
							throw new Error(
								"window.__capiaAgent ausente (build com NEXT_PUBLIC_CAPIA_AGENT_BRIDGE=1)",
							);
						return window.__capiaAgent.call({ method: m, params: p });
					},
					{ m: method, p: params ?? {} },
				);
				const out = JSON.stringify({ ok: true, result });
				res
					.writeHead(200, {
						"Content-Type": "application/json",
						"Content-Length": Buffer.byteLength(out),
					})
					.end(out);
			} catch (e) {
				const out = JSON.stringify({
					ok: false,
					error: { code: "BRIDGE", message: String(e.message ?? e) },
				});
				res
					.writeHead(200, {
						"Content-Type": "application/json",
						"Content-Length": Buffer.byteLength(out),
					})
					.end(out);
			}
		});
	});
	await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
	return {
		server,
		addr: `127.0.0.1:${server.address().port}`,
		close: () => server.close(),
	};
}

/** Executa a CLI do EnginePort (Rust) SEM bloquear o event loop (a ponte HTTP roda neste processo). */
export function portCli(addr, args) {
	const bin = process.env.CAPIA_PORT_CLI;
	const cmd = bin ?? "cargo";
	const full = bin
		? [addr, ...args]
		: [
				"run",
				"-q",
				"-p",
				"capia-engine-port",
				"--example",
				"port_cli",
				"--",
				addr,
				...args,
			];
	const cwd =
		process.env.CAPIA_REPO_ROOT ?? join(import.meta.dirname, "../../..");
	return new Promise((resolve) => {
		execFile(
			cmd,
			full,
			{ encoding: "utf8", cwd, maxBuffer: 16 * 1024 * 1024 },
			(error, stdout, stderr) => {
				const line =
					(stdout ?? "").trim().split("\n").filter(Boolean).at(-1) ?? "{}";
				try {
					resolve(JSON.parse(line));
				} catch {
					resolve({
						ok: false,
						error: {
							code: "CLI",
							message: `${stderr}\n${line}\n${error ?? ""}`,
						},
					});
				}
			},
		);
	});
}

export async function openNewProject(page, base) {
	await page.goto(`${base}/projects`, { waitUntil: "networkidle" });
	await page
		.getByRole("button", { name: /new project|create your first project/i })
		.first()
		.click();
	await page.waitForURL(/editor\//);
	await page.waitForFunction(() => window.__capiaAgent !== undefined, null, {
		timeout: 30000,
	});
	await page.waitForTimeout(2500); // o diálogo de onboarding aparece depois do load
	for (let i = 0; i < 3; i++) {
		const close = page.getByRole("button", { name: /^close$/i });
		if (await close.count())
			await close
				.first()
				.click()
				.catch(() => {});
	}
}

export async function importFiles(page, files) {
	const [chooser] = await Promise.all([
		page.waitForEvent("filechooser"),
		page
			.getByRole("button", { name: /^import/i })
			.first()
			.click(),
	]);
	await chooser.setFiles(files);
	await page.waitForFunction(
		(n) =>
			window.__capiaAgent.call({ method: "inspect_project" }).project.assets
				.length >= n,
		files.length,
		{ timeout: 60000 },
	);
}

export async function dragAssetToTimeline(page, nameRe, x, y) {
	const opt = page.getByRole("option").filter({ hasText: nameRe }).first();
	const bb = await opt.boundingBox();
	await page.mouse.move(bb.x + bb.width / 2, bb.y + bb.height / 2);
	await page.mouse.down();
	await page.mouse.move(bb.x + bb.width / 2 + 30, bb.y + bb.height / 2 + 30, {
		steps: 4,
	});
	await page.mouse.move(x, y, { steps: 14 });
	await page.waitForTimeout(250);
	await page.mouse.up();
	await page.waitForTimeout(800);
}

export const call = (page, method, params = {}) =>
	page.evaluate(
		({ m, p }) => window.__capiaAgent.call({ method: m, params: p }),
		{ m: method, p: params },
	);
