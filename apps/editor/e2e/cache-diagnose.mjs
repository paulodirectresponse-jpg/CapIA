// Diagnóstico do cache de textura externa (2B.1): seeks FORA DE ORDEM num clip de vídeo, registrando
// frame do decoder × identidade do Canvas × decisão do cache de textura × hash do snapshot.
// Precisa de build com NEXT_PUBLIC_CAPIA_E2E_HOOKS=1 NEXT_PUBLIC_CAPIA_AGENT_BRIDGE=1.
//   EDITOR_URL=http://127.0.0.1:3100 node e2e/cache-diagnose.mjs [saida.json]
import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import {
	call,
	importFiles,
	launch,
	makeMedia,
	openNewProject,
} from "./harness.mjs";

const BASE = process.env.EDITOR_URL ?? "http://127.0.0.1:3100";
const OUT = process.argv[2] ?? "cache-diagnose.json";
const SEEKS = (process.env.SEEKS ?? "0.5,2.5,3.5,5.5,0.5,3.5,2.5")
	.split(",")
	.map(Number);
const T = 120_000;

const media = makeMedia();
const { browser, page } = await launch();
const result = { seeks: [], verdict: null };
try {
	await page.addInitScript(() => {
		window.__CAPIA_TRACE__ = true;
	});
	await openNewProject(page, BASE);
	await importFiles(page, [media.main]);
	await page.waitForFunction(
		() => window.__capiaEditor && window.__capiaTrace,
		null,
		{ timeout: 15000 },
	);
	const vid = (await call(page, "inspect_project")).project.assets.find(
		(a) => a.type === "video",
	).id;
	const r = await page.evaluate(
		({ vid }) => {
			const rev = window.__capiaAgent.call({ method: "inspect_project" })
				.project.revision;
			return window.__capiaAgent.call({
				method: "apply_plan",
				params: {
					plan: {
						schemaVersion: 1,
						planId: "diag",
						actor: { kind: "cli", id: "diag" },
						baseRevision: rev,
						operations: [
							{
								type: "insert_clip",
								operationId: "v",
								mediaId: vid,
								startTicks: 0,
								ref: "v",
							},
						],
					},
				},
			});
		},
		{ vid },
	);
	if (!r.ok) throw new Error(`apply_plan: ${JSON.stringify(r.error)}`);
	await page.waitForTimeout(1000);

	for (const t of SEEKS) {
		await page.evaluate(() => window.__capiaTrace.clear());
		await page.evaluate(
			(t) =>
				window.__capiaEditor.playback.seek({ time: Math.round(t * 120000) }),
			t,
		);
		await page.waitForTimeout(1500);
		const snap = await page.evaluate(async () => {
			const s = await window.__capiaEditor.renderer.createSnapshot();
			const u8 = new Uint8Array(await s.blob.arrayBuffer());
			let str = "";
			for (let i = 0; i < u8.length; i += 0x8000)
				str += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
			return btoa(str);
		});
		const events = await page.evaluate(() => window.__capiaTrace.read());
		// pareia cada quadro de vídeo resolvido com a decisão de textura que o seguiu
		const renders = [];
		for (let i = 0; i < events.length; i++) {
			if (events[i].kind !== "video-frame" || events[i].site !== "layer")
				continue;
			const tex = events
				.slice(i + 1)
				.find((e) => e.kind === "texture" && e.textureId.endsWith(":source"));
			renders.push({ frame: events[i], texture: tex ?? null });
		}
		result.seeks.push({
			seekSeconds: t,
			snapshotSha256: createHash("sha256")
				.update(Buffer.from(snap, "base64"))
				.digest("hex")
				.slice(0, 16),
			renders,
		});
	}

	// condição do bug: MESMO objeto Canvas + timestamp DIFERENTE + cache HIT
	const flat = result.seeks.flatMap((s) =>
		s.renders.map((r) => ({ seek: s.seekSeconds, ...r })),
	);
	const violations = [];
	for (let i = 1; i < flat.length; i++) {
		const a = flat[i - 1];
		const b = flat[i];
		if (
			a.frame.canvasId === b.frame.canvasId &&
			a.frame.frameTimestamp !== b.frame.frameTimestamp &&
			b.texture?.decision === "hit"
		) {
			violations.push({
				prev: {
					seek: a.seek,
					ts: a.frame.frameTimestamp,
					canvas: a.frame.canvasId,
				},
				cur: {
					seek: b.seek,
					ts: b.frame.frameTimestamp,
					canvas: b.frame.canvasId,
					decision: b.texture?.decision,
				},
			});
		}
	}
	result.violations = violations;
	result.verdict =
		violations.length > 0
			? "MUTABLE_CANVAS_IDENTITY_CACHE_BUG_CONFIRMED"
			: "NOT_REPRODUCED";
	// seeks distintos => snapshots distintos (o vídeo avança no tempo)
	const distinct = new Set(result.seeks.map((s) => s.snapshotSha256)).size;
	result.distinctSnapshots = distinct;
	result.distinctSeekTimes = new Set(SEEKS).size;
} catch (e) {
	result.error = String(e.stack ?? e);
} finally {
	writeFileSync(OUT, JSON.stringify(result, null, 2));
	await browser.close();
	console.log(
		JSON.stringify(
			{
				verdict: result.verdict,
				distinctSnapshots: result.distinctSnapshots,
				distinctSeekTimes: result.distinctSeekTimes,
				violations: result.violations?.length,
				error: result.error,
			},
			null,
			1,
		),
	);
}
