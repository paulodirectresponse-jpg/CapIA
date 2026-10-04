/** Índice da sequence para a view: clips por track (ordenados) e metadados de renderização. */
import type { AssetRow, Clip, SequenceModel, Track } from "@capia/engine-bindings";
import { buildRows, type Row } from "./layout";

export interface TimelineData {
  seq: SequenceModel;
  rows: Row[];
  totalHeight: number;
  /** Clips por id de track, ordenados por `(start, id)`. */
  byTrack: Map<string, Clip[]>;
  /** Linha por id de track. */
  rowOf: Map<string, Row>;
  assets: Record<string, AssetRow>;
  /** Fim do último clip. */
  duration: number;
  /** Quantidade de clips (métrica). */
  clipCount: number;
}

export function buildData(
  seq: SequenceModel,
  assets: Record<string, AssetRow>,
  trackHeights: Record<string, number>,
): TimelineData {
  const byTrack = new Map<string, Clip[]>();
  for (const t of seq.tracks) byTrack.set(t.id, []);
  let duration = 0;
  let clipCount = 0;
  for (const c of Object.values(seq.clips)) {
    byTrack.get(c.track)?.push(c);
    duration = Math.max(duration, c.start + c.duration);
    clipCount++;
  }
  for (const list of byTrack.values()) {
    list.sort((a, b) => a.start - b.start || (a.id < b.id ? -1 : 1));
  }
  const { rows, totalHeight } = buildRows(seq.tracks, trackHeights);
  const rowOf = new Map(rows.map((r) => [r.track.id, r]));
  return { seq, rows, totalHeight, byTrack, rowOf, assets, duration, clipCount };
}

export type ClipKind = "video" | "audio" | "image" | "text" | "caption" | "solid" | "nested";

export function clipKind(c: Clip, track: Track): ClipKind {
  switch (c.content.type) {
    case "media":
      return c.content.has_video && track.kind === "visual" ? "video" : "audio";
    case "image":
      return "image";
    case "text":
      return track.role === "captions" ? "caption" : "text";
    case "solid":
      return "solid";
    case "nested":
      return "nested";
  }
}

/** Asset referenciado pelo clip (mídia/imagem). */
export function clipAssetId(c: Clip): string | null {
  return c.content.type === "media" || c.content.type === "image" ? c.content.asset : null;
}

/** `true` se o clip depende de mídia que não está online. */
export function clipOffline(c: Clip, assets: Record<string, AssetRow>): boolean {
  const id = clipAssetId(c);
  if (!id) return false;
  const a = assets[id];
  return a !== undefined && a.status !== "online";
}
