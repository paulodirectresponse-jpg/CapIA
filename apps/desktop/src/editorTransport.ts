import { invoke } from "@tauri-apps/api/core";
import {
  ApiError,
  createHttpTransport,
  type ApiErrorBody,
  type BinaryReply,
  type EditorTransport,
} from "@capia/engine-bindings";

/** Estamos dentro do shell Tauri (e não num navegador servido pelo capia-devserver)? */
export function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function asApiError(e: unknown): ApiError {
  if (typeof e === "object" && e !== null && "code" in e && "message" in e) {
    return new ApiError(e as ApiErrorBody);
  }
  return new ApiError({ code: "IPC_ERROR", message: e instanceof Error ? e.message : String(e) });
}

/**
 * Desempacota `u32 LE tamanho do JSON · JSON({mime, meta}) · bytes` (espelho de `pack_binary` em
 * `apps/desktop/src-tauri`). `bytes` é uma visão copiada uma única vez para um ArrayBuffer próprio.
 */
export function unpackBinary(buf: ArrayBuffer): BinaryReply {
  const view = new DataView(buf);
  const n = view.getUint32(0, true);
  const header = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 4, n))) as {
    mime: string;
    meta: Record<string, unknown>;
  };
  return { mime: header.mime, meta: header.meta, bytes: buf.slice(4 + n) };
}

/** Transporte de produto: IPC tipado do Tauri (`editor_call` / `editor_call_binary`). */
export const tauriEditorTransport: EditorTransport = {
  async call(method, params) {
    try {
      return await invoke("editor_call", { method, params: params ?? {} });
    } catch (e) {
      throw asApiError(e);
    }
  },
  async callBinary(method, params) {
    try {
      const raw = await invoke<ArrayBuffer>("editor_call_binary", { method, params: params ?? {} });
      return unpackBinary(raw);
    } catch (e) {
      throw asApiError(e);
    }
  },
};

/** Tauri no produto; HTTP (`/api`) quando a página é servida pelo devserver (dev/E2E). */
export function pickTransport(): EditorTransport {
  return insideTauri() ? tauriEditorTransport : createHttpTransport("");
}
