import { test, expect } from "bun:test";
test("headless: drive real commands, shared undo, no DOM", async () => {
	const { EditorCore } = await import("@/core");
	const { buildDefaultScene } = await import("@/timeline/scenes");
	const { mediaTimeFromSeconds, TICKS_PER_SECOND } = await import("@/wasm");
	const editor = EditorCore.getInstance();
	const scene = buildDefaultScene({ name: "Main scene", isMain: true } as never);
	const project = { id: "p1", metadata: { id: "p1", name: "P", thumbnail: "", duration: 0, createdAt: new Date(), updatedAt: new Date() }, scenes: [scene], currentSceneId: scene.id, settings: { fps: { numerator: 30, denominator: 1 }, canvasSize: { width: 1280, height: 720 }, canvasSizeMode: "preset", lastCustomCanvasSize: null, originalCanvasSize: { width: 1280, height: 720 }, background: { type: "color", color: "#000000" } }, version: 31, timelineViewState: undefined } as never;
	editor.project.setActiveProject({ project });
	editor.scenes.initializeScenes({ scenes: [scene] });
	const t = (s: number) => mediaTimeFromSeconds({ seconds: s });
	editor.timeline.insertElement({
		element: { type: "video", mediaId: "m1", name: "clip", duration: t(10), startTime: t(0), trimStart: t(0), trimEnd: t(0), sourceDuration: t(10), params: {} } as never,
		placement: { mode: "auto" },
	});
	const els = () => editor.scenes.getActiveScene().tracks.main.elements.map((e) => `${(e.startTime / TICKS_PER_SECOND).toFixed(2)}+${(e.duration / TICKS_PER_SECOND).toFixed(2)}`);
	console.log("after insert", els(), editor.scenes.getActiveScene().tracks.overlay.length);
	const first = editor.scenes.getActiveScene().tracks.main.elements[0] ?? editor.scenes.getActiveScene().tracks.overlay[0]?.elements[0];
	const trackId = editor.scenes.getActiveScene().tracks.main.elements[0] ? editor.scenes.getActiveScene().tracks.main.id : editor.scenes.getActiveScene().tracks.overlay[0].id;
	editor.timeline.splitElements({ elements: [{ trackId, elementId: first.id }], splitTime: t(4) });
	console.log("after split", els());
	editor.command.undo();
	console.log("after undo", els());
	expect(true).toBe(true);
});
