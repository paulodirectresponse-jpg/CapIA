import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { EngineClient, EngineInfo } from "@capia/engine-bindings";
import { App } from "./App";

const info: EngineInfo = {
  name: "capia-engine",
  version: "0.0.0",
  engine_api_version: 1,
  document_schema_version: 1,
  command_schema_version: 1,
  ticks_per_second: 705_600_000,
};

describe("App", () => {
  afterEach(cleanup);

  it("identifies CapIA and shows the engine reported by the injected client", async () => {
    const client: EngineClient = { getEngineInfo: () => Promise.resolve(info) };
    render(<App client={client} />);
    expect(screen.getByRole("heading", { name: "CapIA" })).toBeTruthy();
    expect((await screen.findByTestId("engine-status")).textContent).toContain(
      "Engine capia-engine v0.0.0",
    );
  });

  it("shows an unavailable state without crashing when the engine fails", async () => {
    const client: EngineClient = { getEngineInfo: () => Promise.reject(new Error("boom")) };
    render(<App client={client} />);
    expect((await screen.findByTestId("engine-status")).textContent).toBe(
      "Engine indisponível: boom",
    );
  });
});
