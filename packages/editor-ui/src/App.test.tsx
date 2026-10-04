import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { EditorClient } from "@capia/engine-bindings";
import { App } from "./App";

/** Cliente mínimo: só o que o boot usa. */
function fakeClient(engineInfo: () => Promise<unknown>): EditorClient {
  return { engineInfo } as unknown as EditorClient;
}

describe("App", () => {
  afterEach(cleanup);

  it("mostra a tela de boas-vindas depois de consultar o engine", async () => {
    render(
      <App
        client={fakeClient(() => Promise.resolve({ media_available: true }))}
        storage={null}
        pollMs={100000}
      />,
    );
    expect(await screen.findByTestId("welcome")).toBeTruthy();
  });

  it("não quebra quando o engine falha: continua nas boas-vindas", async () => {
    render(
      <App
        client={fakeClient(() => Promise.reject(new Error("boom")))}
        storage={null}
        pollMs={100000}
      />,
    );
    expect(await screen.findByTestId("welcome")).toBeTruthy();
  });
});
