import { describe, expect, it } from "vitest";
import { TICKS_PER_SECOND, type Clip, type Track } from "@capia/engine-bindings";
import { buildData, clipKind, clipOffline } from "./data";
import {
  buildRows,
  clampPps,
  clipAt,
  displayOrder,
  fitPps,
  pxToTicks,
  rowAt,
  rulerStepSeconds,
  ticksToPx,
  visibleClips,
} from "./layout";

const track = (id: string, kind: "visual" | "audio", role: Track["role"] = "overlay"): Track => ({
  id,
  name: id,
  kind,
  role,
  magnetic: false,
  locked: false,
  hidden: false,
  muted: false,
  solo: false,
  sync_lock: true,
  group: null,
});

const clip = (id: string, trackId: string, start: number, dur: number): Clip => ({
  id,
  track: trackId,
  start,
  duration: dur,
  name: id,
  enabled: true,
  content: { type: "solid", color: "#fff" },
  source_in: 0,
  speed: "1",
  reversed: false,
  properties: {},
});

describe("time ↔ pixel", () => {
  it("round-trips and honours the origin and zoom", () => {
    const origin = 2 * TICKS_PER_SECOND;
    expect(ticksToPx(origin, 100, origin)).toBe(0);
    expect(ticksToPx(origin + TICKS_PER_SECOND, 100, origin)).toBe(100);
    expect(pxToTicks(250, 100, origin)).toBe(origin + 2.5 * TICKS_PER_SECOND);
    for (const x of [0, 13.7, 999]) {
      expect(Math.abs(ticksToPx(pxToTicks(x, 80, origin), 80, origin) - x)).toBeLessThan(1e-3);
    }
    expect(clampPps(0.1)).toBe(2);
    expect(clampPps(1e9)).toBe(4000);
  });

  it("picks ruler steps that keep labels apart", () => {
    expect(rulerStepSeconds(100)).toBe(1);
    expect(rulerStepSeconds(10)).toBe(10);
    expect(rulerStepSeconds(4000)).toBeLessThan(0.1);
    for (const pps of [3, 25, 80, 400, 3000])
      expect(rulerStepSeconds(pps) * pps).toBeGreaterThanOrEqual(90);
  });
});

describe("rows", () => {
  const tracks = [
    track("v1", "visual", "main"),
    track("a1", "audio", "music"),
    track("v2", "visual"),
    track("a2", "audio", "sfx"),
  ];

  it("shows visual tracks top→bottom of the compositing stack, then audio in order", () => {
    expect(displayOrder(tracks).map((t) => t.id)).toEqual(["v2", "v1", "a1", "a2"]);
    const { rows, totalHeight } = buildRows(tracks, { v1: 100 });
    expect(rows.map((r) => [r.track.id, r.y, r.h])).toEqual([
      ["v2", 0, 56],
      ["v1", 56, 100],
      ["a1", 156, 44],
      ["a2", 200, 44],
    ]);
    expect(totalHeight).toBe(244);
    expect(buildRows(tracks, { v1: 5000 }).rows[1]?.h).toBe(240);
    expect(rowAt(rows, 0)?.track.id).toBe("v2");
    expect(rowAt(rows, 155.9)?.track.id).toBe("v1");
    expect(rowAt(rows, 156)?.track.id).toBe("a1");
    expect(rowAt(rows, 244)).toBeNull();
    expect(rowAt(rows, -1)).toBeNull();
  });
});

describe("virtualization helpers", () => {
  const list = Array.from({ length: 1000 }, (_, i) => clip(`c${String(i)}`, "v", i * 100, 80));

  it("finds the visible window and the clip under a time with binary search", () => {
    expect(visibleClips(list, 250, 450).map((c) => c.id)).toEqual(["c2", "c3", "c4"]);
    expect(visibleClips(list, 80, 100)).toEqual([]);
    expect(visibleClips(list, 1e9, 2e9)).toEqual([]);
    expect(clipAt(list, 305)?.id).toBe("c3");
    expect(clipAt(list, 385)).toBeNull();
    expect(clipAt(list, 99_990)?.id).toBeUndefined();
  });

  it("indexes a 5000-clip sequence quickly and keeps tracks sorted", () => {
    const clips: Record<string, Clip> = {};
    for (let i = 0; i < 5000; i++) {
      const c = clip(`k${String(i)}`, i % 2 === 0 ? "v" : "w", (4999 - i) * 100, 80);
      clips[c.id] = c;
    }
    const seq = {
      header: { name: "s", frame_rate: "30", sample_rate: 48000 },
      tracks: [track("v", "visual"), track("w", "visual")],
      clips,
      markers: {},
      frame_ticks: 23_520_000,
    };
    const t0 = performance.now();
    const d = buildData(seq, {}, {});
    const ms = performance.now() - t0;
    expect(d.clipCount).toBe(5000);
    expect(
      d.byTrack.get("v")?.every((c, i, a) => i === 0 || (a[i - 1]?.start ?? 0) <= c.start),
    ).toBe(true);
    expect(ms).toBeLessThan(200);
  });

  it("fits the sequence into the viewport", () => {
    expect(fitPps(10 * TICKS_PER_SECOND, 960)).toBeCloseTo(92.16, 2);
    expect(fitPps(0, 960)).toBe(80);
  });
});

describe("clip classification", () => {
  it("classifies content by track and flags offline media", () => {
    const v = track("v", "visual");
    const a = track("a", "audio");
    const caps = track("c", "visual", "captions");
    const media = (has_video: boolean): Clip => ({
      ...clip("m", "v", 0, 1),
      content: { type: "media", asset: "x", has_video, has_audio: true },
    });
    expect(clipKind(media(true), v)).toBe("video");
    expect(clipKind(media(false), a)).toBe("audio");
    expect(clipKind({ ...clip("t", "c", 0, 1), content: { type: "text", text: "x" } }, caps)).toBe(
      "caption",
    );
    expect(clipKind({ ...clip("t", "v", 0, 1), content: { type: "text", text: "x" } }, v)).toBe(
      "text",
    );
    const assets = {
      x: { id: "x", status: "offline" } as never,
    };
    expect(clipOffline(media(true), assets)).toBe(true);
    expect(clipOffline(clip("s", "v", 0, 1), assets)).toBe(false);
  });
});
