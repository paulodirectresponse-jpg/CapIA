// Paridade preview × snapshot × export numa cena com vídeo, overlay de imagem, texto, transform,
// máscara, blur e keyframes. Pronto para PC Windows COM GPU real (GPU_REAL=1 + Edge/Chrome):
//   $env:GPU_REAL=1; $env:EDITOR_URL="http://127.0.0.1:3100"; node e2e/gpu-parity.mjs
// Veredito: GPU_PARITY_PASS | GPU_PARITY_FAIL | GPU_PARITY_PENDING_HUMAN_MACHINE (renderer de software
// nunca produz PASS, mesmo que os quadros batam).
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
	call,
	importFiles,
	launch,
	makeMedia,
	openNewProject,
} from "./harness.mjs";

const BASE = process.env.EDITOR_URL ?? "http://127.0.0.1:3100";
const OUT = process.env.E2E_OUT ?? join(process.cwd(), "gpu-parity-out");
const SAMPLES = (process.env.SAMPLES ?? "0.5,2.5,3.5,5.5")
	.split(",")
	.map(Number);
const PSNR_EXPORT_MIN = Number(process.env.PSNR_EXPORT_MIN ?? 26);
const PSNR_PREVIEW_MIN = Number(process.env.PSNR_PREVIEW_MIN ?? 24);
const T = 120_000;
mkdirSync(OUT, { recursive: true });

const media = makeMedia();
const overlay = join(media.dir, "overlay.png");
execFileSync("ffmpeg", [
	"-loglevel",
	"error",
	"-y",
	"-f",
	"lavfi",
	"-i",
	"color=c=0xff3366:s=640x360,format=rgba,drawbox=x=40:y=40:w=200:h=120:color=white@1:t=fill,drawbox=x=360:y=200:w=220:h=120:color=cyan@1:t=fill",
	"-frames:v",
	"1",
	overlay,
]);

function psnr(a, b) {
	const r = spawnSync(
		"ffmpeg",
		[
			"-hide_banner",
			"-i",
			a,
			"-i",
			b,
			"-lavfi",
			"[0:v]scale=320:180,format=rgb24[a];[1:v]scale=320:180,format=rgb24[b];[a][b]psnr",
			"-f",
			"null",
			"-",
		],
		{ encoding: "utf8" },
	);
	const m = /average:([\d.]+|inf)/.exec(r.stderr ?? "");
	return m ? (m[1] === "inf" ? 99 : Number(m[1])) : null;
}

function distinctColors(png) {
	const r = spawnSync(
		"ffmpeg",
		[
			"-loglevel",
			"error",
			"-i",
			png,
			"-vf",
			"scale=64:36,format=rgb24",
			"-f",
			"rawvideo",
			"-",
		],
		{ encoding: "buffer", maxBuffer: 1 << 24 },
	);
	const set = new Set();
	for (let i = 0; i + 2 < r.stdout.length; i += 3)
		set.add(
			(r.stdout[i] >> 4) * 256 +
				(r.stdout[i + 1] >> 4) * 16 +
				(r.stdout[i + 2] >> 4),
		);
	return set.size;
}

const { browser, page } = await launch({
	headless: process.env.HEADED !== "1",
});
const verdict = {
	base: BASE,
	samples: SAMPLES,
	thresholds: { PSNR_EXPORT_MIN, PSNR_PREVIEW_MIN },
	frames: [],
	checks: [],
};
const check = (name, ok, detail) => {
	verdict.checks.push({ name, ok, detail });
	console.log(`[${ok ? "PASS" : "FAIL"}] ${name} :: ${detail}`);
};

try {
	await openNewProject(page, BASE);
	await importFiles(page, [media.main, overlay]);
	await page.waitForFunction(
		() => window.__capiaEditor && window.__capiaE2E,
		null,
		{ timeout: 15000 },
	);

	verdict.gpu = await page.evaluate(async () => {
		const info = {};
		try {
			const gl = document.createElement("canvas").getContext("webgl2");
			const ext = gl?.getExtension("WEBGL_debug_renderer_info");
			info.webgl2 = ext
				? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
				: gl
					? "webgl2 (renderer hidden)"
					: null;
		} catch {
			info.webgl2 = null;
		}
		try {
			const a = await navigator.gpu?.requestAdapter();
			info.webgpu = a
				? {
						vendor: a.info?.vendor,
						architecture: a.info?.architecture,
						description: a.info?.description,
					}
				: null;
		} catch {
			info.webgpu = null;
		}
		info.h264 =
			typeof VideoEncoder !== "undefined"
				? (
						await VideoEncoder.isConfigSupported({
							codec: "avc1.42001f",
							width: 1280,
							height: 720,
							bitrate: 2e6,
							framerate: 30,
						})
					).supported
				: false;
		return info;
	});
	const gpuText = JSON.stringify(verdict.gpu);
	verdict.softwareRenderer =
		/swiftshader|llvmpipe|software|warp|basic render|microsoft basic/i.test(
			gpuText,
		);
	console.log("GPU:", gpuText, "software=", verdict.softwareRenderer);

	// --- cena
	const plan = (id, ops) =>
		call(page, "apply_plan", {
			plan: {
				schemaVersion: 1,
				planId: id,
				actor: { kind: "cli", id: "gpu-parity" },
				baseRevision: undefined,
				operations: ops,
			},
		});
	const rev = async () =>
		(await call(page, "inspect_project")).project.revision;
	const assets = (await call(page, "inspect_project")).project.assets;
	const vid = assets.find((a) => a.type === "video").id;
	const img = assets.find((a) => a.type === "image").id;
	const applyPlan = async (id, operations) =>
		page.evaluate(
			({ id, operations }) => {
				const rev = window.__capiaAgent.call({ method: "inspect_project" })
					.project.revision;
				return window.__capiaAgent.call({
					method: "apply_plan",
					params: {
						plan: {
							schemaVersion: 1,
							planId: id,
							actor: { kind: "cli", id: "gpu-parity" },
							baseRevision: rev,
							operations,
						},
					},
				});
			},
			{ id, operations },
		);
	const r1 = await applyPlan("scene-1", [
		{
			type: "insert_clip",
			operationId: "v",
			mediaId: vid,
			startTicks: 0,
			ref: "video",
		},
		{
			type: "insert_clip",
			operationId: "i",
			mediaId: img,
			startTicks: 1 * T,
			ref: "image",
		},
	]);
	check(
		"cena: inserir vídeo + imagem (AgentSurface)",
		r1.ok === true,
		JSON.stringify(r1.error ?? r1.receipt?.diff?.added?.length),
	);
	const tracks = (await call(page, "inspect_tracks")).tracks;
	const clips = tracks.flatMap((t) => t.clips);
	const videoClip = clips.find((c) => c.type === "video");
	const imageClip = clips.find((c) => c.type === "image");

	const setup = await page.evaluate(
		({ videoClip, imageClip }) => {
			const ed = window.__capiaEditor;
			const E2E = window.__capiaE2E;
			const find = (id) => {
				const t = ed.scenes.getActiveScene().tracks;
				for (const tr of [...t.overlay, t.main, ...t.audio]) {
					const el = tr.elements.find((e) => e.id === id);
					if (el) return { tr, el };
				}
				return null;
			};
			const out = {};
			// transform + opacity na imagem
			const im = find(imageClip.id);
			ed.timeline.updateElements({
				updates: [
					{
						trackId: im.tr.id,
						elementId: im.el.id,
						patch: {
							duration: Math.round(5 * 120000),
							params: {
								...im.el.params,
								"transform.scaleX": 0.5,
								"transform.scaleY": 0.5,
								"transform.positionX": 220,
								"transform.positionY": -110,
								"transform.rotate": 20,
								opacity: 0.9,
							},
						},
					},
				],
			});
			// máscara (elipse) na imagem
			ed.timeline.updateElements({
				updates: [
					{
						trackId: im.tr.id,
						elementId: im.el.id,
						patch: {
							masks: [E2E.buildDefaultMaskInstance({ maskType: "ellipse" })],
						},
					},
				],
			});
			// blur no vídeo
			const vd = find(videoClip.id);
			out.effectId = ed.timeline.addClipEffect({
				trackId: vd.tr.id,
				elementId: vd.el.id,
				effectType: "blur",
			});
			// texto
			ed.timeline.insertElement({
				element: E2E.buildTextElement({
					raw: {
						duration: Math.round(4 * 120000),
						params: {
							content: "CapIA GPU parity",
							fontSize: 64,
							color: "#ffff00",
						},
					},
					startTime: Math.round(2 * 120000),
				}),
				placement: { mode: "auto" },
			});
			return out;
		},
		{ videoClip, imageClip },
	);
	check(
		"cena: transform, máscara, blur e texto aplicados",
		Boolean(setup.effectId),
		JSON.stringify(setup),
	);

	const textClip = (await call(page, "inspect_tracks")).tracks
		.flatMap((t) => t.clips.map((c) => ({ ...c, trackId: t.id })))
		.find((c) => c.type === "text");
	await page.evaluate(
		({ textClip }) => {
			const ed = window.__capiaEditor;
			ed.timeline.upsertKeyframes({
				keyframes: [
					{
						trackId: textClip.trackId,
						elementId: textClip.id,
						propertyPath: "opacity",
						time: 0,
						value: 0.2,
					},
					{
						trackId: textClip.trackId,
						elementId: textClip.id,
						propertyPath: "opacity",
						time: Math.round(1 * 120000),
						value: 1,
					},
				],
			});
		},
		{ textClip },
	);
	const withKeys = await call(page, "inspect_clip", { clipId: textClip.id });
	check(
		"cena: keyframes de opacidade no texto",
		JSON.stringify(withKeys.clip.element.animations ?? {}).includes("opacity"),
		"animations.opacity",
	);
	await page.waitForTimeout(800);

	// --- amostras: snapshot do compositor × captura do preview na tela
	const previewBox = await page.evaluate(() => {
		const c = [...document.querySelectorAll("canvas")].find(
			(x) => x.width >= 640 && x.getBoundingClientRect().width > 300,
		);
		const r = c.getBoundingClientRect();
		return { x: r.x, y: r.y, width: r.width, height: r.height };
	});
	for (const t of SAMPLES) {
		await page.evaluate(
			(t) =>
				window.__capiaEditor.playback.seek({ time: Math.round(t * 120000) }),
			t,
		);
		await page.waitForTimeout(1800);
		const snapB64 = await page.evaluate(async () => {
			const s = await window.__capiaEditor.renderer.createSnapshot();
			const u8 = new Uint8Array(await s.blob.arrayBuffer());
			let str = "";
			for (let i = 0; i < u8.length; i += 0x8000)
				str += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
			return btoa(str);
		});
		const snap = join(OUT, `snapshot-${t}.png`);
		writeFileSync(snap, Buffer.from(snapB64, "base64"));
		const shot = join(OUT, `preview-${t}.png`);
		await page.screenshot({ path: shot, clip: previewBox });
		verdict.frames.push({
			t,
			snap,
			shot,
			colors: distinctColors(snap),
			psnrPreviewVsSnapshot: psnr(shot, snap),
		});
	}

	// --- export (mp4 se a plataforma tiver H.264; senão webm) e comparação por quadro
	const format = verdict.gpu.h264 ? "mp4" : "webm";
	const t0 = Date.now();
	const exp = await page.evaluate(async (format) => {
		const res = await window.__capiaEditor.project.export({
			options: { format, quality: "high", includeAudio: false },
		});
		if (!res.success) return { ok: false, error: res.error };
		const u8 = new Uint8Array(res.buffer);
		let str = "";
		for (let i = 0; i < u8.length; i += 0x8000)
			str += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
		return { ok: true, size: u8.length, b64: btoa(str) };
	}, format);
	verdict.export = {
		format,
		ok: exp.ok,
		error: exp.error,
		size: exp.size,
		ms: Date.now() - t0,
	};
	check(
		`export ${format}`,
		exp.ok,
		JSON.stringify({ size: exp.size, ms: verdict.export.ms, error: exp.error }),
	);
	if (exp.ok) {
		const file = join(OUT, `export.${format}`);
		writeFileSync(file, Buffer.from(exp.b64, "base64"));
		for (const f of verdict.frames) {
			const out = join(OUT, `export-${f.t}.png`);
			execFileSync("ffmpeg", [
				"-loglevel",
				"error",
				"-y",
				"-ss",
				String(f.t),
				"-i",
				file,
				"-frames:v",
				"1",
				out,
			]);
			f.exportFrame = out;
			f.psnrExportVsSnapshot = psnr(out, f.snap);
		}
	}

	for (const f of verdict.frames) {
		check(
			`t=${f.t}s: snapshot não-vazio`,
			f.colors >= 6,
			`${f.colors} cores distintas`,
		);
		check(
			`t=${f.t}s: preview(tela) ≈ snapshot`,
			f.psnrPreviewVsSnapshot !== null &&
				f.psnrPreviewVsSnapshot >= PSNR_PREVIEW_MIN,
			`PSNR ${f.psnrPreviewVsSnapshot}`,
		);
		check(
			`t=${f.t}s: export ≈ snapshot`,
			f.psnrExportVsSnapshot !== undefined &&
				f.psnrExportVsSnapshot >= PSNR_EXPORT_MIN,
			`PSNR ${f.psnrExportVsSnapshot}`,
		);
	}
} catch (e) {
	verdict.checks.push({
		name: "exceção",
		ok: false,
		detail: String(e.stack ?? e),
	});
	console.error(e);
} finally {
	const allOk = verdict.checks.every((c) => c.ok);
	verdict.verdict = !allOk
		? "GPU_PARITY_FAIL"
		: verdict.softwareRenderer
			? "GPU_PARITY_PENDING_HUMAN_MACHINE"
			: "GPU_PARITY_PASS";
	verdict.note = verdict.softwareRenderer
		? "Renderer de SOFTWARE (SwiftShader/WARP/llvmpipe): os quadros foram comparados, mas isto NÃO é evidência de GPU real. Rode em PC Windows com GPU física (GPU_REAL=1)."
		: "Renderer com GPU física.";
	writeFileSync(
		join(OUT, "gpu-parity-verdict.json"),
		JSON.stringify(verdict, null, 2),
	);
	await browser.close();
	console.log(`VERDICT: ${verdict.verdict}`);
	process.exitCode = verdict.verdict === "GPU_PARITY_FAIL" ? 1 : 0;
}
