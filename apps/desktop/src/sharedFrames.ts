import { invoke } from "@tauri-apps/api/core";
import type { FrameData, FrameRequest, FrameSource } from "@capia/editor-ui";

/** Espelha `capia_webview_surface`: `"CAPF"` LE, seq, width, height; pixels a partir do byte 64. */
const MAGIC = 0x4650_4143;
const HEADER = 64;
const MAX_SIDE = 1280;

interface WebView2Bridge {
  addEventListener(
    type: "sharedbufferreceived",
    cb: (e: { getBuffer(): ArrayBuffer; additionalData?: unknown }) => void,
  ): void;
  removeEventListener?(type: string, cb: unknown): void;
}

function bridge(): WebView2Bridge | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as { chrome?: { webview?: WebView2Bridge } };
  return w.chrome?.webview ?? null;
}

/**
 * Lê um quadro do SharedBuffer depois de o engine tê-lo escrito. Exportada para teste: só valida
 * o cabeçalho e devolve uma **visão** (zero cópia) dos pixels.
 */
export function readSharedFrame(buf: ArrayBuffer, expectedSeq: number): FrameData {
  const v = new DataView(buf);
  if (v.getUint32(0, true) !== MAGIC) throw new Error("SharedBuffer sem cabeçalho CAPF");
  const seq = v.getUint32(4, true);
  const width = v.getUint32(8, true);
  const height = v.getUint32(12, true);
  if (seq !== expectedSeq)
    throw new Error(`quadro fora de ordem (${String(seq)} ≠ ${String(expectedSeq)})`);
  const bytes = width * height * 4;
  if (HEADER + bytes > buf.byteLength) throw new Error("quadro maior que o SharedBuffer");
  return { width, height, rgba: new Uint8Array(buf, HEADER, bytes), warnings: 0 };
}

/**
 * Cria a fonte P2 (SharedBuffer → WebGL) ou devolve `null` (fora do WebView2, versão antiga do
 * runtime, timeout): quem chama mantém o caminho por IPC. Nunca lança.
 */
export async function createSharedFrameSource(fallback: FrameSource): Promise<FrameSource | null> {
  const wv = bridge();
  if (!wv) return null;
  try {
    const received = new Promise<ArrayBuffer>((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error("SharedBuffer não chegou à página em 5 s"));
      }, 5000);
      wv.addEventListener("sharedbufferreceived", (e) => {
        clearTimeout(timer);
        resolve(e.getBuffer());
      });
    });
    await invoke("preview_surface_init");
    const buf = await received;
    let seq = 0;
    return {
      kind: "shared-buffer",
      async render(sequence: string, req: FrameRequest): Promise<FrameData> {
        if (req.width > MAX_SIDE || req.height > MAX_SIDE) return fallback.render(sequence, req);
        const r = await invoke<{ seq: number; warnings?: unknown }>("preview_render_shared", {
          sequence,
          at: req.at,
          width: req.width,
          height: req.height,
        });
        seq = r.seq;
        const frame = readSharedFrame(buf, seq);
        return { ...frame, warnings: Array.isArray(r.warnings) ? r.warnings.length : 0 };
      },
    };
  } catch {
    return null;
  }
}
