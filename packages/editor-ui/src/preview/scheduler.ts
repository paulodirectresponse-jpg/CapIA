/**
 * Agendador de quadros do preview: **o mais recente vence** (latest-wins). Há no máximo um pedido
 * em voo e um pendente; um pedido novo substitui o pendente (isso é um *dropped slot*). Assim o
 * scrub/playback nunca acumula fila e a latência mostrada é a do quadro realmente apresentado.
 */

export interface FrameData {
  width: number;
  height: number;
  rgba: Uint8Array<ArrayBuffer>;
  warnings: number;
}

export interface FrameRequest {
  at: number;
  width: number;
  height: number;
}

export interface FrameSink {
  readonly mode: "webgl" | "2d" | "none";
  present(frame: FrameData): void;
  dispose(): void;
}

export interface PreviewMetrics {
  presented: number;
  dropped: number;
  failed: number;
  lastLatencyMs: number;
  avgLatencyMs: number;
  /** Quadros apresentados por segundo (janela de ~1 s). */
  fps: number;
  mode: FrameSink["mode"];
}

export const EMPTY_METRICS: PreviewMetrics = {
  presented: 0,
  dropped: 0,
  failed: 0,
  lastLatencyMs: 0,
  avgLatencyMs: 0,
  fps: 0,
  mode: "none",
};

interface Pending extends FrameRequest {
  t0: number;
}

export class FrameScheduler {
  private inflight = false;
  private pending: Pending | null = null;
  private disposed = false;
  private presented = 0;
  private dropped = 0;
  private failed = 0;
  private last = 0;
  private avg = 0;
  private stamps: number[] = [];

  constructor(
    private readonly fetchFrame: (req: FrameRequest) => Promise<FrameData>,
    private readonly sink: FrameSink,
    private readonly onMetrics: (m: PreviewMetrics) => void,
    private readonly now: () => number = () => performance.now(),
  ) {}

  request(req: FrameRequest): void {
    if (this.disposed) return;
    if (this.pending) this.dropped += 1;
    this.pending = { ...req, t0: this.now() };
    if (!this.inflight) void this.pump();
  }

  private async pump(): Promise<void> {
    this.inflight = true;
    try {
      while (this.pending && !this.disposed) {
        const job = this.pending;
        this.pending = null;
        try {
          const frame = await this.fetchFrame(job);
          // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- muda durante o await
          if (this.disposed) return;
          this.sink.present(frame);
          const t = this.now();
          this.last = t - job.t0;
          this.avg = this.presented === 0 ? this.last : this.avg * 0.8 + this.last * 0.2;
          this.presented += 1;
          this.stamps.push(t);
          while (this.stamps.length > 0 && t - (this.stamps[0] ?? t) > 1000) this.stamps.shift();
        } catch {
          this.failed += 1;
        }
        this.emit();
      }
    } finally {
      this.inflight = false;
    }
  }

  metrics(): PreviewMetrics {
    return {
      presented: this.presented,
      dropped: this.dropped,
      failed: this.failed,
      lastLatencyMs: this.last,
      avgLatencyMs: this.avg,
      fps: this.stamps.length,
      mode: this.sink.mode,
    };
  }

  private emit(): void {
    this.onMetrics(this.metrics());
  }

  dispose(): void {
    this.disposed = true;
    this.pending = null;
    this.sink.dispose();
  }
}

/**
 * Qualidade automática: renderiza em 720p quando o quadro cabe no orçamento (latência média
 * abaixo de 2 quadros) e cai para 540p quando o preview não acompanha; histerese para não oscilar.
 */
export function pickAutoHeight(
  current: 540 | 720,
  avgLatencyMs: number,
  frameMs: number,
): 540 | 720 {
  if (avgLatencyMs <= 0) return current;
  if (current === 720 && avgLatencyMs > frameMs * 2) return 540;
  if (current === 540 && avgLatencyMs < frameMs * 0.8) return 720;
  return current;
}

/**
 * Qualidade automática **estável**: um pico isolado de latência (abrir o decode do próximo clip num
 * corte) nunca troca a resolução — troca recria o quadro e pisca. Exige a condição sustentada por
 * `CONFIRM` avaliações seguidas e respeita um intervalo mínimo entre trocas.
 */
export class AutoQuality {
  static readonly CONFIRM = 6;
  static readonly COOLDOWN_MS = 4000;
  private streak = 0;
  private lastSwitch = Number.NEGATIVE_INFINITY;

  constructor(private readonly now: () => number = () => performance.now()) {}

  update(current: 540 | 720, avgLatencyMs: number, frameMs: number): 540 | 720 {
    const want = pickAutoHeight(current, avgLatencyMs, frameMs);
    if (want === current) {
      this.streak = 0;
      return current;
    }
    this.streak += 1;
    if (this.streak < AutoQuality.CONFIRM || this.now() - this.lastSwitch < AutoQuality.COOLDOWN_MS)
      return current;
    this.streak = 0;
    this.lastSwitch = this.now();
    return want;
  }
}

/** Resolução de saída do preview: altura escolhida (e metade em modo proxy), largura pela razão. */
export function previewSize(
  seqW: number,
  seqH: number,
  quality: "auto" | "540" | "720",
  autoHeight: 540 | 720,
  proxy: boolean,
): { width: number; height: number } {
  const base = quality === "auto" ? autoHeight : quality === "540" ? 540 : 720;
  let height = proxy ? Math.round(base / 2) : base;
  height = Math.max(2, Math.min(height, seqH));
  height -= height % 2;
  let width = Math.round((height * seqW) / Math.max(1, seqH));
  width = Math.max(2, width);
  width -= width % 2;
  return { width, height };
}
