import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

describe("tauriTransport", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("forwards commands to the Tauri IPC unchanged", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    const { tauriTransport } = await import("./tauriTransport");
    await expect(tauriTransport.invoke("get_engine_info")).resolves.toEqual({ ok: true });
    expect(invokeMock).toHaveBeenCalledWith("get_engine_info", undefined);
  });
});
