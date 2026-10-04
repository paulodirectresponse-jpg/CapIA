import { describe, expect, it, vi } from "vitest";
import type {
  ChangeSet,
  EditorClient,
  ProjectSnapshot,
  SequenceModel,
} from "@capia/engine-bindings";
import { EditorController } from "./controller";

const FRAME = 23_520_000;

const seqModel = (): SequenceModel => ({
  header: { name: "S", frame_rate: "30", sample_rate: 48000, width: 1080, height: 1920 },
  tracks: [
    {
      id: "T1",
      kind: "visual",
      role: "overlay",
      name: "O",
      locked: false,
      hidden: false,
      muted: false,
      solo: false,
      magnetic: false,
      sync_lock: false,
      group: null,
    },
  ] as unknown as SequenceModel["tracks"],
  clips: {
    c1: {
      id: "c1",
      track: "T1",
      start: 0,
      duration: 60 * FRAME,
      name: "c1",
      enabled: true,
      content: { type: "solid", color: "#fff" },
      source_in: 0,
      speed: "1",
      reversed: false,
      properties: {},
    },
  },
  markers: {},
  frame_ticks: FRAME,
});

const snapshot = (): ProjectSnapshot => ({
  revision: 1,
  can_undo: false,
  can_redo: false,
  project: { path: "p.capia", name: "p" },
  sequences: [
    {
      id: "S",
      name: "S",
      frame_rate: "30",
      frame_ticks: FRAME,
      sample_rate: 48000,
      width: 1080,
      height: 1920,
      folder: null,
      duration: 60 * FRAME,
      clip_count: 1,
      nested_usage: 0,
    },
  ],
  folders: [],
  deliverables: [],
  assets: [],
});

const change = (revision: number): ChangeSet => ({
  revision,
  can_undo: true,
  can_redo: false,
  patches: [],
  sequence_summaries: [],
  touched_sequences: [],
});

function fake() {
  const calls: string[] = [];
  const client = {
    engineInfo: () => Promise.resolve({ media_available: true }),
    openProject: () => Promise.resolve(snapshot()),
    sequence: () => Promise.resolve(seqModel()),
    snapshot: vi.fn(() => Promise.resolve({ ...snapshot(), revision: 9 })),
    execute: vi.fn((label: string, cmds: { type: string }[]) => {
      calls.push(`execute:${cmds.map((c) => c.type).join("+")}`);
      return Promise.resolve(change(2));
    }),
    undo: vi.fn(() => {
      calls.push("undo");
      return Promise.resolve(change(3));
    }),
    redo: vi.fn(() => {
      calls.push("redo");
      return Promise.resolve(change(4));
    }),
    history: () => Promise.resolve({ entries: [], cursor: 0 }),
    assets: () => Promise.resolve([]),
    thumbnail: () => Promise.reject(new Error("x")),
  } as unknown as EditorClient;
  return { client, calls };
}

async function ready() {
  const { client, calls } = fake();
  const c = new EditorController(client, { storage: null, pollMs: 1_000_000 });
  await c.boot();
  await c.openProject("p.capia");
  return { c, calls, client };
}

function clip1(c: EditorController) {
  const cl = c.activeSequence()?.clips.c1;
  if (!cl) throw new Error("c1");
  return cl;
}

describe("EditorController: toda escrita é comando/undo/redo", () => {
  it("ações de edição só chamam execute/undo/redo", async () => {
    const { c, calls } = await ready();
    c.select(["c1"]);
    c.seek(10 * FRAME);
    await c.split();
    await c.deleteSelection(false);
    c.select(["c1"]);
    await c.setProperty(clip1(c), "opacity", 0.5);
    await c.setClipEnabled("c1", false);
    await c.addMarker();
    await c.undo();
    await c.redo();
    expect(calls.length).toBeGreaterThan(0);
    expect(calls.every((x) => x.startsWith("execute:") || x === "undo" || x === "redo")).toBe(true);
    expect(calls).toContain("execute:split_clip");
    expect(calls).toContain("execute:set_property");
  });

  it("selecionar, buscar, zoom e preferências não escrevem no engine", async () => {
    const { c, calls } = await ready();
    c.select(["c1"]);
    c.clearSelection();
    c.setZoom(120);
    c.setSnapping(false);
    c.seek(5 * FRAME);
    c.setPanels({ leftWidth: 400 });
    expect(calls).toEqual([]);
  });

  it("evento de revisão mais nova de outro cliente ressincroniza; a própria revisão é ignorada", async () => {
    const { c, client } = await ready();
    // eslint-disable-next-line @typescript-eslint/unbound-method -- é um vi.fn, sem this
    const snap = client.snapshot as unknown as ReturnType<typeof vi.fn>;
    c.handleEvent({ kind: "revision_changed", revision: 1 }); // já temos
    await new Promise((r) => setTimeout(r, 0));
    expect(snap.mock.calls.length).toBe(0);
    c.handleEvent({ kind: "revision_changed", revision: 9 });
    await new Promise((r) => setTimeout(r, 5));
    expect(snap.mock.calls.length).toBe(1);
    expect(c.state.model.revision).toBe(9);
  });

  it("resposta com revisão que pula uma (outro cliente commitou no intervalo) ressincroniza em vez de aplicar", async () => {
    const { c, client } = await ready();
    // réplica em r1; o engine responde r3 (r2 foi de outro cliente) ⇒ não pode aplicar o patch às cegas
    (client.execute as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(change(3));
    // eslint-disable-next-line @typescript-eslint/unbound-method -- é um vi.fn, sem this
    const snap = client.snapshot as unknown as ReturnType<typeof vi.fn>;
    await c.addMarker();
    expect(snap.mock.calls.length).toBe(1);
    expect(c.state.model.revision).toBe(9); // snapshot do engine (revisão verdadeira)
  });

  it("falha de comando vira toast + diagnóstico e mantém a réplica", async () => {
    const { c, client } = await ready();
    (client.execute as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      Object.assign(new Error("boom"), { code: "OVERLAP", details: undefined, name: "ApiError" }),
    );
    const before = c.state.model;
    await c.addMarker();
    expect(c.state.model).toBe(before);
    expect(c.state.toasts.length).toBe(1);
    expect(c.state.lastError).not.toBeNull();
  });
});
