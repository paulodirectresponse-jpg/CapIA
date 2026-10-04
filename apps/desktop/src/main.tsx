import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { EditorClient } from "@capia/engine-bindings";
import { App, ipcFrameSource } from "@capia/editor-ui";
import { TimelineCore } from "@capia/ui-timeline";
import wasmUrl from "../../../packages/ui-timeline/src/generated/capia_timeline.wasm?url";
import { insideTauri, pickTransport } from "./editorTransport";
import { tauriPlatform } from "./platform";
import { createSharedFrameSource } from "./sharedFrames";

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

const client = new EditorClient(pickTransport());

// P2 (ADR-069): SharedBuffer do WebView2 quando disponível; senão o mesmo preview por IPC binário.
const ipcFrames = ipcFrameSource(client);
const shared = insideTauri() ? await createSharedFrameSource(ipcFrames) : null;

createRoot(root).render(
  <StrictMode>
    <App
      client={client}
      frames={shared ?? ipcFrames}
      {...(insideTauri() ? { platform: tauriPlatform } : {})}
      loadCore={() => TimelineCore.load(fetch(wasmUrl))}
    />
  </StrictMode>,
);
