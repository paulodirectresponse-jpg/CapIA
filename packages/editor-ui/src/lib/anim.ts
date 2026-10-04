import type { Animatable, Clip, Interp, Ticks } from "@capia/engine-bindings";
import { parseRate } from "./timecode";

/** Propriedades animáveis de um clip visual, com faixa e passo para os campos do inspector. */
export const ANIMATABLE: {
  name: string;
  min: number;
  max: number;
  dflt: number;
  step: number;
}[] = [
  { name: "position_x", min: -100000, max: 100000, dflt: 0, step: 1 },
  { name: "position_y", min: -100000, max: 100000, dflt: 0, step: 1 },
  { name: "scale", min: 0, max: 100, dflt: 1, step: 0.01 },
  { name: "rotation", min: -36000, max: 36000, dflt: 0, step: 90 },
  { name: "opacity", min: 0, max: 1, dflt: 1, step: 0.05 },
  { name: "volume_db", min: -120, max: 24, dflt: 0, step: 0.5 },
];

export const EASE: Interp = { bezier: { x1: 0.42, y1: 0, x2: 0.58, y2: 1 } };

export function interpKind(i: Interp): "linear" | "hold" | "ease" {
  if (i === "hold") return "hold";
  if (i === "linear") return "linear";
  return "ease";
}

export function interpFromKind(k: "linear" | "hold" | "ease"): Interp {
  return k === "ease" ? EASE : k;
}

/** `cubic-bezier` como no CSS: progresso de valor para a fração de tempo `u`. */
function bezierEase(b: { x1: number; y1: number; x2: number; y2: number }, u: number): number {
  const cx = 3 * b.x1;
  const bx = 3 * (b.x2 - b.x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * b.y1;
  const by = 3 * (b.y2 - b.y1) - cy;
  const ay = 1 - cy - by;
  const sx = (s: number) => ((ax * s + bx) * s + cx) * s;
  const sy = (s: number) => ((ay * s + by) * s + cy) * s;
  let s = u;
  for (let i = 0; i < 8; i++) {
    const err = sx(s) - u;
    if (Math.abs(err) < 1e-6) return sy(s);
    const d = (3 * ax * s + 2 * bx) * s + cx;
    if (Math.abs(d) < 1e-6) break;
    s -= err / d;
  }
  let lo = 0;
  let hi = 1;
  s = u;
  for (let i = 0; i < 24; i++) {
    const x = sx(s);
    if (Math.abs(x - u) < 1e-6) break;
    if (x < u) lo = s;
    else hi = s;
    s = (lo + hi) / 2;
  }
  return sy(s);
}

/** Valor da propriedade no tempo de conteúdo `ct` (só para exibição; o render é do engine). */
export function evalAt(a: Animatable | undefined, ct: Ticks, dflt: number): number {
  if (!a) return dflt;
  if ("static" in a) return a.static;
  const k = a.animated;
  const first = k[0];
  const last = k[k.length - 1];
  if (!first || !last) return dflt;
  if (ct <= first.time) return first.value;
  if (ct >= last.time) return last.value;
  for (let i = 0; i + 1 < k.length; i++) {
    const a0 = k[i];
    const a1 = k[i + 1];
    if (!a0 || !a1 || ct >= a1.time) continue;
    if (a0.interp === "hold") return a0.value;
    const u = (ct - a0.time) / Math.max(1, a1.time - a0.time);
    const p = a0.interp === "linear" ? u : bezierEase(a0.interp.bezier, u);
    return a0.value + (a1.value - a0.value) * p;
  }
  return last.value;
}

/** Tempo de conteúdo ↔ tempo da sequence (velocidade racional e reverso). */
export function contentTimeAt(clip: Clip, t: Ticks): Ticks {
  const { num, den } = parseRate(clip.speed);
  const local = t - clip.start;
  const off = clip.reversed ? clip.duration - local : local;
  return Math.round(clip.source_in + (off * num) / den);
}

export function sequenceTimeOf(clip: Clip, ct: Ticks): Ticks {
  const { num, den } = parseRate(clip.speed);
  const off = Math.round(((ct - clip.source_in) * den) / num);
  return clip.reversed ? clip.start + clip.duration - off : clip.start + off;
}

export function keyframesOf(
  clip: Clip,
  prop: string,
): { at: Ticks; value: number; interp: Interp }[] {
  const a = clip.properties[prop];
  if (!a || !("animated" in a)) return [];
  return a.animated.map((k) => ({
    at: sequenceTimeOf(clip, k.time),
    value: k.value,
    interp: k.interp,
  }));
}
