import { describe, expect, it, vi } from "vitest";
import fixture from "../fixtures/engine_info.json";
import {
  createEngineClient,
  ENGINE_COMMANDS,
  EngineContractError,
  parseEngineInfo,
  type EngineTransport,
} from "./index";

describe("engine contract", () => {
  it("accepts the fixture shared with the Rust test", () => {
    expect(parseEngineInfo(fixture)).toEqual(fixture);
  });

  it("keeps ticks_per_second a JavaScript-safe integer", () => {
    expect(Number.isSafeInteger(fixture.ticks_per_second)).toBe(true);
    expect(fixture.ticks_per_second).toBe(705_600_000);
  });

  it.each([null, 42, "x", {}, { ...fixture, ticks_per_second: 1.5 }, { ...fixture, name: 7 }])(
    "rejects malformed payload %j",
    (bad) => {
      expect(() => parseEngineInfo(bad)).toThrow(EngineContractError);
    },
  );
});

describe("createEngineClient", () => {
  it("invokes the documented command and validates the answer", async () => {
    const invoke = vi.fn<EngineTransport["invoke"]>().mockResolvedValue(fixture);
    const info = await createEngineClient({ invoke }).getEngineInfo();
    expect(invoke).toHaveBeenCalledWith(ENGINE_COMMANDS.getEngineInfo);
    expect(info.name).toBe("capia-engine");
  });

  it("rejects when the engine answers with an invalid payload", async () => {
    const invoke = vi.fn<EngineTransport["invoke"]>().mockResolvedValue({ nope: true });
    await expect(createEngineClient({ invoke }).getEngineInfo()).rejects.toThrow(
      EngineContractError,
    );
  });
});
