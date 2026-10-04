import { describe, expect, it } from "vitest";
import { FrameScheduler, pickAutoHeight, previewSize, type FrameData } from "./scheduler";

function deferred() {
  let resolve!: (f: FrameData) => void;
  const promise = new Promise<FrameData>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
const frame = (n: number): FrameData => ({
  width: 2,
  height: 2,
  rgba: new Uint8Array(16).fill(n),
  warnings: 0,
});
const sink = () => {
  const shown: number[] = [];
  return {
    shown,
    mode: "2d" as const,
    present: (f: FrameData) => shown.push(f.rgba[0] ?? -1),
    dispose: () => undefined,
  };
};

describe("FrameScheduler (latest-wins)", () => {
  it("descarta pedidos intermediários e conta dropped slots", async () => {
    const gates = [deferred(), deferred(), deferred()];
    const gate = (n: number) => {
      const g = gates[n];
      if (!g) throw new Error("gate");
      return g;
    };
    let i = 0;
    const calls: number[] = [];
    const s = sink();
    let t = 0;
    const sched = new FrameScheduler(
      (r) => {
        calls.push(r.at);
        return gate(i++).promise;
      },
      s,
      () => undefined,
      () => (t += 10),
    );
    sched.request({ at: 1, width: 2, height: 2 });
    sched.request({ at: 2, width: 2, height: 2 }); // pendente
    sched.request({ at: 3, width: 2, height: 2 }); // substitui o 2 → 1 dropped
    gate(0).resolve(frame(1));
    await new Promise((r) => setTimeout(r, 0));
    gate(1).resolve(frame(3));
    await new Promise((r) => setTimeout(r, 0));
    expect(calls).toEqual([1, 3]);
    expect(s.shown).toEqual([1, 3]);
    const m = sched.metrics();
    expect(m.dropped).toBe(1);
    expect(m.presented).toBe(2);
    expect(m.lastLatencyMs).toBeGreaterThan(0);
  });

  it("falha de render não derruba o agendador", async () => {
    const s = sink();
    let n = 0;
    const sched = new FrameScheduler(
      () => (n++ === 0 ? Promise.reject(new Error("x")) : Promise.resolve(frame(7))),
      s,
      () => undefined,
    );
    sched.request({ at: 1, width: 2, height: 2 });
    await new Promise((r) => setTimeout(r, 0));
    sched.request({ at: 2, width: 2, height: 2 });
    await new Promise((r) => setTimeout(r, 0));
    expect(sched.metrics().failed).toBe(1);
    expect(s.shown).toEqual([7]);
  });

  it("dispose impede apresentar depois", async () => {
    const g = deferred();
    const s = sink();
    const sched = new FrameScheduler(
      () => g.promise,
      s,
      () => undefined,
    );
    sched.request({ at: 1, width: 2, height: 2 });
    sched.dispose();
    g.resolve(frame(1));
    await new Promise((r) => setTimeout(r, 0));
    expect(s.shown).toEqual([]);
  });
});

describe("qualidade do preview", () => {
  it("540p/720p respeitam a razão e são pares; proxy reduz pela metade", () => {
    expect(previewSize(1920, 1080, "720", 720, false)).toEqual({ width: 1280, height: 720 });
    expect(previewSize(1080, 1920, "540", 720, false)).toEqual({ width: 304, height: 540 });
    expect(previewSize(1920, 1080, "720", 720, true)).toEqual({ width: 640, height: 360 });
    expect(previewSize(640, 360, "720", 720, false).height).toBe(360); // nunca amplia
  });
  it("auto tem histerese", () => {
    expect(pickAutoHeight(720, 100, 33)).toBe(540);
    expect(pickAutoHeight(720, 40, 33)).toBe(720);
    expect(pickAutoHeight(540, 20, 33)).toBe(720);
    expect(pickAutoHeight(540, 30, 33)).toBe(540);
  });
});
