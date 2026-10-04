/**
 * Cache de visuais de mídia (miniaturas e picos de waveform). Os payloads grandes não passam pelo
 * contrato JSON do documento: vêm por chamadas binárias/sob demanda e ficam num cache com limite.
 * A timeline consulta de forma **síncrona** (devolve o que já está pronto e dispara o carregamento).
 */
import type { EditorClient } from "@capia/engine-bindings";

const MAX_THUMBS = 400;
const MAX_PEAK_SETS = 200;

export class MediaVisuals {
  private thumbs = new Map<string, HTMLImageElement | "loading" | "failed">();
  private urls = new Map<string, string>();
  private peaks = new Map<string, Float32Array | "loading" | "failed">();
  private listeners = new Set<() => void>();
  private disposed = false;

  constructor(
    private readonly client: EditorClient,
    private readonly onLatency: (ms: number) => void = () => undefined,
  ) {}

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  private notify(): void {
    for (const f of [...this.listeners]) f();
  }

  /** Miniatura pronta (ou `null`, iniciando o carregamento). */
  thumbnail(asset: string): HTMLImageElement | null {
    const cur = this.thumbs.get(asset);
    if (cur && cur !== "loading" && cur !== "failed") return cur;
    if (!cur) void this.loadThumb(asset);
    return null;
  }

  /** URL (blob:) da miniatura para `<img>`, ou `null` se ainda não carregou. */
  thumbnailUrl(asset: string): string | null {
    this.thumbnail(asset);
    return this.urls.get(asset) ?? null;
  }

  private async loadThumb(asset: string): Promise<void> {
    this.thumbs.set(asset, "loading");
    const t0 = performance.now();
    try {
      const r = await this.client.thumbnail(asset, 0, 240);
      if (this.disposed) return;
      const url = URL.createObjectURL(new Blob([r.bytes], { type: r.mime }));
      const img = new Image();
      img.src = url;
      await img.decode().catch(() => undefined);
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- muda durante o await
      if (this.disposed) {
        URL.revokeObjectURL(url);
        return;
      }
      this.evict(this.thumbs, MAX_THUMBS, (k, v) => {
        const u = this.urls.get(k);
        if (u) URL.revokeObjectURL(u);
        this.urls.delete(k);
        return v;
      });
      this.urls.set(asset, url);
      this.thumbs.set(asset, img);
      this.onLatency(performance.now() - t0);
    } catch {
      this.thumbs.set(asset, "failed");
    }
    this.notify();
  }

  /** Picos (min,max)*buckets do asset inteiro, ou `null` (iniciando o carregamento). */
  waveform(asset: string, buckets: number): Float32Array | null {
    const key = `${asset}:${String(buckets)}`;
    const cur = this.peaks.get(key);
    if (cur && cur !== "loading" && cur !== "failed") return cur;
    if (!cur) void this.loadPeaks(asset, buckets, key);
    return null;
  }

  private async loadPeaks(asset: string, buckets: number, key: string): Promise<void> {
    this.peaks.set(key, "loading");
    try {
      const r = await this.client.peaks(asset, buckets);
      if (this.disposed) return;
      this.evict(this.peaks, MAX_PEAK_SETS, (_k, v) => v);
      this.peaks.set(key, Float32Array.from(r.peaks));
    } catch {
      this.peaks.set(key, "failed");
    }
    this.notify();
  }

  /** Invalida os visuais de um asset (relink/troca de arquivo). */
  invalidate(asset: string): void {
    this.thumbs.delete(asset);
    const u = this.urls.get(asset);
    if (u) URL.revokeObjectURL(u);
    this.urls.delete(asset);
    for (const k of [...this.peaks.keys()]) if (k.startsWith(`${asset}:`)) this.peaks.delete(k);
    this.notify();
  }

  private evict<V>(map: Map<string, V>, max: number, onEvict: (k: string, v: V) => V): void {
    while (map.size >= max) {
      const first = map.keys().next();
      if (first.done) break;
      const v = map.get(first.value);
      map.delete(first.value);
      if (v !== undefined) onEvict(first.value, v);
    }
  }

  dispose(): void {
    this.disposed = true;
    for (const u of this.urls.values()) URL.revokeObjectURL(u);
    this.urls.clear();
    this.thumbs.clear();
    this.peaks.clear();
    this.listeners.clear();
  }
}
