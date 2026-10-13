// Benchmark de UI da timeline: com e sem janelamento temporal (mesmo build, flag de runtime E2E).
// Uso: EDITOR_URL=... NS=100,1000,5000,10000 node e2e/perf-virtualization.mjs > perf-virtualization.json
import { importFiles, launch, makeMedia, openNewProject } from "./harness.mjs";

const BASE = process.env.EDITOR_URL ?? "http://127.0.0.1:3100";
const NS = (process.env.NS ?? "100,1000,5000,10000").split(",").map(Number);
const LAYOUTS = (process.env.LAYOUTS ?? "dense,sequential").split(",");
const MODES = (process.env.MODES ?? "baseline,virtualized").split(",");
const media = makeMedia();
const out = [];

for (const N of NS) {
	for (const layout of LAYOUTS) {
		for (const mode of MODES) {
			const { browser, page } = await launch();
			if (mode === "baseline")
				await page.addInitScript(() => {
					window.__CAPIA_NO_VIRTUALIZATION__ = true;
				});
			const row = { N, layout, mode };
			try {
				await openNewProject(page, BASE);
				await importFiles(page, [media.main]);
				await page.waitForFunction(
					() => window.__capiaEditor !== undefined,
					null,
					{ timeout: 15000 },
				);
				Object.assign(
					row,
					await page.evaluate(
						async ({ N, layout }) => {
							const ed = window.__capiaEditor;
							const mediaId = ed.media.getAssets()[0].id;
							const sc = ed.scenes.getActiveScene();
							const T = 120000;
							// dense = 50 clips/track, todas começam em t=0 (pior caso p/ janelamento temporal);
							// sequential = 10 tracks com clips em sequência no tempo (projeto realista)
							const per = layout === "dense" ? 50 : Math.ceil(N / 10);
							const nTracks = Math.max(1, Math.ceil(N / per));
							const mk = (t, c) => ({
								id: `e-${t}-${c}`,
								type: "video",
								mediaId,
								name: `c${t}-${c}`,
								duration: Math.round(0.5 * T),
								startTime: Math.round(c * 0.6 * T),
								trimStart: 0,
								trimEnd: 0,
								sourceDuration: 12 * T,
								params: {
									"transform.positionX": 0,
									"transform.positionY": 0,
									"transform.scaleX": 1,
									"transform.scaleY": 1,
									"transform.rotate": 0,
									opacity: 1,
									blendMode: "normal",
									volume: 0,
									muted: false,
								},
								isSourceAudioEnabled: true,
								hidden: false,
							});
							const overlay = [];
							let left = N;
							for (let t = 0; t < nTracks; t++) {
								const n = Math.min(per, left);
								left -= n;
								overlay.push({
									id: `t-${t}`,
									name: `T${t}`,
									type: "video",
									muted: false,
									hidden: false,
									elements: Array.from({ length: n }, (_, i) => mk(t, i)),
								});
							}
							const lt = [];
							try {
								new PerformanceObserver((l) =>
									l.getEntries().forEach((e) => lt.push(e.duration)),
								).observe({ entryTypes: ["longtask"] });
							} catch {}
							const t0 = performance.now();
							ed.timeline.updateTracks({ ...sc.tracks, overlay });
							await new Promise((r) =>
								requestAnimationFrame(() => requestAnimationFrame(r)),
							);
							await new Promise((r) => setTimeout(r, 500));
							return {
								loadMs: Math.round(performance.now() - t0),
								dom: document.querySelectorAll("*").length,
								heapMB: performance.memory
									? Math.round(performance.memory.usedJSHeapSize / 1e6)
									: null,
								longTaskMaxMs: Math.round(Math.max(0, ...lt)),
								nTracks,
							};
						},
						{ N, layout },
					),
				);
				const frames = () =>
					page.evaluate(
						() =>
							new Promise((res) => {
								const ds = [];
								let last = performance.now(),
									n = 0;
								const f = (t) => {
									ds.push(t - last);
									last = t;
									if (++n < 60) requestAnimationFrame(f);
									else {
										ds.shift();
										ds.sort((a, b) => a - b);
										res({
											p50: Math.round(ds[Math.floor(ds.length / 2)]),
											p95: Math.round(ds[Math.floor(ds.length * 0.95)]),
											max: Math.round(ds[ds.length - 1]),
										});
									}
								};
								requestAnimationFrame(f);
							}),
					);
				const scrolling = frames();
				for (let i = 0; i < 14; i++) {
					await page.mouse.move(800, 700);
					await page.mouse.wheel(400, 0);
					await page.waitForTimeout(40);
				}
				row.scrollFrame = await scrolling;
				let z = Date.now();
				await page.mouse.click(1517, 520);
				await page.evaluate(
					() =>
						new Promise((r) =>
							requestAnimationFrame(() => requestAnimationFrame(r)),
						),
				);
				row.zoomOutMs = Date.now() - z;
				row.cmd = await page.evaluate(async () => {
					const ed = window.__capiaEditor;
					const tr = ed.scenes.getActiveScene().tracks.overlay[0];
					const T = 120000;
					const m = async (f) => {
						const t0 = performance.now();
						f();
						await new Promise((r) => requestAnimationFrame(r));
						return Math.round((performance.now() - t0) * 10) / 10;
					};
					const split = await m(() =>
						ed.timeline.splitElements({
							elements: [{ trackId: tr.id, elementId: tr.elements[0].id }],
							splitTime: Math.round(0.25 * T),
						}),
					);
					const move = await m(() =>
						ed.timeline.moveElements({
							moves: [
								{
									sourceTrackId: tr.id,
									targetTrackId: tr.id,
									elementId: tr.elements[1].id,
									newStartTime: Math.round(500 * T),
								},
							],
						}),
					);
					const trim = await m(() =>
						ed.timeline.updateElementTrim({
							elementId: tr.elements[2].id,
							trimStart: Math.round(0.1 * T),
							trimEnd: 0,
							startTime: Math.round(tr.elements[2].startTime + 0.1 * T),
							duration: Math.round(0.4 * T),
						}),
					);
					const undo = await m(() => ed.command.undo());
					return { split, move, trim, undo };
				});
				row.domAfterInteractions = await page.evaluate(
					() => document.querySelectorAll("*").length,
				);
			} catch (e) {
				row.error = String(e.message ?? e).slice(0, 200);
			}
			console.error(JSON.stringify(row));
			out.push(row);
			await browser.close();
		}
	}
}
console.log(JSON.stringify(out, null, 1));
