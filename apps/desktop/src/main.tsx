import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createEngineClient } from "@capia/engine-bindings";
import { App } from "@capia/editor-ui";
import { tauriTransport } from "./tauriTransport";

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

createRoot(root).render(
  <StrictMode>
    <App client={createEngineClient(tauriTransport)} />
  </StrictMode>,
);
