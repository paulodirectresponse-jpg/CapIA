import { describe, expect, it } from "vitest";
import type { ChangeSet, Clip, ProjectSnapshot, SequenceModel, Track } from "./editor";
import { applyChange, fromSnapshot, withSequenceModel } from "./readmodel";

const track = (id: string): Track => ({
  id,
  name: id,
  kind: "visual",
  role: "overlay",
  magnetic: false,
  locked: false,
  hidden: false,
  muted: false,
  solo: false,
  sync_lock: true,
  group: null,
});

const clip = (id: string, start = 0): Clip => ({
  id,
  track: "v",
  start,
  duration: 10,
  name: id,
  enabled: true,
  content: { type: "solid", color: "#fff" },
  source_in: 0,
  speed: "1",
  reversed: false,
  properties: {},
});

const snap: ProjectSnapshot = {
  revision: 1,
  can_undo: false,
  can_redo: false,
  project: { path: "/p.capia", name: "p.capia" },
  sequences: [],
  folders: [],
  deliverables: [],
  assets: [],
};

const empty = (): SequenceModel => ({
  header: { name: "s", frame_rate: "30", sample_rate: 48000 },
  tracks: [track("v")],
  clips: {},
  markers: {},
  frame_ticks: 23_520_000,
});

describe("applyChange", () => {
  const base = withSequenceModel(fromSnapshot(snap), "s", empty());

  it("inserts, replaces and removes clips by patch, sharing untouched structure", () => {
    const ins: ChangeSet = {
      revision: 2,
      can_undo: true,
      can_redo: false,
      patches: [{ op: "clip", sequence: "s", id: "c1", old: null, new: clip("c1") }],
    };
    const m1 = applyChange(base, ins);
    expect(Object.keys(m1.models.s?.clips ?? {})).toEqual(["c1"]);
    expect(m1.revision).toBe(2);
    expect(m1.canUndo).toBe(true);
    expect(m1.models.s?.tracks).toBe(base.models.s?.tracks);
    // o objeto antigo não é mutado
    expect(Object.keys(base.models.s?.clips ?? {})).toEqual([]);

    const moved = applyChange(m1, {
      revision: 3,
      can_undo: true,
      can_redo: false,
      patches: [{ op: "clip", sequence: "s", id: "c1", old: clip("c1"), new: clip("c1", 5) }],
    });
    expect(moved.models.s?.clips.c1?.start).toBe(5);

    const gone = applyChange(moved, {
      revision: 4,
      can_undo: true,
      can_redo: false,
      patches: [{ op: "clip", sequence: "s", id: "c1", old: clip("c1", 5), new: null }],
    });
    expect(gone.models.s?.clips).toEqual({});
  });

  it("applies track insertion at an index and removal", () => {
    const m = applyChange(base, {
      revision: 2,
      can_undo: true,
      can_redo: false,
      patches: [
        { op: "track", sequence: "s", id: "t2", old: null, new: { index: 0, track: track("t2") } },
      ],
    });
    expect(m.models.s?.tracks.map((t) => t.id)).toEqual(["t2", "v"]);
    const r = applyChange(m, {
      revision: 3,
      can_undo: true,
      can_redo: false,
      patches: [
        { op: "track", sequence: "s", id: "t2", old: { index: 0, track: track("t2") }, new: null },
      ],
    });
    expect(r.models.s?.tracks.map((t) => t.id)).toEqual(["v"]);
  });

  it("ignores clip patches of sequences that are not loaded but updates summaries and flags", () => {
    const m = applyChange(fromSnapshot(snap), {
      revision: 9,
      can_undo: true,
      can_redo: true,
      patches: [{ op: "clip", sequence: "other", id: "c", old: null, new: clip("c") }],
      sequence_summaries: [
        {
          id: "other",
          name: "Other",
          frame_rate: "30",
          frame_ticks: 23_520_000,
          sample_rate: 48000,
          width: 1080,
          height: 1920,
          folder: null,
          duration: 10,
          clip_count: 1,
          nested_usage: 0,
        },
      ],
    });
    expect(m.models.other).toBeUndefined();
    expect(m.sequences.other?.clip_count).toBe(1);
    expect(m.canRedo).toBe(true);
  });

  it("removes a deleted sequence together with its model, and tracks folders/deliverables", () => {
    let m = applyChange(base, {
      revision: 2,
      can_undo: true,
      can_redo: false,
      patches: [
        { op: "folder", id: "f", old: null, new: { id: "f", name: "AD 1", parent: null } },
        {
          op: "deliverable",
          id: "d",
          old: null,
          new: { id: "d", name: "D", sequence: "s", preset: "h264-mp4", path: "o.mp4" },
        },
      ],
    });
    expect(m.folders.f?.name).toBe("AD 1");
    expect(m.deliverables.d?.sequence).toBe("s");
    m = applyChange(m, {
      revision: 3,
      can_undo: true,
      can_redo: false,
      patches: [
        {
          op: "sequence",
          id: "s",
          old: { name: "s", frame_rate: "30", sample_rate: 48000 },
          new: null,
        },
        { op: "deliverable", id: "d", old: null, new: null },
      ],
    });
    expect(m.models.s).toBeUndefined();
    expect(m.deliverables.d).toBeUndefined();
  });
});
