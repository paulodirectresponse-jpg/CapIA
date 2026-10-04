import { describe, expect, it } from "vitest";
import { TICKS_PER_SECOND } from "@capia/engine-bindings";
import { AudioMonitor, pcmFromReply, type AudioCtxLike, type PcmChunk } from "./audio";

function fakeCtx() {
  const starts: number[] = [];
  const ctx = {
    currentTime: 0,
    destination: {},
    resume: () => Promise.resolve(),
    createBuffer: (ch: number, frames: number) => {
      const data = Array.from({ length: ch }, () => new Float32Array(frames));
      return { getChannelData: (c: number) => data[c] ?? new Float32Array(0) };
    },
    createBufferSource: () => ({
      buffer: null,
      connect: () => undefined,
      start: (when: number) => starts.push(when),
      stop: () => undefined,
    }),
  } satisfies AudioCtxLike;
  return { ctx, starts };
}

const chunk = (frames = 24_000): PcmChunk => ({
  sampleRate: 48_000,
  channels: 2,
  frames,
  samples: new Float32Array(frames * 2).fill(0.5),
});
const tick = () => new Promise((r) => setTimeout(r, 0));

describe("AudioMonitor", () => {
  it("agenda blocos contíguos e expõe o relógio de áudio como posição", async () => {
    const { ctx, starts } = fakeCtx();
    const asked: number[] = [];
    const m = new AudioMonitor(
      (_s, from) => {
        asked.push(from);
        return Promise.resolve(chunk());
      },
      () => ctx,
    );
    expect(m.position()).toBeNull();
    m.start("S", 2 * TICKS_PER_SECOND, 60 * TICKS_PER_SECOND);
    await tick();
    await tick();
    expect(m.active).toBe(true);
    expect(starts.length).toBeGreaterThanOrEqual(2);
    // contíguos: cada bloco começa onde o anterior termina (0,5 s)
    expect((starts[1] ?? 0) - (starts[0] ?? 0)).toBeCloseTo(0.5, 6);
    expect((asked[1] ?? 0) - (asked[0] ?? 0)).toBe(TICKS_PER_SECOND / 2);
    expect(m.position()).toBe(2 * TICKS_PER_SECOND); // currentTime ainda 0 (dentro do colchão)
    ctx.currentTime = 1.06;
    expect(m.position()).toBe(3 * TICKS_PER_SECOND);
    m.stop();
    expect(m.active).toBe(false);
    expect(m.position()).toBeNull();
  });

  it("stop invalida o que estava em voo (nada é agendado depois)", async () => {
    const { ctx, starts } = fakeCtx();
    let release: (c: PcmChunk) => void = () => undefined;
    const m = new AudioMonitor(
      () => new Promise<PcmChunk>((r) => (release = r)),
      () => ctx,
    );
    m.start("S", 0, 10 * TICKS_PER_SECOND);
    m.stop();
    release(chunk());
    await tick();
    expect(starts).toEqual([]);
  });

  it("falha do engine desativa só o áudio (vídeo segue)", async () => {
    const { ctx } = fakeCtx();
    const m = new AudioMonitor(
      () => Promise.reject(new Error("boom")),
      () => ctx,
    );
    m.start("S", 0, 10 * TICKS_PER_SECOND);
    await tick();
    await tick();
    expect(m.active).toBe(false);
  });

  it("sem AudioContext não faz nada", () => {
    const m = new AudioMonitor(
      () => Promise.resolve(chunk()),
      () => null,
    );
    m.start("S", 0, TICKS_PER_SECOND);
    expect(m.active).toBe(false);
  });

  it("decodifica a resposta binária do engine", () => {
    const f = new Float32Array([0.25, -0.25, 0.5, -0.5]);
    const c = pcmFromReply(f.buffer, { sample_rate: 48000, channels: 2, frames: 2 });
    expect([c.sampleRate, c.channels, c.frames]).toEqual([48000, 2, 2]);
    expect([...c.samples]).toEqual([0.25, -0.25, 0.5, -0.5]);
  });
});
