import { invoke } from "@tauri-apps/api/core";
import type { EngineTransport } from "@capia/engine-bindings";

/** Único lugar da UI que conhece o Tauri: adapta `invoke` ao contrato de transporte do engine. */
export const tauriTransport: EngineTransport = {
  invoke: (command, args) => invoke(command, args),
};
