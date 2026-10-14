// Bootstrap robusto e diagnosticável do projeto para os harnesses E2E (2B.1B).
//
// Falhas aqui NUNCA são "GPU_PARITY_FAIL": são HARNESS_BOOTSTRAP_FAIL (projeto não abriu) ou
// SERVER_BOOT_FAIL (servidor ausente/stale). Toda falha deixa evidência em disco:
//   <out>/bootstrap-failure.png   <out>/bootstrap-diagnostics.json
//
// Espera por NAVEGAÇÃO do Next (client-side) usa `location.pathname`, não o evento `load` do Playwright.
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export class BootstrapError extends Error {
	constructor(message, { kind = "HARNESS_BOOTSTRAP_FAIL", diagnostics } = {}) {
		super(message);
		this.name = "BootstrapError";
		this.kind = kind;
		this.diagnostics = diagnostics;
	}
}

const collectors = new WeakMap();

/** Coleta erros de console/página, requisições falhas/4xx-5xx, pendentes e diálogos desde o launch. */
export function attachDiagnostics(page) {
	const c = {
		consoleErrors: [],
		pageErrors: [],
		failedRequests: [],
		badResponses: [],
		dialogs: [],
		pending: new Map(),
	};
	page.on("console", (m) => {
		if (m.type() === "error") c.consoleErrors.push(m.text().slice(0, 300));
	});
	page.on("pageerror", (e) =>
		c.pageErrors.push(String(e.stack ?? e).slice(0, 600)),
	);
	page.on("request", (r) => c.pending.set(r, r.url()));
	page.on("requestfinished", (r) => c.pending.delete(r));
	page.on("requestfailed", (r) => {
		c.pending.delete(r);
		c.failedRequests.push({
			url: r.url().slice(0, 200),
			error: r.failure()?.errorText ?? "?",
		});
	});
	page.on("response", (r) => {
		if (r.status() >= 400)
			c.badResponses.push({ status: r.status(), url: r.url().slice(0, 200) });
	});
	page.on("dialog", (d) => {
		c.dialogs.push({ type: d.type(), message: d.message().slice(0, 200) });
		void d.dismiss().catch(() => {});
	});
	collectors.set(page, c);
	return c;
}

function snapshotCollector(page) {
	const c = collectors.get(page);
	if (!c) return null;
	return {
		consoleErrors: c.consoleErrors.slice(-30),
		pageErrors: c.pageErrors.slice(-20),
		failedRequests: c.failedRequests.slice(-30),
		badResponses: c.badResponses.slice(-30),
		dialogs: c.dialogs,
		pendingRequests: [...c.pending.values()]
			.slice(0, 20)
			.map((u) => u.slice(0, 200)),
	};
}

/**
 * Fault injection SÓ para o teste negativo do harness (E2E_FAULT):
 *  storage   — IndexedDB/OPFS lançam erro (createNewProject falha ao gravar)
 *  no-button — nenhum botão visível
 *  nav       — em /projects, history.pushState/replaceState viram no-op (router.push não navega)
 */
export async function applyFaultInjection(page, fault) {
	if (!fault) return;
	await page.addInitScript((fault) => {
		if (fault === "storage") {
			const boom = () => {
				throw new Error("E2E_FAULT=storage");
			};
			try {
				IDBFactory.prototype.open = boom;
			} catch {}
			try {
				navigator.storage.getDirectory = () =>
					Promise.reject(new Error("E2E_FAULT=storage"));
			} catch {}
		}
		if (fault === "no-button") {
			document.addEventListener("DOMContentLoaded", () => {
				const style = document.createElement("style");
				style.textContent = "button{display:none!important}";
				document.head.appendChild(style);
			});
		}
		if (fault === "nav" && location.pathname.startsWith("/projects")) {
			// só em /projects: simula "router.push não navega" sem quebrar o roteamento do editor
			history.pushState = () => {};
			history.replaceState = () => {};
		}
	}, fault);
}

export async function serverPreflight(base) {
	const result = {
		ok: false,
		base,
		status: null,
		assetsChecked: 0,
		failures: [],
		reasons: [],
	};
	let html = "";
	try {
		const res = await fetch(`${base}/projects`, { redirect: "follow" });
		result.status = res.status;
		html = await res.text();
		if (res.status !== 200)
			result.reasons.push(`GET /projects => HTTP ${res.status}`);
	} catch (e) {
		result.reasons.push(`GET /projects falhou: ${e.cause?.code ?? e.message}`);
		return result;
	}
	const assets = [
		...new Set(
			[...html.matchAll(/(?:src|href)="(\/_next\/[^"]+\.(?:js|css))"/g)].map(
				(m) => m[1],
			),
		),
	];
	if (assets.length === 0)
		result.reasons.push("nenhum asset /_next/ na página (HTML inesperado)");
	for (const asset of assets) {
		try {
			const res = await fetch(`${base}${asset}`);
			result.assetsChecked++;
			if (res.status !== 200)
				result.failures.push({ asset, status: res.status });
		} catch (e) {
			result.failures.push({ asset, error: e.message });
		}
	}
	if (result.failures.length)
		result.reasons.push(
			`${result.failures.length} asset(s) do build não são servidos — servidor stale/build trocado sob um \`next start\` antigo`,
		);
	result.ok = result.reasons.length === 0;
	return result;
}

async function listButtons(page) {
	return page.evaluate(() =>
		[...document.querySelectorAll("button")].map((b) => {
			const r = b.getBoundingClientRect();
			return {
				text: (b.textContent ?? "").trim().slice(0, 60),
				disabled: b.disabled,
				visible:
					r.width > 0 &&
					r.height > 0 &&
					getComputedStyle(b).visibility !== "hidden",
			};
		}),
	);
}

async function pathnameNow(page) {
	return page.evaluate(() => location.pathname).catch(() => "?");
}

async function bootEvents(page) {
	return page
		.evaluate(() =>
			window.__capiaTrace
				? window.__capiaTrace.read().filter((e) => e.kind === "boot")
				: null,
		)
		.catch(() => null);
}

async function storageProbe(page) {
	return page.evaluate(async () => {
		const out = {};
		try {
			out.indexedDB = await new Promise((resolve) => {
				const req = indexedDB.open("capia-probe", 1);
				req.onerror = () => resolve(`error: ${req.error?.message}`);
				req.onsuccess = () => {
					req.result.close();
					indexedDB.deleteDatabase("capia-probe");
					resolve("ok");
				};
			});
		} catch (e) {
			out.indexedDB = `throw: ${e.message}`;
		}
		try {
			const root = await navigator.storage.getDirectory();
			const h = await root.getFileHandle("capia-probe.txt", { create: true });
			await root.removeEntry(h.name).catch(() => {});
			out.opfs = "ok";
		} catch (e) {
			out.opfs = `error: ${e.message}`;
		}
		try {
			out.persisted = await navigator.storage.persisted();
		} catch {
			out.persisted = null;
		}
		return out;
	});
}

async function waitForEditorGlobals(page, timeout) {
	try {
		await page.waitForFunction(
			() =>
				Boolean(
					window.__capiaAgent && window.__capiaEditor && window.__capiaE2E,
				),
			null,
			{ timeout, polling: 250 },
		);
	} catch {
		const present = await page.evaluate(() => ({
			__capiaAgent: Boolean(window.__capiaAgent),
			__capiaEditor: Boolean(window.__capiaEditor),
			__capiaE2E: Boolean(window.__capiaE2E),
		}));
		throw new Error(
			`globals do editor ausentes após ${timeout}ms: ${JSON.stringify(present)} (build sem NEXT_PUBLIC_CAPIA_E2E_HOOKS=1/AGENT_BRIDGE=1? página não hidratou?)`,
		);
	}
}

async function uiCreate(page, diag) {
	const matcher = /new project|create your first project/i;
	const buttons = await listButtons(page);
	diag.buttons = {
		total: buttons.length,
		matching: buttons.filter((b) => matcher.test(b.text)),
	};
	diag.openDialogsBeforeClick = await page.evaluate(() =>
		[...document.querySelectorAll('[role="dialog"]')].map((d) =>
			(d.textContent ?? "").trim().slice(0, 120),
		),
	);
	if (diag.openDialogsBeforeClick.length) {
		await page.keyboard.press("Escape").catch(() => {});
		await page.waitForTimeout(300);
	}
	const target = page.getByRole("button", { name: matcher }).first();
	diag.target = {
		count: await page.getByRole("button", { name: matcher }).count(),
		visible: await target.isVisible().catch(() => false),
		enabled: await target.isEnabled().catch(() => false),
	};
	diag.urlBeforeClick = page.url();
	await target.click({ timeout: 15000 });
	diag.samples = [];
	const t0 = Date.now();
	const sampleAt = [0, 1000, 5000, 10000];
	let navigated = false;
	for (const at of sampleAt) {
		const wait = at - (Date.now() - t0);
		if (wait > 0) {
			try {
				await page.waitForFunction(
					() => location.pathname.startsWith("/editor/"),
					null,
					{
						timeout: wait,
						polling: 100,
					},
				);
				navigated = true;
			} catch {}
		}
		const pathname = await pathnameNow(page);
		diag.samples.push({
			afterMs: at,
			pathname,
			boot: (await bootEvents(page))?.length ?? null,
		});
		if (pathname.startsWith("/editor/")) {
			navigated = true;
			break;
		}
	}
	if (!navigated) {
		try {
			await page.waitForFunction(
				() => location.pathname.startsWith("/editor/"),
				null,
				{
					timeout: 10000,
					polling: 250,
				},
			);
			navigated = true;
		} catch {}
	}
	return navigated;
}

/**
 * Abre um projeto REAL em /editor/[id].
 * Modo "ui": clica "New project". Se a UI não navegar, registra o diagnóstico e (salvo BOOTSTRAP_STRICT_UI=1)
 * cria o projeto pelo MESMO ProjectManager/storage reais (`__capiaEditor.project.createNewProject`) e abre
 * /editor/[id] — o modo e a falha da UI ficam gravados (nada é escondido).
 */
export async function bootstrapProject(
	page,
	base,
	{
		outDir = process.env.E2E_OUT ?? join(process.cwd(), "gpu-parity-out"),
		name = "E2E project",
	} = {},
) {
	mkdirSync(outDir, { recursive: true });
	const strict = process.env.BOOTSTRAP_STRICT_UI === "1";
	const diag = {
		base,
		startedAt: new Date().toISOString(),
		strictUi: strict,
		mode: "ui",
		steps: [],
	};
	const step = (s) =>
		diag.steps.push({ at: Date.now() - Date.parse(diag.startedAt), step: s });
	const fail = async (message, kind = "HARNESS_BOOTSTRAP_FAIL") => {
		diag.failure = message;
		diag.finalUrl = page.url();
		diag.bootEvents = await bootEvents(page);
		diag.collector = snapshotCollector(page);
		diag.storage = await storageProbe(page).catch((e) => ({
			probeError: String(e),
		}));
		diag.buttonsNow = await listButtons(page).catch(() => null);
		await page
			.screenshot({ path: join(outDir, "bootstrap-failure.png") })
			.catch(() => {});
		writeFileSync(
			join(outDir, "bootstrap-diagnostics.json"),
			JSON.stringify(diag, null, 2),
		);
		throw new BootstrapError(message, { kind, diagnostics: diag });
	};

	try {
		step("goto /projects");
		await page.goto(`${base}/projects`, { waitUntil: "domcontentloaded" });
		await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {
			diag.networkIdleTimeout = true;
		});
		step("projects loaded");
		try {
			await page.waitForFunction(
				() => Boolean(window.__capiaEditor && window.__capiaTrace),
				null,
				{
					timeout: 30000,
					polling: 250,
				},
			);
		} catch {
			await fail(
				"/projects não expôs __capiaEditor/__capiaTrace em 30s (build sem NEXT_PUBLIC_CAPIA_E2E_HOOKS=1 ou página não hidratou)",
			);
		}
		diag.storageBefore = await storageProbe(page);

		step("ui create");
		let navigated = false;
		try {
			navigated = await uiCreate(page, diag);
		} catch (e) {
			diag.uiError = String(e.message ?? e).split("\n")[0];
		}
		diag.uiBootEvents = await bootEvents(page);

		if (!navigated) {
			diag.uiFailure =
				diag.uiError ?? "URL não mudou para /editor/ após o clique";
			if (strict)
				await fail(`UI não navegou (BOOTSTRAP_STRICT_UI=1): ${diag.uiFailure}`);
			step("programmatic fallback");
			diag.mode = "programmatic-fallback";
			let created;
			try {
				created = await page.evaluate(
					(name) => window.__capiaEditor.project.createNewProject({ name }),
					name,
				);
			} catch (e) {
				diag.fallbackError = String(e.message ?? e).split("\n")[0];
				await fail(
					`createNewProject falhou também pelo caminho programático: ${diag.fallbackError}`,
				);
			}
			diag.projectId = created;
			// goto pode abortar se o Next também estiver navegando (corrida real): tenta de novo e
			// confia no pathname final, não no retorno do goto.
			diag.fallbackGotoErrors = [];
			let arrived = false;
			for (let attempt = 0; attempt < 3 && !arrived; attempt++) {
				await page
					.goto(`${base}/editor/${created}`, { waitUntil: "commit" })
					.catch((e) => {
						diag.fallbackGotoErrors.push(String(e.message ?? e).split("\n")[0]);
					});
				try {
					await page.waitForFunction(
						(id) => location.pathname === `/editor/${id}`,
						created,
						{ timeout: 10000, polling: 250 },
					);
					arrived = true;
				} catch {
					await page.waitForTimeout(1000);
				}
			}
			if (!arrived)
				await fail(
					`fallback: não chegou em /editor/${created} após 3 tentativas (pathname=${await pathnameNow(page)})`,
				);
		}

		const pathname = await pathnameNow(page);
		const projectId = pathname.split("/")[2];
		if (!projectId)
			await fail(`pathname inesperado após criar o projeto: ${pathname}`);
		diag.projectId = projectId;
		step("editor route");

		try {
			await waitForEditorGlobals(page, 45000);
		} catch (e) {
			await fail(String(e.message ?? e));
		}
		step("editor globals");
		try {
			await page.waitForFunction(
				() => {
					// getActive() LANÇA enquanto o projeto não carregou: tratar como "ainda não".
					try {
						return (
							window.__capiaEditor.project.getActive()?.metadata?.id ?? false
						);
					} catch {
						return false;
					}
				},
				null,
				{ timeout: 20000, polling: 250 },
			);
		} catch {
			await fail("projeto ativo ausente no editor após 20s");
		}
		diag.activeProjectId = await page.evaluate(() => {
			try {
				return window.__capiaEditor.project.getActive().metadata.id;
			} catch {
				return null;
			}
		});
		if (diag.activeProjectId !== projectId)
			await fail(
				`projeto ativo (${diag.activeProjectId}) ≠ id da rota (${projectId})`,
			);

		await page.waitForTimeout(2500); // o diálogo de onboarding aparece depois do load
		for (let i = 0; i < 3; i++) {
			const close = page.getByRole("button", { name: /^close$/i });
			if (await close.count())
				await close
					.first()
					.click()
					.catch(() => {});
		}
		step("ready");
		diag.finalUrl = page.url();
		diag.ok = true;
		writeFileSync(
			join(outDir, "bootstrap-diagnostics.json"),
			JSON.stringify(diag, null, 2),
		);
		return {
			projectId,
			mode: diag.mode,
			url: page.url(),
			activeProjectId: diag.activeProjectId,
			diagnostics: diag,
		};
	} catch (e) {
		if (e instanceof BootstrapError) throw e;
		return fail(
			`erro inesperado no bootstrap: ${String(e.stack ?? e)
				.split("\n")
				.slice(0, 3)
				.join(" | ")}`,
		);
	}
}

/** Os 7 critérios do teste PROJECT_BOOTSTRAP, a partir do resultado de `bootstrapProject`. */
export async function evaluateProjectBootstrap(page, result) {
	const globals = await page.evaluate(() => ({
		agent: Boolean(window.__capiaAgent),
		editor: Boolean(window.__capiaEditor),
		e2e: Boolean(window.__capiaE2E),
		active: (() => {
			try {
				return window.__capiaEditor?.project?.getActive()?.metadata?.id ?? null;
			} catch {
				return null;
			}
		})(),
	}));
	const pathname = new URL(page.url()).pathname;
	const criteria = {
		"1 abriu /projects": result.diagnostics.steps.some(
			(s) => s.step === "projects loaded",
		),
		"2 projeto real criado": Boolean(result.projectId),
		"3 projectId obtido": /^[0-9a-f-]{36}$/i.test(result.projectId ?? ""),
		"4 chegou em /editor/[projectId]":
			pathname === `/editor/${result.projectId}`,
		"5 __capiaAgent existe": globals.agent,
		"6 __capiaEditor existe": globals.editor,
		"7 projeto ativo = projectId": globals.active === result.projectId,
	};
	return {
		ok: Object.values(criteria).every(Boolean),
		criteria,
		globals,
		mode: result.mode,
		projectId: result.projectId,
	};
}
