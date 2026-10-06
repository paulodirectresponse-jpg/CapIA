/** Medidas de progresso de export exibidas ao usuário (tudo derivado de quadros e relógio). */
export interface ExportStats {
  percent: number;
  elapsedMs: number;
  /** `null` até haver quadros suficientes para estimar. */
  etaMs: number | null;
  /** Quadros renderizados por segundo. */
  fps: number;
  /** Segundos de vídeo gerados por segundo de relógio (precisa da taxa da sequence). */
  realtime: number | null;
}

export function exportStats(
  done: number,
  total: number,
  startedAt: number | undefined,
  now: number,
  /** Quadros por segundo da sequence exportada (`null` se desconhecido). */
  seqFps: number | null,
): ExportStats {
  const elapsedMs = startedAt === undefined ? 0 : Math.max(0, now - startedAt);
  const percent = total > 0 ? Math.min(100, Math.max(0, Math.floor((done / total) * 100))) : 0;
  const fps = elapsedMs > 0 ? (done * 1000) / elapsedMs : 0;
  const etaMs =
    done > 0 && total > done && elapsedMs > 0
      ? Math.round((elapsedMs / done) * (total - done))
      : total > 0 && done >= total
        ? 0
        : null;
  const realtime = seqFps !== null && seqFps > 0 && fps > 0 ? fps / seqFps : null;
  return { percent, elapsedMs, etaMs, fps, realtime };
}

/** `83_000` → `1:23`; `3_723_000` → `1:02:03`. */
export function formatDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${String(h)}:${two(m)}:${two(r)}` : `${String(m)}:${two(r)}`;
}

/** `"30"`, `"30000/1001"` → número. */
export function rateToFps(rate: string): number | null {
  const [n, d] = rate.split("/");
  const num = Number(n);
  const den = d === undefined ? 1 : Number(d);
  return Number.isFinite(num) && Number.isFinite(den) && den > 0 && num > 0 ? num / den : null;
}
