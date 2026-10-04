/** Geometria da timeline: tempo ↔ pixel, linhas de tracks e escala da régua (funções puras). */
import { TICKS_PER_SECOND, type Clip, type Track, type Ticks } from "@capia/engine-bindings";

export const RULER_H = 28;
export const MIN_PPS = 2;
export const MAX_PPS = 4000;
export const DEFAULT_ROW_H = { visual: 56, audio: 44 } as const;
export const MIN_ROW_H = 24;
export const MAX_ROW_H = 240;

export function ticksToPx(t: Ticks, pps: number, originTicks: Ticks): number {
  return ((t - originTicks) / TICKS_PER_SECOND) * pps;
}

export function pxToTicks(x: number, pps: number, originTicks: Ticks): Ticks {
  return Math.round(originTicks + (x / pps) * TICKS_PER_SECOND);
}

export function clampPps(pps: number): number {
  return Math.min(MAX_PPS, Math.max(MIN_PPS, pps));
}

export interface Row {
  track: Track;
  /** Posição no conteúdo (px, a partir do topo da área de tracks). */
  y: number;
  h: number;
  /** Índice de exibição (0 = linha do topo). */
  displayIndex: number;
}

/**
 * Ordem de exibição: tracks visuais do **topo da pilha para a base** (a última do array é a
 * mais alta no compositor), depois as de áudio na ordem do documento.
 */
export function displayOrder(tracks: Track[]): Track[] {
  const visual = tracks.filter((t) => t.kind === "visual").reverse();
  const audio = tracks.filter((t) => t.kind === "audio");
  return [...visual, ...audio];
}

export function buildRows(
  tracks: Track[],
  heights: Record<string, number>,
): { rows: Row[]; totalHeight: number } {
  let y = 0;
  const rows = displayOrder(tracks).map((track, displayIndex) => {
    const h = Math.min(
      MAX_ROW_H,
      Math.max(MIN_ROW_H, heights[track.id] ?? DEFAULT_ROW_H[track.kind]),
    );
    const row: Row = { track, y, h, displayIndex };
    y += h;
    return row;
  });
  return { rows, totalHeight: y };
}

/** Linha em `contentY` (px na área de tracks) ou `null`. Busca binária. */
export function rowAt(rows: Row[], contentY: number): Row | null {
  if (contentY < 0) return null;
  let lo = 0;
  let hi = rows.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const r = rows[mid];
    if (!r) return null;
    if (contentY < r.y) hi = mid - 1;
    else if (contentY >= r.y + r.h) lo = mid + 1;
    else return r;
  }
  return null;
}

/** Passos "bonitos" da régua em segundos (quadros só aparecem com zoom alto). */
const STEPS_S = [
  0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600,
];

/** Menor passo cujo espaçamento em pixels ≥ `minPx`. */
export function rulerStepSeconds(pps: number, minPx = 90): number {
  for (const s of STEPS_S) if (s * pps >= minPx) return s;
  return STEPS_S[STEPS_S.length - 1] ?? 3600;
}

/** Clips da track com sobreposição à janela `[from, to)`, de uma lista ordenada por início. */
export function visibleClips(sorted: Clip[], from: Ticks, to: Ticks): Clip[] {
  // primeiro clip cujo fim > from: como não há sobreposição na track, o fim é crescente com o início
  let lo = 0;
  let hi = sorted.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    const c = sorted[mid];
    if (c && c.start + c.duration <= from) lo = mid + 1;
    else hi = mid;
  }
  const out: Clip[] = [];
  for (let i = lo; i < sorted.length; i++) {
    const c = sorted[i];
    if (!c || c.start >= to) break;
    out.push(c);
  }
  return out;
}

/** Clip em `t` numa lista ordenada (busca binária), ou `null`. */
export function clipAt(sorted: Clip[], t: Ticks): Clip | null {
  let lo = 0;
  let hi = sorted.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const c = sorted[mid];
    if (!c) return null;
    if (t < c.start) hi = mid - 1;
    else if (t >= c.start + c.duration) lo = mid + 1;
    else return c;
  }
  return null;
}

/** Zoom que cabe `duration` em `widthPx` (com margem de 4%). */
export function fitPps(duration: Ticks, widthPx: number): number {
  if (duration <= 0 || widthPx <= 0) return 80;
  return clampPps((widthPx * 0.96) / (duration / TICKS_PER_SECOND));
}
