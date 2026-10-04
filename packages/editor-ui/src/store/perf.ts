/** Amostras de desempenho percebido pela UI (ms), com teto. Só leitura para E2E/benchmarks. */
export type PerfKey = "commit" | "history" | "thumb" | "preview" | "rpc" | "apply";

const MAX = 1000;

export interface PerfSummary {
  n: number;
  p50: number;
  p95: number;
  max: number;
}

export function summarize(samples: readonly number[]): PerfSummary {
  if (samples.length === 0) return { n: 0, p50: 0, p95: 0, max: 0 };
  const s = [...samples].sort((a, b) => a - b);
  const at = (q: number) => s[Math.min(s.length - 1, Math.floor(q * s.length))] ?? 0;
  return { n: s.length, p50: at(0.5), p95: at(0.95), max: s[s.length - 1] ?? 0 };
}

export class PerfLog {
  private readonly data: Record<PerfKey, number[]> = {
    commit: [],
    history: [],
    thumb: [],
    preview: [],
    rpc: [],
    apply: [],
  };

  record(key: PerfKey, ms: number): void {
    const a = this.data[key];
    a.push(ms);
    if (a.length > MAX) a.shift();
  }

  samples(key: PerfKey): number[] {
    return [...this.data[key]];
  }

  summary(): Record<PerfKey, PerfSummary> {
    return {
      commit: summarize(this.data.commit),
      history: summarize(this.data.history),
      thumb: summarize(this.data.thumb),
      preview: summarize(this.data.preview),
      rpc: summarize(this.data.rpc),
      apply: summarize(this.data.apply),
    };
  }

  reset(): void {
    for (const k of Object.keys(this.data) as PerfKey[]) this.data[k].length = 0;
  }
}
