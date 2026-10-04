import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { EditorClient } from "@capia/engine-bindings";
import { App } from "@capia/editor-ui";
import { TimelineCore } from "@capia/ui-timeline";
import wasmUrl from "../../../packages/ui-timeline/src/generated/capia_timeline.wasm?url";
import { insideTauri, pickTransport } from "./editorTransport";
import { tauriPlatform } from "./platform";

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

const client = new EditorClient(pickTransport());

createRoot(root).render(
  <StrictMode>
    <App
      client={client}
      {...(insideTauri() ? { platform: tauriPlatform } : {})}
      loadCore={() => TimelineCore.load(fetch(wasmUrl))}
    />
  </StrictMode>,
);
