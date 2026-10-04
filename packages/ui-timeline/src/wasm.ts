/**
 * Ponte com o core em WASM (`capia-timeline-wasm`, ADR-070). O módulo guarda uma réplica da
 * sequence ativa; os mesmos patches que o engine devolve a mantêm em dia. O ghost de arrastos usa
 * `resolve_snap`/`resolve_point_snap`/`resolve_group_move`/`resolve_placement` do core — nenhuma
 * matemática de snap é reimplementada em JS.
 */
import type { PatchOp, SequenceModel, Ticks } from "@capia/engine-bindings";

const OP = {
  LOAD: 1,
  PATCH: 2,
  THRESHOLD: 3,
  SNAP_CLIP: 4,
  SNAP_POINT: 5,
  GROUP_MOVE: 6,
  PLACEMENT: 7,
  FRAME_ALIGN: 8,
} as const;

interface Exports {
  memory: WebAssembly.Memory;
  capia_alloc(len: number): number;
  capia_free(ptr: number, len: number): void;
  capia_call(op: number, ptr: number, len: number): bigint;
}

export class CoreError extends Error {
  readonly code: string;
  readonly hint: unknown;
  constructor(code: string, message: string, hint?: unknown) {
    super(message);
    this.name = "CoreError";
    this.code = code;
    this.hint = hint;
  }
}

export interface SnapTarget {
  type: "playhead" | "marker" | "clip_start" | "clip_end" | "sequence_start";
  t: Ticks;
}

export type PlacementStrategy =
  | { type: "explicit"; track: string }
  | { type: "first_available" }
  | { type: "prefer"; track: string };

export type Placement =
  | { kind: "existing"; track: string }
  | {
      kind: "new_track";
      track_kind: "visual" | "audio";
      insert: "above" | "below";
      relative_to: string | null;
    };

export class TimelineCore {
  private constructor(private readonly x: Exports) {}

  /** Instancia o módulo a partir dos bytes (Node/testes) ou de uma resposta HTTP (navegador). */
  static async load(source: BufferSource | Response | Promise<Response>): Promise<TimelineCore> {
    const resolved = await source;
    let instance: WebAssembly.Instance;
    if (resolved instanceof Response) {
      if (typeof WebAssembly.instantiateStreaming === "function") {
        try {
          instance = (await WebAssembly.instantiateStreaming(resolved.clone(), {})).instance;
        } catch {
          instance = (await WebAssembly.instantiate(await resolved.arrayBuffer(), {})).instance;
        }
      } else {
        instance = (await WebAssembly.instantiate(await resolved.arrayBuffer(), {})).instance;
      }
    } else {
      instance = (await WebAssembly.instantiate(resolved, {})).instance;
    }
    return new TimelineCore(instance.exports as unknown as Exports);
  }

  // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- o chamador declara o formato da resposta
  private call<T>(op: number, input: unknown): T {
    const x = this.x;
    const bytes = new TextEncoder().encode(JSON.stringify(input));
    const inPtr = x.capia_alloc(bytes.length);
    new Uint8Array(x.memory.buffer, inPtr, bytes.length).set(bytes);
    const packed = x.capia_call(op, inPtr, bytes.length);
    x.capia_free(inPtr, bytes.length);
    const outPtr = Number(packed >> 32n);
    const outLen = Number(packed & 0xffff_ffffn);
    // copia antes de liberar (a memória do módulo pode crescer/realocar)
    const text = new TextDecoder().decode(new Uint8Array(x.memory.buffer, outPtr, outLen).slice());
    x.capia_free(outPtr, outLen);
    const value = JSON.parse(text) as {
      error?: { code: string; message: string; hint?: unknown };
    } & T;
    if (value.error) throw new CoreError(value.error.code, value.error.message, value.error.hint);
    return value;
  }

  /** Carrega a réplica da sequence (JSON do `sequence.get`). */
  loadSequence(id: string, model: SequenceModel): void {
    this.call(OP.LOAD, { id, sequence: model });
  }

  /** Aplica os patches do engine; `PATCH_MISMATCH` = réplica divergente (recarregue). */
  applyPatches(patches: PatchOp[]): void {
    this.call(OP.PATCH, { patches });
  }

  /** `px` na tela → ticks no zoom atual (`pxPerSecond`), exato (racional em milésimos). */
  threshold(px: number, pxPerSecond: number): Ticks {
    return this.call<{ ticks: Ticks }>(OP.THRESHOLD, {
      px_milli: Math.round(px * 1000),
      pps_milli: Math.max(1, Math.round(pxPerSecond * 1000)),
    }).ticks;
  }

  snapClip(
    clip: string,
    proposedStart: Ticks,
    threshold: Ticks,
    playhead: Ticks | null,
    enabled: boolean,
  ) {
    return this.call<{ start: Ticks; snapped_to: SnapTarget | null }>(OP.SNAP_CLIP, {
      clip,
      proposed_start: proposedStart,
      threshold,
      playhead,
      enabled,
    });
  }

  snapPoint(
    t: Ticks,
    threshold: Ticks,
    exclude: string[],
    playhead: Ticks | null,
  ): SnapTarget | null {
    return this.call<{ target: SnapTarget | null }>(OP.SNAP_POINT, {
      t,
      threshold,
      exclude,
      playhead,
    }).target;
  }

  groupMove(
    members: string[],
    deltaTime: Ticks,
    deltaTracks: number,
    snap: { threshold: Ticks; playhead: Ticks | null } | null,
  ) {
    return this.call<{ delta_time: Ticks; delta_tracks: number }>(OP.GROUP_MOVE, {
      members,
      delta_time: deltaTime,
      delta_tracks: deltaTracks,
      snap,
    });
  }

  placement(
    strategy: PlacementStrategy,
    kind: "visual" | "audio",
    spans: [Ticks, Ticks][],
  ): Placement {
    return this.call<Placement>(OP.PLACEMENT, { strategy, kind, spans });
  }

  alignToFrame(t: Ticks): Ticks {
    return this.call<{ t: Ticks }>(OP.FRAME_ALIGN, { t }).t;
  }
}
