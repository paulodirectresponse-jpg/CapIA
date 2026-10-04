/** Timecode e conversões de tempo da UI. Tempo é inteiro em Ticks (705.600.000/s). */
import { TICKS_PER_SECOND, type Ticks } from "@capia/engine-bindings";

/** Taxa nominal em quadros/s a partir do texto do engine (`30`, `30000/1001`). */
export function parseRate(text: string): { num: number; den: number } {
  const [n, d] = text.split("/");
  const num = Number(n);
  const den = d === undefined ? 1 : Number(d);
  return {
    num: Number.isFinite(num) && num > 0 ? num : 30,
    den: Number.isFinite(den) && den > 0 ? den : 1,
  };
}

/** Duração do quadro em ticks (inteiro, garantido pelo engine). */
export function frameTicks(rateText: string): Ticks {
  const { num, den } = parseRate(rateText);
  return Math.round((TICKS_PER_SECOND * den) / num);
}

/** Quadros/s inteiros para o timecode (arredonda 29,97 → 30; drop-frame não é usado). */
export function nominalFps(rateText: string): number {
  const { num, den } = parseRate(rateText);
  return Math.max(1, Math.round(num / den));
}

export function ticksToFrames(t: Ticks, frame: Ticks): number {
  return Math.floor(t / frame);
}

export function framesToTicks(n: number, frame: Ticks): Ticks {
  return n * frame;
}

/** `HH:MM:SS:FF` (FF = quadro dentro do segundo nominal). */
export function formatTimecode(t: Ticks, rateText: string): string {
  const fps = nominalFps(rateText);
  const frame = frameTicks(rateText);
  const total = Math.max(0, Math.floor(t / frame));
  const ff = total % fps;
  const secs = Math.floor(total / fps);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(secs / 3600))}:${pad(Math.floor(secs / 60) % 60)}:${pad(secs % 60)}:${pad(ff)}`;
}

export function secondsToTicks(s: number): Ticks {
  return Math.round(s * TICKS_PER_SECOND);
}

export function ticksToSeconds(t: Ticks): number {
  return t / TICKS_PER_SECOND;
}

/** Alinha ao quadro mais próximo (half-up). */
export function snapToFrame(t: Ticks, frame: Ticks): Ticks {
  return Math.floor((t + frame / 2) / frame) * frame;
}
