/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- fakes for the DOM-bound controller deps and partial track fixtures */
import { describe, expect, test } from "bun:test";
import {
	ElementInteractionController,
	type ElementInteractionDeps,
} from "@/timeline/controllers/element-interaction-controller";
import { snapGroupEdges } from "@/timeline/group-move/snap";
import type { MoveGroup } from "@/timeline/group-move/types";
import { resolveTimelineSnap } from "@/timeline/snapping/resolve";
import { getTimelineSnapThresholdInTicks } from "@/timeline/snapping/threshold";
import type { SceneTracks } from "@/timeline";
import { mediaTime, ZERO_MEDIA_TIME, TICKS_PER_SECOND } from "@/wasm";

// Characterization tests: they pin the CURRENT snapping behavior (no redesign).
const sec = (s: number) =>
	mediaTime({ ticks: Math.round(s * TICKS_PER_SECOND) });
const ZOOM = 1; // 50 px/s -> 10 px threshold = 0.2 s = 24 000 ticks

function element({
	id,
	start,
	duration,
}: {
	id: string;
	start: number;
	duration: number;
}) {
	return {
		id,
		type: "video" as const,
		name: id,
		mediaId: "m",
		startTime: sec(start),
		duration: sec(duration),
		trimStart: ZERO_MEDIA_TIME,
		trimEnd: ZERO_MEDIA_TIME,
		params: {},
	};
}

function tracksWith(elements: ReturnType<typeof element>[]): SceneTracks {
	return {
		overlay: [],
		main: {
			id: "main",
			name: "Main",
			type: "video",
			muted: false,
			hidden: false,
			elements,
		},
		audio: [],
	} as SceneTracks;
}

function groupOf({ duration }: { duration: number }): MoveGroup {
	const anchor = {
		trackId: "main",
		elementId: "moving",
		elementType: "video",
		duration: sec(duration),
		timeOffset: ZERO_MEDIA_TIME,
		trackSection: "main",
		sectionIndex: 0,
		displayIndex: 0,
	} as const;
	return { anchor, members: [anchor] } as MoveGroup;
}

function snap({
	anchor,
	playhead,
	elements,
	duration = 1,
}: {
	anchor: number;
	playhead: number;
	elements: ReturnType<typeof element>[];
	duration?: number;
}) {
	return snapGroupEdges({
		group: groupOf({ duration }),
		anchorStartTime: sec(anchor),
		tracks: tracksWith([
			element({ id: "moving", start: 0, duration }),
			...elements,
		]),
		playheadTime: sec(playhead),
		zoomLevel: ZOOM,
	});
}

describe("timeline snapping (characterization)", () => {
	test("threshold is 10 px expressed in ticks and shrinks with zoom", () => {
		expect(getTimelineSnapThresholdInTicks({ zoomLevel: 1 })).toBe(24_000);
		expect(getTimelineSnapThresholdInTicks({ zoomLevel: 2 })).toBe(12_000);
	});

	test("clip start -> playhead (inside threshold)", () => {
		const result = snap({ anchor: 3.1, playhead: 3.0, elements: [] });
		expect(result.snappedAnchorStartTime).toBe(sec(3.0));
		expect(result.snapPoint?.type).toBe("playhead");
	});

	test("outside the threshold nothing snaps", () => {
		const result = snap({ anchor: 3.5, playhead: 3.0, elements: [] });
		expect(result.snappedAnchorStartTime).toBe(sec(3.5));
		expect(result.snapPoint).toBeNull();
	});

	test("clip start -> other clip end (edge to edge)", () => {
		const other = element({ id: "other", start: 2, duration: 3 }); // ends at 5
		const result = snap({ anchor: 5.1, playhead: 100, elements: [other] });
		expect(result.snappedAnchorStartTime).toBe(sec(5.0));
		expect(result.snapPoint?.type).toBe("element-end");
		expect(result.snapPoint?.elementId).toBe("other");
	});

	test("clip END -> other clip start (the moving clip's trailing edge snaps too)", () => {
		const other = element({ id: "other", start: 10, duration: 2 });
		// moving duration 2 s: end = anchor+2 = 9.9 -> snaps to 10 => anchor 8.0
		const result = snap({
			anchor: 7.9,
			playhead: 100,
			elements: [other],
			duration: 2,
		});
		expect(result.snappedAnchorStartTime).toBe(sec(8.0));
		expect(result.snapPoint?.type).toBe("element-start");
	});

	test("tie between two equidistant points: the first built source wins (clip edges before playhead)", () => {
		const other = element({ id: "other", start: 3.2, duration: 1 }); // start at 3.2
		// anchor 3.1 is exactly 0.1 s from playhead 3.0 and from edge 3.2
		const result = snap({ anchor: 3.1, playhead: 3.0, elements: [other] });
		expect(result.snapPoint?.type).toBe("element-start");
		expect(result.snappedAnchorStartTime).toBe(sec(3.2));
	});

	test("nearest point beats an earlier-listed farther one", () => {
		const farther = element({ id: "far", start: 3.3, duration: 1 });
		const result = snap({ anchor: 3.04, playhead: 3.0, elements: [farther] });
		expect(result.snapPoint?.type).toBe("playhead");
		expect(result.snappedAnchorStartTime).toBe(sec(3.0));
	});

	test("resolveTimelineSnap: distance exactly at the threshold still snaps", () => {
		const result = resolveTimelineSnap({
			targetTime: sec(1.2),
			snapPoints: [{ time: sec(1.0), type: "playhead" }],
			maxSnapDistance: 24_000,
		});
		expect(result.snapPoint?.type).toBe("playhead");
	});

	describe("controller seam: snapping disabled / Shift held", () => {
		function resultFor({
			enabled,
			shift,
		}: {
			enabled: boolean;
			shift: boolean;
		}) {
			const tracks = tracksWith([
				element({ id: "moving", start: 0, duration: 1 }),
			]);
			const deps = {
				snap: { isEnabled: () => enabled },
				input: { isShiftHeld: () => shift },
				scene: { getTracks: () => tracks },
				viewport: { getZoomLevel: () => ZOOM },
				playback: { getCurrentTime: () => sec(3.0) },
			} as unknown as ElementInteractionDeps;
			const controller = new ElementInteractionController({
				depsRef: { current: deps },
			});
			const snapResult = Reflect.get(controller, "snapResult") as (args: {
				frameSnappedTime: ReturnType<typeof sec>;
				group: MoveGroup;
			}) => { snappedTime: ReturnType<typeof sec>; snapPoint: unknown };
			return snapResult.call(controller, {
				frameSnappedTime: sec(3.1),
				group: groupOf({ duration: 1 }),
			});
		}

		test("enabled: snaps to the playhead", () => {
			expect(resultFor({ enabled: true, shift: false }).snappedTime).toBe(
				sec(3.0),
			);
		});
		test("disabled: keeps the frame-snapped time", () => {
			const result = resultFor({ enabled: false, shift: false });
			expect(result.snappedTime).toBe(sec(3.1));
			expect(result.snapPoint).toBeNull();
		});
		test("Shift held overrides snapping", () => {
			expect(resultFor({ enabled: true, shift: true }).snappedTime).toBe(
				sec(3.1),
			);
		});
	});
});
