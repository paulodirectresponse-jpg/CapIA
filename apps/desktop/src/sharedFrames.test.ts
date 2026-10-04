import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

function packed(seq: number, w: number, h: number, fill: number): ArrayBuffer {
  const buf = new ArrayBuffer(64 + w * h * 4 + 32);
  const v = new DataView(buf);
  v.setUint32(0, 0x4650_4143, true);
  v.setUint32(4, seq, true);
  v.setUint32(8, w, true);
  v.setUint32(12, h, true);
  new Uint8Array(buf, 64, w * h * 4).fill(fill);
  return buf;
}

describe("readSharedFrame", () => {
  it("devolve uma visão (zero cópia) dos pixels depois do cabeçalho", async () => {
    const { readSharedFrame } = await import("./sharedFrames");
    const buf = packed(5, 2, 2, 7);
    const f = readSharedFrame(buf, 5);
    expect([f.width, f.height]).toEqual([2, 2]);
    expect(f.rgba.byteLength).toBe(16);
    expect(f.rgba.buffer).toBe(buf);
    expect(f.rgba[0]).toBe(7);
  });

  it("recusa cabeçalho inválido, quadro fora de ordem e quadro maior que o buffer", async () => {
    const { readSharedFrame } = await import("./sharedFrames");
    expect(() => readSharedFrame(new ArrayBuffer(128), 1)).toThrow(/CAPF/);
    expect(() => readSharedFrame(packed(4, 2, 2, 1), 5)).toThrow(/fora de ordem/);
    const big = packed(1, 2, 2, 1);
    new DataView(big).setUint32(8, 4000, true);
    expect(() => readSharedFrame(big, 1)).toThrow(/maior/);
  });

  it("sem a ponte do WebView2 a fonte P2 não existe (o IPC assume)", async () => {
    const { createSharedFrameSource } = await import("./sharedFrames");
    const fb = { kind: "ipc" as const, render: vi.fn() };
    await expect(createSharedFrameSource(fb)).resolves.toBeNull();
  });
});
