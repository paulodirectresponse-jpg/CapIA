import { test } from "bun:test";
test("headless domain perf (no UI)", async () => {
	const { EditorCore } = await import("@/core");
	const { buildDefaultScene } = await import("@/timeline/scenes");
	const editor = EditorCore.getInstance();
	const scene = buildDefaultScene({ name: "Main scene", isMain: true } as never);
	const project = { id: "p1", metadata: { id: "p1", name: "P", thumbnail: "", duration: 0, createdAt: new Date(), updatedAt: new Date() }, scenes: [scene], currentSceneId: scene.id, settings: { fps: { numerator: 30, denominator: 1 }, canvasSize: { width: 1280, height: 720 }, canvasSizeMode: "preset", lastCustomCanvasSize: null, originalCanvasSize: { width: 1280, height: 720 }, background: { type: "color", color: "#000000" } }, version: 31 } as never;
	editor.project.setActiveProject({ project });
	editor.scenes.initializeScenes({ scenes: [scene] });
	const T = 120000;
	const rows: unknown[] = [];
	for (const N of [100, 1000, 5000, 10000, 50000]) {
		const per = 50; const nT = Math.ceil(N / per);
		const mk = (t: number, c: number) => ({ id: `e-${t}-${c}`, type: "video", mediaId: "m", name: `c${t}-${c}`, duration: T / 2, startTime: Math.round(c * 0.6 * T), trimStart: 0, trimEnd: 0, sourceDuration: 20 * T, params: {} });
		const overlay = Array.from({ length: nT }, (_, t) => ({ id: `t-${t}`, name: `T${t}`, type: "video", muted: false, hidden: false, elements: Array.from({ length: Math.min(per, N - t * per) }, (_, c) => mk(t, c)) }));
		editor.timeline.updateTracks({ ...editor.scenes.getActiveScene().tracks, overlay } as never);
		editor.command.clear();
		const tr = editor.scenes.getActiveScene().tracks.overlay[0];
		const m = (f: () => void) => { const t0 = performance.now(); f(); return +(performance.now() - t0).toFixed(2); };
		const split = m(() => editor.timeline.splitElements({ elements: [{ trackId: tr.id, elementId: tr.elements[0].id }], splitTime: Math.round(0.25 * T) as never }));
		const move = m(() => editor.timeline.moveElements({ moves: [{ sourceTrackId: tr.id, targetTrackId: tr.id, elementId: tr.elements[1].id, newStartTime: Math.round(900 * T) as never }] }));
		const del = m(() => editor.timeline.deleteElements({ elements: [{ trackId: tr.id, elementId: tr.elements[3].id }] }));
		const undo = m(() => editor.command.undo());
		const ser = m(() => { JSON.stringify(editor.scenes.getScenes()); });
		rows.push({ N, split, move, del, undo, serializeMs: ser });
	}
	console.log(JSON.stringify(rows));
});
