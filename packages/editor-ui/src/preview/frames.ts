import type { EditorClient } from "@capia/engine-bindings";
import type { FrameData, FrameRequest } from "./scheduler";

/**
 * De onde vêm os quadros do preview. O padrão (`ipc`) recebe os bytes pela resposta binária do
 * transporte; o shell desktop pode fornecer o caminho P2 do ADR-069 (`shared-buffer`: o engine
 * escreve direto numa memória compartilhada com o WebView2, sem cópia por IPC).
 */
export interface FrameSource {
  readonly kind: "ipc" | "shared-buffer";
  render(sequence: string, req: FrameRequest): Promise<FrameData>;
  dispose?(): void;
}

export function ipcFrameSource(client: EditorClient): FrameSource {
  return {
    kind: "ipc",
    async render(sequence, req) {
      const r = await client.renderFrame(sequence, req.at, req.width, req.height);
      const w = r.meta.warnings;
      return {
        width: Number(r.meta.width),
        height: Number(r.meta.height),
        rgba: new Uint8Array(r.bytes),
        warnings: Array.isArray(w) ? w.length : 0,
      };
    },
  };
}
