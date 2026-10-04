import { TICKS_PER_SECOND, type Ticks } from "@capia/engine-bindings";

/** Trecho de PCM f32 intercalado devolvido pelo engine (`render.audio`). */
export interface PcmChunk {
  sampleRate: number;
  channels: number;
  frames: number;
  samples: Float32Array;
}

/** O mínimo do WebAudio que usamos (permite testar sem navegador). */
export interface AudioCtxLike {
  readonly currentTime: number;
  readonly destination: unknown;
  resume(): Promise<void>;
  createBuffer(channels: number, frames: number, sampleRate: number): AudioBufferLike;
  createBufferSource(): SourceLike;
  close?(): Promise<void>;
}
export interface AudioBufferLike {
  getChannelData(ch: number): Float32Array;
}
export interface SourceLike {
  buffer: AudioBufferLike | null;
  connect(dest: unknown): void;
  start(when: number): void;
  stop(): void;
}

const CHUNK_SECONDS = 0.5;
const AHEAD_SECONDS = 1.5;
const START_CUSHION = 0.06;

/**
 * Monitoração de áudio do preview. A 1× o **relógio de áudio é o mestre**: o playhead anda
 * conforme o que o `AudioContext` já tocou, então imagem e som não se afastam. O engine mixa
 * (mute/solo/volume/fades) — aqui só agendamos blocos contíguos de 0,5 s com ~1,5 s de folga.
 */
export class AudioMonitor {
  private ctx: AudioCtxLike | null = null;
  private gen = 0;
  private origin: { ticks: Ticks; ctxTime: number } | null = null;
  private sources: SourceLike[] = [];
  private nextTicks: Ticks = 0;
  private nextCtxTime = 0;
  private running = false;

  constructor(
    private readonly fetchChunk: (seq: string, from: Ticks, duration: Ticks) => Promise<PcmChunk>,
    private readonly makeCtx: () => AudioCtxLike | null,
  ) {}

  /** Há áudio tocando (ou prestes a)? */
  get active(): boolean {
    return this.running;
  }

  /** Posição (ticks) segundo o relógio de áudio; `null` até o 1º bloco estar agendado. */
  position(): Ticks | null {
    if (!this.running || !this.ctx || !this.origin) return null;
    const elapsed = Math.max(0, this.ctx.currentTime - this.origin.ctxTime);
    return this.origin.ticks + Math.round(elapsed * TICKS_PER_SECOND);
  }

  start(seq: string, at: Ticks, durationTicks: Ticks): void {
    this.stop();
    this.ctx ??= this.makeCtx();
    const ctx = this.ctx;
    if (!ctx) return;
    this.running = true;
    const gen = ++this.gen;
    this.nextTicks = at;
    void ctx.resume().catch(() => undefined);
    void this.pump(gen, seq, durationTicks);
  }

  stop(): void {
    this.gen++;
    this.running = false;
    this.origin = null;
    for (const s of this.sources) {
      try {
        s.stop();
      } catch {
        /* já terminou */
      }
    }
    this.sources = [];
  }

  dispose(): void {
    this.stop();
    void this.ctx?.close?.().catch(() => undefined);
    this.ctx = null;
  }

  private async pump(gen: number, seq: string, durationTicks: Ticks): Promise<void> {
    const chunkTicks = Math.round(CHUNK_SECONDS * TICKS_PER_SECOND);
    while (this.gen === gen && this.ctx && this.nextTicks < durationTicks) {
      const ctx = this.ctx;
      const aheadOk = this.origin !== null && this.nextCtxTime - ctx.currentTime >= AHEAD_SECONDS;
      if (aheadOk) {
        await new Promise((r) => setTimeout(r, 100));
        continue;
      }
      let chunk: PcmChunk;
      try {
        chunk = await this.fetchChunk(seq, this.nextTicks, chunkTicks);
      } catch {
        // sem áudio (falha do engine): o vídeo segue pelo relógio normal
        if (this.gen === gen) this.running = false;
        return;
      }
      if (this.gen !== gen) return;
      const buf = ctx.createBuffer(chunk.channels, chunk.frames, chunk.sampleRate);
      for (let c = 0; c < chunk.channels; c++) {
        const out = buf.getChannelData(c);
        for (let i = 0; i < chunk.frames; i++) out[i] = chunk.samples[i * chunk.channels + c] ?? 0;
      }
      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.connect(ctx.destination);
      if (this.origin === null) {
        this.nextCtxTime = ctx.currentTime + START_CUSHION;
        this.origin = { ticks: this.nextTicks, ctxTime: this.nextCtxTime };
      }
      src.start(this.nextCtxTime);
      this.sources.push(src);
      if (this.sources.length > 12) this.sources.shift();
      this.nextCtxTime += chunk.frames / chunk.sampleRate;
      this.nextTicks += Math.round((chunk.frames / chunk.sampleRate) * TICKS_PER_SECOND);
    }
  }
}

/** Decodifica a resposta binária do engine (`application/x-f32le`). */
export function pcmFromReply(bytes: ArrayBuffer, meta: Record<string, unknown>): PcmChunk {
  return {
    sampleRate: Number(meta.sample_rate),
    channels: Number(meta.channels),
    frames: Number(meta.frames),
    samples: new Float32Array(bytes),
  };
}

export function webAudioContext(): AudioCtxLike | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as { AudioContext?: new () => AudioCtxLike };
  try {
    return w.AudioContext ? new w.AudioContext() : null;
  } catch {
    return null;
  }
}
