import { describe, expect, it } from "vitest";
import { TICKS_PER_SECOND } from "@capia/engine-bindings";
import { en } from "../i18n/en";
import { ptBR } from "../i18n/ptBR";
import { createTranslator, describeError } from "../i18n";
import {
  ACTION_IDS,
  DEFAULT_BINDINGS,
  buildLookup,
  eventToBinding,
  findConflicts,
  rebind,
  resetBindings,
  resolveBindings,
} from "./keymap";
import { DEFAULT_PREFS, loadPrefs, sanitizePrefs, savePrefs, type KeyValueStorage } from "./prefs";
import { formatTimecode, frameTicks, nominalFps, snapToFrame } from "./timecode";

const key = (
  k: string,
  m: Partial<{ ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }> = {},
) => ({
  key: k,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...m,
});

describe("keymap", () => {
  it("normalizes events to bindings and resolves them through the default preset", () => {
    const lookup = buildLookup(resolveBindings({}));
    expect(lookup.get(eventToBinding(key("b", { ctrlKey: true })) ?? "")).toBe("split");
    expect(lookup.get(eventToBinding(key("s")) ?? "")).toBe("split");
    expect(lookup.get(eventToBinding(key(" ")) ?? "")).toBe("playPause");
    expect(lookup.get(eventToBinding(key("Delete", { shiftKey: true })) ?? "")).toBe(
      "rippleDelete",
    );
    expect(lookup.get(eventToBinding(key("z", { ctrlKey: true, shiftKey: true })) ?? "")).toBe(
      "redo",
    );
    expect(lookup.get(eventToBinding(key("Z", { shiftKey: true })) ?? "")).toBe("zoomFit");
    // ⌘ conta como Ctrl; '+' (que exige Shift em alguns layouts) não precisa de Shift
    expect(eventToBinding(key("z", { metaKey: true }))).toBe("Ctrl+Z");
    expect(lookup.get(eventToBinding(key("+", { shiftKey: true })) ?? "")).toBe("zoomIn");
    expect(eventToBinding(key("Shift", { shiftKey: true }))).toBeNull();
  });

  it("ships a conflict-free default preset", () => {
    expect(findConflicts(resolveBindings({}))).toEqual([]);
    expect(ACTION_IDS.length).toBe(Object.keys(DEFAULT_BINDINGS).length);
  });

  it("detects conflicts on rebind, and resets one action or everything", () => {
    const { custom, conflicts } = rebind({}, "undo", ["Ctrl+Y"]);
    expect(conflicts).toEqual([{ binding: "Ctrl+Y", actions: ["undo", "redo"] }]);
    expect(resolveBindings(custom).undo).toEqual(["Ctrl+Y"]);
    expect(resolveBindings(resetBindings(custom, "undo")).undo).toEqual(["Ctrl+Z"]);
    expect(resetBindings(custom)).toEqual({});
    const ok = rebind({}, "split", ["X"]);
    expect(ok.conflicts).toEqual([]);
  });
});

describe("prefs", () => {
  it("falls back field by field on corrupt data and never throws", () => {
    const p = sanitizePrefs({
      language: "klingon",
      panels: { leftWidth: "wide", rightWidth: 99999, timelineHeight: 400 },
      keymap: { split: ["X"], undo: "nope", redo: [1] },
      preview: { quality: "8k", proxy: true },
      timeline: { pxPerSecond: -5, trackHeights: { v: 1000, a: "x" } },
    });
    expect(p.language).toBe(DEFAULT_PREFS.language);
    expect(p.panels.leftWidth).toBe(DEFAULT_PREFS.panels.leftWidth);
    expect(p.panels.rightWidth).toBe(640);
    expect(p.panels.timelineHeight).toBe(400);
    expect(p.keymap).toEqual({ split: ["X"] });
    expect(p.preview).toEqual({ quality: "auto", proxy: true, safeAreas: false, audio: true });
    expect(p.timeline.pxPerSecond).toBe(2);
    expect(p.timeline.trackHeights).toEqual({ v: 240 });
    expect(sanitizePrefs(null)).toEqual(DEFAULT_PREFS);
    expect(sanitizePrefs([1, 2])).toEqual(DEFAULT_PREFS);
  });

  it("round-trips through storage and recovers from invalid JSON or a throwing storage", () => {
    const mem = new Map<string, string>();
    const storage: KeyValueStorage = {
      getItem: (k) => mem.get(k) ?? null,
      setItem: (k, v) => void mem.set(k, v),
    };
    const prefs = { ...DEFAULT_PREFS, language: "pt-BR" as const };
    expect(savePrefs(storage, prefs)).toBe(true);
    expect(loadPrefs(storage)).toEqual({ prefs, recovered: false });
    mem.set("capia.prefs.v1", "{not json");
    expect(loadPrefs(storage)).toEqual({ prefs: DEFAULT_PREFS, recovered: true });
    const broken: KeyValueStorage = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("full");
      },
    };
    expect(loadPrefs(broken).recovered).toBe(true);
    expect(savePrefs(broken, prefs)).toBe(false);
    expect(loadPrefs(null).prefs).toEqual(DEFAULT_PREFS);
  });
});

describe("i18n", () => {
  it("has every English key translated and non-empty in both languages", () => {
    for (const k of Object.keys(en) as (keyof typeof en)[]) {
      expect(ptBR[k].length, k).toBeGreaterThan(0);
      // mesmos placeholders {x} nas duas línguas
      const ph = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      expect(ph(ptBR[k]), k).toEqual(ph(en[k]));
    }
  });

  it("interpolates parameters and maps API error codes", () => {
    const t = createTranslator("pt-BR");
    expect(t("project.usedBy", { count: 3 })).toBe("usada 3×");
    expect(describeError(t, "OVERLAP", "x")).toBe(ptBR["err.OVERLAP"]);
    expect(describeError(createTranslator("en"), "WEIRD_CODE", "raw message")).toBe("raw message");
    expect(describeError(createTranslator("en"), "NOT_FOUND", "clip c1")).toContain("clip c1");
  });
});

describe("timecode", () => {
  it("formats HH:MM:SS:FF for integer and fractional rates, and snaps to frames", () => {
    const f30 = frameTicks("30");
    expect(f30).toBe(23_520_000);
    expect(nominalFps("30000/1001")).toBe(30);
    expect(formatTimecode(0, "30")).toBe("00:00:00:00");
    expect(formatTimecode(TICKS_PER_SECOND * 61 + f30 * 5, "30")).toBe("00:01:01:05");
    expect(formatTimecode(TICKS_PER_SECOND * 3600, "30")).toBe("01:00:00:00");
    expect(snapToFrame(f30 * 3 + 100, f30)).toBe(f30 * 3);
    expect(snapToFrame(f30 * 3 + f30 / 2, f30)).toBe(f30 * 4);
  });
});

describe("recent projects (Home)", () => {
  it("promotes without duplicating, newest first, capped", async () => {
    const { pushRecent, MAX_RECENT } = await import("./prefs");
    let list = pushRecent([], "/a.capia", 1);
    list = pushRecent(list, "/b.capia", 2);
    list = pushRecent(list, "/a.capia", 3);
    expect(list.map((r) => r.path)).toEqual(["/a.capia", "/b.capia"]);
    for (let i = 0; i < 20; i++) list = pushRecent(list, `/p${String(i)}.capia`, 10 + i);
    expect(list).toHaveLength(MAX_RECENT);
    expect(list[0]?.path).toBe("/p19.capia");
    expect(pushRecent(list, "  ", 99)).toEqual(list);
  });

  it("sanitizes a corrupted recent list field by field", async () => {
    const { sanitizePrefs } = await import("./prefs");
    const p = sanitizePrefs({
      recent: [{ path: "/ok.capia", openedAt: 5 }, { path: 7 }, "x", { path: "", openedAt: 1 }],
    });
    expect(p.recent).toEqual([{ path: "/ok.capia", openedAt: 5 }]);
    expect(sanitizePrefs({ recent: "nope" }).recent).toEqual([]);
  });
});

describe("export stats", () => {
  it("computes percent, fps, ETA and realtime factor", async () => {
    const { exportStats, formatDuration, rateToFps } = await import("./exportStats");
    const s = exportStats(300, 900, 1_000, 11_000, 30); // 300 quadros em 10 s
    expect(s.percent).toBe(33);
    expect(s.fps).toBeCloseTo(30, 5);
    expect(s.etaMs).toBe(20_000);
    expect(s.realtime).toBeCloseTo(1, 5);
    expect(exportStats(0, 900, 1_000, 2_000, 30).etaMs).toBeNull();
    expect(exportStats(900, 900, 1_000, 2_000, null).etaMs).toBe(0);
    expect(exportStats(5, 0, undefined, 5, null).percent).toBe(0);
    expect(formatDuration(83_000)).toBe("1:23");
    expect(formatDuration(3_723_000)).toBe("1:02:03");
    expect(rateToFps("30000/1001")).toBeCloseTo(29.97, 2);
    expect(rateToFps("x")).toBeNull();
  });
});
