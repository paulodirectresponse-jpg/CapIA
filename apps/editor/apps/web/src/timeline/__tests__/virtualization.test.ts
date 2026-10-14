import { describe, expect, test } from "bun:test";
import {
	WINDOW_CHUNK_PX,
	WINDOW_OVERSCAN_PX,
	computeVisibleRowWindow,
	computeVisibleTimeWindow,
	isInWindow,
	selectVisibleElements,
	selectVisibleRows,
} from "@/timeline/virtualization";
import { TICKS_PER_SECOND } from "@/wasm";

const T = TICKS_PER_SECOND;

function clip({
	id,
	startSeconds,
	durationSeconds = 1,
}: {
	id: string;
	startSeconds: number;
	durationSeconds?: number;
}) {
	return { id, startTime: startSeconds * T, duration: durationSeconds * T };
}

describe("timeline time-window virtualization", () => {
	test("window covers the viewport plus overscan and is chunk-quantized", () => {
		// zoom 1 = 50 px/s; viewport 1000 px at scroll 0 -> [0, 1000+overscan] px
		const w = computeVisibleTimeWindow({
			scrollLeft: 0,
			viewportWidth: 1000,
			zoomLevel: 1,
		});
		expect(w.start).toBe(0);
		const expectedEndPx =
			Math.ceil(1000 / WINDOW_CHUNK_PX) * WINDOW_CHUNK_PX + WINDOW_OVERSCAN_PX;
		expect(w.end).toBe(Math.ceil((expectedEndPx / 50) * T));
		// scrolling within one chunk does not change the window (stable identity for memo)
		const a = computeVisibleTimeWindow({
			scrollLeft: 1000,
			viewportWidth: 1000,
			zoomLevel: 1,
		});
		const b = computeVisibleTimeWindow({
			scrollLeft: 800 + WINDOW_CHUNK_PX - 1,
			viewportWidth: 1000,
			zoomLevel: 1,
		});
		expect(b.start).toBe(a.start);
	});

	test("zooming in shrinks the window in time", () => {
		const wide = computeVisibleTimeWindow({
			scrollLeft: 0,
			viewportWidth: 1000,
			zoomLevel: 1,
		});
		const tight = computeVisibleTimeWindow({
			scrollLeft: 0,
			viewportWidth: 1000,
			zoomLevel: 8,
		});
		expect(tight.end).toBeLessThan(wide.end / 4);
	});

	test("intersection is inclusive at both edges", () => {
		const window = { start: 10 * T, end: 20 * T };
		expect(isInWindow({ ...clip({ id: "a", startSeconds: 9 }), window })).toBe(
			true,
		); // ends exactly at start
		expect(isInWindow({ ...clip({ id: "b", startSeconds: 20 }), window })).toBe(
			true,
		); // starts exactly at end
		expect(isInWindow({ ...clip({ id: "c", startSeconds: 8 }), window })).toBe(
			false,
		);
		expect(isInWindow({ ...clip({ id: "d", startSeconds: 21 }), window })).toBe(
			false,
		);
	});

	test("selectVisibleElements keeps pinned ids (selected/dragged) outside the window", () => {
		const elements = [
			clip({ id: "in", startSeconds: 12 }),
			clip({ id: "out", startSeconds: 100 }),
			clip({ id: "pinned", startSeconds: 200 }),
		];
		const picked = selectVisibleElements({
			elements,
			window: { start: 10 * T, end: 20 * T },
			keep: (id) => id === "pinned",
		});
		expect(picked.map((e) => e.id)).toEqual(["in", "pinned"]);
		expect(
			selectVisibleElements({ elements, window: null, keep: () => false }),
		).toBe(elements);
	});

	test("10 000 clips: only a small bounded set is selected for a typical viewport", () => {
		const elements = Array.from({ length: 10_000 }, (_, i) =>
			clip({ id: `c${i}`, startSeconds: i * 0.6, durationSeconds: 0.5 }),
		);
		const w = computeVisibleTimeWindow({
			scrollLeft: 5000,
			viewportWidth: 1500,
			zoomLevel: 4,
		});
		const picked = selectVisibleElements({
			elements,
			window: w,
			keep: () => false,
		});
		expect(picked.length).toBeGreaterThan(0);
		expect(picked.length).toBeLessThan(100);
	});
});

describe("virtualização vertical de faixas", () => {
	const rows = Array.from({ length: 100 }, (_, i) => ({
		id: `t${i}`,
		top: i * 40,
		height: 40,
	}));

	test("janela quantizada com overscan", () => {
		const w = computeVisibleRowWindow({ scrollTop: 130, viewportHeight: 200 });
		expect(w.top).toBe(0); // floor(130/120)*120 - 300 < 0
		expect(w.bottom).toBe(Math.ceil(330 / 120) * 120 + 300);
	});

	test("no topo monta só ~ viewport+overscan", () => {
		const w = computeVisibleRowWindow({ scrollTop: 0, viewportHeight: 400 });
		const out = selectVisibleRows({ rows, window: w, keep: () => false });
		expect(out.length).toBeLessThan(25);
		expect(out[0].id).toBe("t0");
	});

	test("no meio não monta as faixas distantes", () => {
		const w = computeVisibleRowWindow({ scrollTop: 2000, viewportHeight: 400 });
		const ids = selectVisibleRows({ rows, window: w, keep: () => false }).map(
			(r) => r.id,
		);
		expect(ids).not.toContain("t0");
		expect(ids).not.toContain("t99");
		expect(ids).toContain("t50");
	});

	test("faixas 'keep' (selecionada/arrasto) sempre montadas", () => {
		const w = computeVisibleRowWindow({ scrollTop: 2000, viewportHeight: 400 });
		const ids = selectVisibleRows({
			rows,
			window: w,
			keep: (id) => id === "t0",
		}).map((r) => r.id);
		expect(ids).toContain("t0");
	});

	test("window null = tudo (virtualização desligada)", () => {
		expect(
			selectVisibleRows({ rows, window: null, keep: () => false }).length,
		).toBe(100);
	});

	test("faixa parcialmente visível na borda é montada", () => {
		const out = selectVisibleRows({
			rows: [{ id: "x", top: 1000, height: 40 }],
			window: { top: 1039, bottom: 2000 },
			keep: () => false,
		});
		expect(out.length).toBe(1);
	});
});
