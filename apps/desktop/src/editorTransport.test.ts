import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

function pack(meta: unknown, bytes: number[]): ArrayBuffer {
  const header = new TextEncoder().encode(JSON.stringify({ mime: "application/x-rgba", meta }));
  const out = new Uint8Array(4 + header.length + bytes.length);
  new DataView(out.buffer).setUint32(0, header.length, true);
  out.set(header, 4);
  out.set(bytes, 4 + header.length);
  return out.buffer;
}

describe("editor transport (Tauri)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("repassa método e parâmetros ao IPC fixo editor_call", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    const { tauriEditorTransport } = await import("./editorTransport");
    await expect(tauriEditorTransport.call("engine.info", { a: 1 })).resolves.toEqual({ ok: true });
    expect(invokeMock).toHaveBeenCalledWith("editor_call", {
      method: "engine.info",
      params: { a: 1 },
    });
  });

  it("converte erro estruturado do engine em ApiError", async () => {
    invokeMock.mockRejectedValue({ code: "OVERLAP", message: "x" });
    const { tauriEditorTransport } = await import("./editorTransport");
    await expect(tauriEditorTransport.call("command.execute")).rejects.toMatchObject({
      name: "ApiError",
      code: "OVERLAP",
    });
  });

  it("desempacota bytes + metadados do editor_call_binary", async () => {
    invokeMock.mockResolvedValue(pack({ width: 2, height: 1 }, [9, 8, 7]));
    const { tauriEditorTransport } = await import("./editorTransport");
    const r = await tauriEditorTransport.callBinary("render.frame", {});
    expect(r.mime).toBe("application/x-rgba");
    expect(r.meta).toEqual({ width: 2, height: 1 });
    expect([...new Uint8Array(r.bytes)]).toEqual([9, 8, 7]);
  });
});
