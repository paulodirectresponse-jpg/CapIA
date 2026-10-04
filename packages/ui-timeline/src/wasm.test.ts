import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Clip, SequenceModel, Track } from "@capia/engine-bindings";
import { CoreError, TimelineCore } from "./wasm";

const F = 23_520_000;
const wasmPath = fileURLToPath(new URL("./generated/capia_timeline.wasm", import.meta.url));

const track = (id: string, kind: "visual" | "audio"): Track => ({
  id,
  name: "",
  kind,
  role: kind === "visual" ? "overlay" : "music",
  magnetic: false,
  locked: false,
  hidden: false,
  muted: false,
  solo: false,
  sync_lock: true,
  group: null,
});

const clip = (id: string, start: number, frames: number): Clip => ({
  id,
  track: "v",
  start: start * F,
  duration: frames * F,
  name: "",
  enabled: true,
  content: { type: "solid", color: "#fff" },
  source_in: 0,
  speed: "1",
  reversed: false,
  properties: {},
});

const model = (): SequenceModel => ({
  header: { name: "s", frame_rate: "30", sample_rate: 48000 },
  tracks: [track("v", "visual"), track("a", "audio")],
  clips: { c1: clip("c1", 0, 10), c2: clip("c2", 30, 10) },
  markers: {},
  frame_ticks: F,
});

async function core(): Promise<TimelineCore> {
  const c = await TimelineCore.load(readFileSync(wasmPath));
  c.loadSequence("s", model());
  return c;
}

describe("TimelineCore (real WASM build of the engine's UX functions)", () => {
  it("computes the snap threshold exactly like the engine", async () => {
    const c = await core();
    expect(c.threshold(10, 50)).toBe(141_120_000);
    expect(c.threshold(8, 80.5)).toBeGreaterThan(0);
  });

  it("snaps a dragged clip to neighbours and respects the enabled flag", async () => {
    const c = await core();
    const th = 2 * F;
    const on = c.snapClip("c1", 19 * F, th, null, true);
    expect(on.start).toBe(20 * F);
    expect(on.snapped_to?.type).toBe("clip_start");
    expect(c.snapClip("c1", 19 * F, th, null, false)).toEqual({ start: 19 * F, snapped_to: null });
  });

  it("stays in sync when the engine's patches are applied, and rejects divergent ones", async () => {
    const c = await core();
    const moved: Clip = { ...clip("c1", 5, 10) };
    c.applyPatches([{ op: "clip", sequence: "s", id: "c1", old: clip("c1", 0, 10), new: moved }]);
    expect(c.snapPoint(5 * F + 1, 100, ["c2"], null)?.t).toBe(5 * F);
    expect(() =>
      c.applyPatches([{ op: "clip", sequence: "s", id: "c1", old: clip("c1", 0, 10), new: null }]),
    ).toThrowError(CoreError);
    try {
      c.applyPatches([{ op: "clip", sequence: "s", id: "c1", old: clip("c1", 0, 10), new: null }]);
    } catch (e) {
      expect((e as CoreError).code).toBe("PATCH_MISMATCH");
    }
  });

  it("resolves group moves, placement and frame alignment", async () => {
    const c = await core();
    expect(c.groupMove(["c1"], -3 * F, 0, null).delta_time).toBe(0);
    expect(c.placement({ type: "first_available" }, "visual", [[0, 10 * F]]).kind).toBe(
      "new_track",
    );
    expect(c.placement({ type: "first_available" }, "visual", [[12 * F, 5 * F]]).kind).toBe(
      "existing",
    );
    expect(c.alignToFrame(3 * F + F / 2)).toBe(4 * F);
  });

  it("reports structured errors (not loaded / invalid input) instead of throwing opaque ones", async () => {
    const empty = await TimelineCore.load(readFileSync(wasmPath));
    expect(() => empty.snapPoint(0, 1, [], null)).toThrowError(/no sequence is loaded/);
    const c = await core();
    expect(() => c.groupMove([], 0, 0, null)).toThrowError(CoreError);
  });
});
