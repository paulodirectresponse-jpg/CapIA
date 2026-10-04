/**
 * `TimelineView`: renderer em **canvas** virtualizado (sem DOM por clip) + camada de interação por
 * gestos. Só desenha o que está na janela de tempo/linhas visível; os arrastos calculam o *ghost*
 * localmente com o core em WASM (snap/grupo/colocação) e o documento só muda no *drop*, por
 * callbacks que viram **um** comando no controlador (ARCHITECTURE §7). Nada aqui escreve no
 * documento.
 */
import {
  TICKS_PER_SECOND,
  type Clip,
  type SequenceModel,
  type Ticks,
} from "@capia/engine-bindings";
import { clipAssetId, clipKind, clipOffline, buildData, type TimelineData } from "./data";
import {
  RULER_H,
  clampPps,
  clipAt,
  pxToTicks,
  rowAt,
  rulerStepSeconds,
  ticksToPx,
  visibleClips,
  type Row,
} from "./layout";
import type { AssetRow } from "@capia/engine-bindings";
import { readPalette, roleColor, type Palette } from "./palette";
import type { TimelineCore } from "./wasm";

export interface MoveItem {
  clip: string;
  track: string;
  start: Ticks;
}

export interface MovePlan {
  moves: MoveItem[];
  /** Reorder de/para track magnética (um clip). */
  reorder: { clip: string; track: string; before: string | null; start: Ticks | null } | null;
  duplicate: boolean;
}

export type DropTarget =
  | { kind: "track"; track: string; time: Ticks }
  | { kind: "new-track"; side: "above" | "below"; time: Ticks };

export interface TimelineCallbacks {
  onScrub(t: Ticks): void;
  onSelect(ids: string[], mode: "replace" | "add" | "toggle"): void;
  onMove(plan: MovePlan): void;
  onTrim(clip: string, edge: "in" | "out", to: Ticks): void;
  onOpenNested(clip: string): void;
  onContextMenu(
    target: { clip: string | null; track: string | null; time: Ticks },
    x: number,
    y: number,
  ): void;
  onDropAsset(asset: string, target: DropTarget): void;
  onDropSequence(sequence: string, target: DropTarget): void;
  onViewport(v: { pps: number; origin: Ticks; scrollY: number }): void;
  onKeyframeMove?(clip: string, prop: string, from: Ticks, to: Ticks): void;
  onBladeCut?(clip: string, at: Ticks): void;
}

export interface ViewProviders {
  /** Pares (min,max) em [-1,1] cobrindo o clip em `buckets` colunas, ou `null` se ainda indisponível. */
  peaks(asset: string, buckets: number): Float32Array | null;
  /** Miniatura do asset (ou `null` se ainda indisponível). */
  thumbnail(asset: string): CanvasImageSource | null;
  formatTime(t: Ticks): string;
  /** Duração (ticks) do item sendo arrastado da biblioteca/projeto, para o preview do drop. */
  dragSpan(): Ticks | null;
}

export interface ViewState {
  pps: number;
  origin: Ticks;
  scrollY: number;
  playhead: Ticks;
  selection: ReadonlySet<string>;
  snapping: boolean;
  tool: "select" | "blade";
  markers: { id: string; time: Ticks; label: string }[];
  /** Clip ativo no inspector (mostra keyframes). */
  focusClip: string | null;
}

export interface ViewStats {
  fps: number;
  paintMs: number;
  visibleClips: number;
  frames: number;
}

type Gesture =
  | { type: "scrub" }
  | { type: "marquee"; x0: number; y0: number; x1: number; y1: number; additive: boolean }
  | {
      type: "move";
      primary: string;
      members: string[];
      x0: number;
      y0: number;
      moved: boolean;
      duplicate: boolean;
      dt: Ticks;
      dTracks: number;
      destTrack: string | null;
      reorderBefore: string | null | undefined;
      snapAt: Ticks | null;
    }
  | {
      type: "trim";
      clip: string;
      edge: "in" | "out";
      to: Ticks;
      x0: number;
      moved: boolean;
      snapAt: Ticks | null;
    };

const EDGE_PX = 6;
const DRAG_THRESHOLD_PX = 3;
const SNAP_PX = 8;

export class TimelineView {
  readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private data: TimelineData | null = null;
  private state: ViewState = {
    pps: 80,
    origin: 0,
    scrollY: 0,
    playhead: 0,
    selection: new Set(),
    snapping: true,
    tool: "select",
    markers: [],
    focusClip: null,
  };
  private palette: Palette;
  private width = 0;
  private height = 0;
  private dpr = 1;
  private dirty = true;
  private raf = 0;
  private disposed = false;
  private gesture: Gesture | null = null;
  private hoverCursor = "default";
  private dropPreview: { target: DropTarget; span: Ticks } | null = null;
  private frameTimes: number[] = [];
  readonly stats: ViewStats = { fps: 0, paintMs: 0, visibleClips: 0, frames: 0 };
  private readonly cleanup: (() => void)[] = [];

  constructor(
    canvas: HTMLCanvasElement,
    private readonly cb: TimelineCallbacks,
    private readonly providers: ViewProviders,
    private readonly core: () => TimelineCore | null,
  ) {
    this.canvas = canvas;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("2D canvas is not available");
    this.ctx = ctx;
    this.palette = readPalette(canvas);
    this.bind();
    this.loop();
  }

  // ------------------------------------------------------------------------------- estado

  setSequence(
    seq: SequenceModel | null,
    assets: Record<string, AssetRow>,
    heights: Record<string, number>,
  ) {
    this.data = seq ? buildData(seq, assets, heights) : null;
    this.invalidate();
  }

  setState(patch: Partial<ViewState>) {
    this.state = { ...this.state, ...patch, pps: clampPps(patch.pps ?? this.state.pps) };
    this.invalidate();
  }

  get viewState(): Readonly<ViewState> {
    return this.state;
  }

  get contentHeight(): number {
    return this.data?.totalHeight ?? 0;
  }

  get contentDuration(): Ticks {
    return this.data?.duration ?? 0;
  }

  resize(width: number, height: number, dpr: number) {
    this.width = Math.max(1, Math.floor(width));
    this.height = Math.max(1, Math.floor(height));
    this.dpr = dpr || 1;
    this.canvas.width = Math.floor(this.width * this.dpr);
    this.canvas.height = Math.floor(this.height * this.dpr);
    this.canvas.style.width = `${String(this.width)}px`;
    this.canvas.style.height = `${String(this.height)}px`;
    this.invalidate();
  }

  refreshPalette() {
    this.palette = readPalette(this.canvas);
    this.invalidate();
  }

  invalidate() {
    this.dirty = true;
  }

  dispose() {
    this.disposed = true;
    cancelAnimationFrame(this.raf);
    for (const f of this.cleanup) f();
    this.cleanup.length = 0;
  }

  /** Retângulo (px do canvas) de um clip — usado por testes E2E e pelo foco programático. */
  clipRect(id: string): { x: number; y: number; w: number; h: number } | null {
    const d = this.data;
    const c = d?.seq.clips[id];
    if (!d || !c) return null;
    const row = d.rowOf.get(c.track);
    if (!row) return null;
    const { pps, origin, scrollY } = this.state;
    return {
      x: ticksToPx(c.start, pps, origin),
      y: RULER_H + row.y - scrollY + 2,
      w: (c.duration / TICKS_PER_SECOND) * pps,
      h: row.h - 4,
    };
  }

  rowRect(trackId: string): { y: number; h: number } | null {
    const row = this.data?.rowOf.get(trackId);
    return row ? { y: RULER_H + row.y - this.state.scrollY, h: row.h } : null;
  }

  showDropPreview(p: { target: DropTarget; span: Ticks } | null) {
    this.dropPreview = p;
    this.invalidate();
  }

  // ---------------------------------------------------------------------------- loop/pintura

  private loop = () => {
    if (this.disposed) return;
    if (this.dirty) {
      this.dirty = false;
      const t0 = performance.now();
      this.paint();
      const dt = performance.now() - t0;
      this.stats.paintMs = dt;
      this.stats.frames++;
      const now = performance.now();
      this.frameTimes.push(now);
      while (this.frameTimes.length > 0 && now - (this.frameTimes[0] ?? now) > 1000)
        this.frameTimes.shift();
      this.stats.fps = this.frameTimes.length;
    }
    this.raf = requestAnimationFrame(this.loop);
  };

  private paint() {
    const { ctx, palette: p, width, height } = this;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.fillStyle = p.bg;
    ctx.fillRect(0, 0, width, height);
    ctx.font = p.font;
    ctx.textBaseline = "middle";
    const d = this.data;
    this.stats.visibleClips = 0;
    if (d) {
      this.paintRows(d);
      this.paintClips(d);
      this.paintMarkers();
      this.paintOverlays(d);
    }
    this.paintRuler();
    this.paintPlayhead();
  }

  private tx(t: Ticks): number {
    return ticksToPx(t, this.state.pps, this.state.origin);
  }

  private visibleRange(): [Ticks, Ticks] {
    const { pps, origin } = this.state;
    return [
      origin - TICKS_PER_SECOND,
      origin + (this.width / pps) * TICKS_PER_SECOND + TICKS_PER_SECOND,
    ];
  }

  private paintRows(d: TimelineData) {
    const { ctx, palette: p, width, height } = this;
    const { scrollY } = this.state;
    for (const row of d.rows) {
      const y = RULER_H + row.y - scrollY;
      if (y > height || y + row.h < RULER_H) continue;
      ctx.fillStyle = row.displayIndex % 2 === 0 ? p.rowA : p.rowB;
      ctx.fillRect(0, y, width, row.h);
      ctx.fillStyle = p.rowLine;
      ctx.fillRect(0, y + row.h - 1, width, 1);
      if (row.track.hidden || row.track.locked) {
        ctx.fillStyle = "rgba(0,0,0,0.22)";
        ctx.fillRect(0, y, width, row.h - 1);
      }
    }
  }

  private paintClips(d: TimelineData) {
    const { ctx, height } = this;
    const { scrollY, selection } = this.state;
    const [from, to] = this.visibleRange();
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, RULER_H, this.width, height - RULER_H);
    ctx.clip();
    for (const row of d.rows) {
      const y = RULER_H + row.y - scrollY;
      if (y > height || y + row.h < RULER_H) continue;
      const list = d.byTrack.get(row.track.id) ?? [];
      for (const c of visibleClips(list, from, to)) {
        this.stats.visibleClips++;
        this.paintClip(d, row, c, y, selection.has(c.id));
      }
    }
    ctx.restore();
  }

  private paintClip(d: TimelineData, row: Row, c: Clip, rowY: number, selected: boolean) {
    const { ctx, palette: p } = this;
    const x0 = this.tx(c.start);
    const x1 = this.tx(c.start + c.duration);
    const w = Math.max(1, x1 - x0);
    const y = rowY + 2;
    const h = row.h - 4;
    const kind = clipKind(c, row.track);
    const color = kind === "nested" ? (p.role.nested ?? "#c8895a") : roleColor(p, row.track.role);
    const offline = clipOffline(c, d.assets);
    ctx.globalAlpha = c.enabled && !row.track.hidden ? 1 : 0.45;
    // corpo
    ctx.fillStyle = withAlpha(color, kind === "audio" ? 0.28 : 0.4);
    roundRect(ctx, x0, y, w, h, 4);
    ctx.fill();
    // conteúdo
    ctx.save();
    ctx.beginPath();
    roundRect(ctx, x0, y, w, h, 4);
    ctx.clip();
    const asset = clipAssetId(c);
    if (w > 8) {
      if ((kind === "video" || kind === "image") && asset) {
        const img = this.providers.thumbnail(asset);
        const th = h - 16;
        if (img && th > 8) {
          const tileW = Math.max(24, Math.round(th * 1.6));
          const first = Math.max(0, Math.floor((0 - x0) / tileW));
          for (let tx = x0 + first * tileW; tx < Math.min(x1, this.width); tx += tileW) {
            ctx.drawImage(img, tx, y + 15, tileW, th);
          }
          ctx.fillStyle = "rgba(0,0,0,0.18)";
          ctx.fillRect(x0, y + 15, w, th);
        }
      } else if (kind === "audio" && asset) {
        this.paintWaveform(asset, x0, y, w, h, color);
      }
      if (kind === "text" || kind === "caption") {
        const label = c.content.type === "text" ? c.content.text : "";
        ctx.fillStyle = p.text;
        ctx.fillText(ellipsize(ctx, label, w - 10), Math.max(x0, 0) + 6, y + h / 2 + 6);
      }
      if (kind === "nested") {
        ctx.fillStyle = "rgba(255,255,255,0.08)";
        for (let sx = x0; sx < x1; sx += 10) ctx.fillRect(sx, y + 15, 4, h - 15);
      }
    }
    // barra de título
    ctx.fillStyle = withAlpha(color, 0.85);
    ctx.fillRect(x0, y, w, 14);
    ctx.fillStyle = "#0b0d12";
    const title = c.name || defaultName(c);
    const labelX = Math.max(x0, 0) + 5;
    ctx.fillText(ellipsize(ctx, title, x1 - labelX - 4), labelX, y + 7.5);
    // transição de entrada
    if (c.transition_in) {
      const tw = Math.max(6, (c.transition_in.duration / TICKS_PER_SECOND) * this.state.pps);
      const center = c.transition_in.kind === "dissolve" ? x0 : x0 + tw / 2;
      ctx.fillStyle = "rgba(255,255,255,0.5)";
      ctx.beginPath();
      ctx.moveTo(center - tw / 2, y + h);
      ctx.lineTo(center + tw / 2, y + h);
      ctx.lineTo(center, y + h - 12);
      ctx.closePath();
      ctx.fill();
    }
    ctx.restore();
    // offline
    if (offline) {
      ctx.save();
      ctx.beginPath();
      roundRect(ctx, x0, y, w, h, 4);
      ctx.clip();
      ctx.strokeStyle = withAlpha(p.danger, 0.55);
      ctx.lineWidth = 2;
      for (let sx = x0 - h; sx < x1; sx += 12) {
        ctx.beginPath();
        ctx.moveTo(sx, y + h);
        ctx.lineTo(sx + h, y);
        ctx.stroke();
      }
      ctx.restore();
    }
    // contorno
    ctx.lineWidth = selected ? 2 : 1;
    ctx.strokeStyle = selected ? "#ffffff" : withAlpha(color, 0.9);
    roundRect(ctx, x0 + 0.5, y + 0.5, w - 1, h - 1, 4);
    ctx.stroke();
    if (c.group) {
      ctx.fillStyle = p.accent;
      ctx.fillRect(x0 + 1, y + h - 4, Math.max(2, w - 2), 3);
    }
    // keyframes do clip em foco
    if (this.state.focusClip === c.id) this.paintKeyframes(c, y + h - 9);
    ctx.globalAlpha = 1;
  }

  private paintWaveform(asset: string, x0: number, y: number, w: number, h: number, color: string) {
    const { ctx } = this;
    const visL = Math.max(0, x0);
    const visR = Math.min(this.width, x0 + w);
    const cols = Math.max(1, Math.floor(visR - visL));
    // picos do trecho total do clip em `w` colunas (limitado); recorta a parte visível
    const buckets = Math.min(4096, Math.max(8, Math.floor(w)));
    const peaks = this.providers.peaks(asset, buckets);
    const mid = y + 15 + (h - 15) / 2;
    const amp = (h - 18) / 2;
    ctx.strokeStyle = withAlpha(color, 0.95);
    ctx.lineWidth = 1;
    ctx.beginPath();
    if (!peaks) {
      ctx.moveTo(visL, mid);
      ctx.lineTo(visR, mid);
    } else {
      const n = peaks.length / 2;
      for (let i = 0; i < cols; i++) {
        const frac = (visL + i - x0) / w;
        const k = Math.min(n - 1, Math.max(0, Math.floor(frac * n)));
        const mn = peaks[k * 2] ?? 0;
        const mx = peaks[k * 2 + 1] ?? 0;
        ctx.moveTo(visL + i + 0.5, mid - mx * amp);
        ctx.lineTo(visL + i + 0.5, mid - mn * amp + 1);
      }
    }
    ctx.stroke();
  }

  private paintKeyframes(c: Clip, y: number) {
    const { ctx } = this;
    ctx.fillStyle = "#ffd166";
    const seen = new Set<number>();
    for (const a of Object.values(c.properties)) {
      if (!("animated" in a)) continue;
      for (const k of a.animated) {
        const t = c.start + (k.time - c.source_in) / (parseSpeed(c.speed) || 1);
        const x = this.tx(t);
        const key = Math.round(x);
        if (seen.has(key) || x < -6 || x > this.width + 6) continue;
        seen.add(key);
        ctx.beginPath();
        ctx.moveTo(x, y - 4);
        ctx.lineTo(x + 4, y);
        ctx.lineTo(x, y + 4);
        ctx.lineTo(x - 4, y);
        ctx.closePath();
        ctx.fill();
      }
    }
  }

  private paintMarkers() {
    const { ctx, palette: p } = this;
    ctx.fillStyle = p.snap;
    for (const m of this.state.markers) {
      const x = this.tx(m.time);
      if (x < -8 || x > this.width + 8) continue;
      ctx.beginPath();
      ctx.moveTo(x - 5, 0);
      ctx.lineTo(x + 5, 0);
      ctx.lineTo(x, 9);
      ctx.closePath();
      ctx.fill();
      ctx.fillRect(x, 9, 1, this.height);
    }
  }

  private paintRuler() {
    const { ctx, palette: p, width } = this;
    const { pps, origin } = this.state;
    ctx.fillStyle = p.ruler;
    ctx.fillRect(0, 0, width, RULER_H);
    ctx.fillStyle = p.rowLine;
    ctx.fillRect(0, RULER_H - 1, width, 1);
    const step = rulerStepSeconds(pps);
    const stepTicks = step * TICKS_PER_SECOND;
    const first = Math.floor(origin / stepTicks) * stepTicks;
    const endT = origin + (width / pps) * TICKS_PER_SECOND;
    ctx.fillStyle = p.rulerText;
    for (let t = first; t <= endT; t += stepTicks) {
      const x = this.tx(t);
      ctx.fillStyle = p.rulerTick;
      ctx.fillRect(Math.round(x), RULER_H - 9, 1, 9);
      // subdivisões
      const sub = stepTicks / 5;
      for (let k = 1; k < 5; k++) {
        ctx.fillRect(Math.round(this.tx(t + k * sub)), RULER_H - 4, 1, 4);
      }
      ctx.fillStyle = p.rulerText;
      ctx.fillText(this.providers.formatTime(Math.max(0, Math.round(t))), x + 4, 9);
    }
  }

  private paintPlayhead() {
    const { ctx, palette: p } = this;
    const x = this.tx(this.state.playhead);
    if (x < -8 || x > this.width + 8) return;
    ctx.fillStyle = p.playhead;
    ctx.fillRect(Math.round(x), 0, 1.5, this.height);
    ctx.beginPath();
    ctx.moveTo(x - 6, 0);
    ctx.lineTo(x + 7, 0);
    ctx.lineTo(x + 0.5, 11);
    ctx.closePath();
    ctx.fill();
  }

  private paintOverlays(d: TimelineData) {
    const { ctx, palette: p } = this;
    const g = this.gesture;
    const { scrollY } = this.state;
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, RULER_H, this.width, this.height - RULER_H);
    ctx.clip();
    if (g?.type === "move" && g.moved) {
      const primary = d.seq.clips[g.primary];
      for (const id of g.members) {
        const c = d.seq.clips[id];
        if (!c) continue;
        const destRow = this.destRowFor(d, c, g.dTracks);
        if (!destRow) continue;
        const x = this.tx(c.start + g.dt);
        const w = (c.duration / TICKS_PER_SECOND) * this.state.pps;
        const y = RULER_H + destRow.y - scrollY + 2;
        ctx.fillStyle = "rgba(255,255,255,0.18)";
        ctx.strokeStyle = "#ffffff";
        ctx.setLineDash([5, 3]);
        ctx.lineWidth = 1.5;
        roundRect(ctx, x, y, w, destRow.h - 4, 4);
        ctx.fill();
        ctx.stroke();
        ctx.setLineDash([]);
      }
      if (g.reorderBefore !== undefined && primary) {
        const row = g.destTrack ? d.rowOf.get(g.destTrack) : undefined;
        if (row) {
          const before = g.reorderBefore ? d.seq.clips[g.reorderBefore] : undefined;
          const atT = before
            ? before.start
            : (d.byTrack.get(row.track.id) ?? []).reduce(
                (m, c) => Math.max(m, c.start + c.duration),
                0,
              );
          const x = this.tx(atT);
          ctx.fillStyle = p.accent;
          ctx.fillRect(x - 1.5, RULER_H + row.y - scrollY, 3, row.h);
        }
      }
      if (g.snapAt !== null) this.paintSnapLine(g.snapAt);
    } else if (g?.type === "trim" && g.moved) {
      const c = d.seq.clips[g.clip];
      const row = c ? d.rowOf.get(c.track) : undefined;
      if (c && row) {
        const x0 = g.edge === "in" ? this.tx(g.to) : this.tx(c.start);
        const x1 = g.edge === "out" ? this.tx(g.to) : this.tx(c.start + c.duration);
        ctx.fillStyle = "rgba(255,255,255,0.18)";
        ctx.strokeStyle = "#ffffff";
        ctx.setLineDash([5, 3]);
        roundRect(ctx, x0, RULER_H + row.y - scrollY + 2, Math.max(1, x1 - x0), row.h - 4, 4);
        ctx.fill();
        ctx.stroke();
        ctx.setLineDash([]);
        if (g.snapAt !== null) this.paintSnapLine(g.snapAt);
      }
    } else if (g?.type === "marquee") {
      const x = Math.min(g.x0, g.x1);
      const y = Math.min(g.y0, g.y1);
      ctx.fillStyle = p.accentSoft;
      ctx.strokeStyle = p.accent;
      ctx.lineWidth = 1;
      ctx.fillRect(x, y, Math.abs(g.x1 - g.x0), Math.abs(g.y1 - g.y0));
      ctx.strokeRect(x + 0.5, y + 0.5, Math.abs(g.x1 - g.x0), Math.abs(g.y1 - g.y0));
    }
    const dp = this.dropPreview;
    if (dp) {
      const x = this.tx(dp.target.time);
      const w = Math.max(6, (dp.span / TICKS_PER_SECOND) * this.state.pps);
      if (dp.target.kind === "track") {
        const row = d.rowOf.get(dp.target.track);
        if (row) {
          ctx.fillStyle = p.accentSoft;
          ctx.strokeStyle = p.accent;
          ctx.lineWidth = 2;
          roundRect(ctx, x, RULER_H + row.y - scrollY + 2, w, row.h - 4, 4);
          ctx.fill();
          ctx.stroke();
        }
      } else {
        const y = dp.target.side === "above" ? RULER_H : RULER_H + d.totalHeight - scrollY;
        ctx.fillStyle = p.accent;
        ctx.fillRect(0, y - 1.5, this.width, 3);
        ctx.fillStyle = p.accentSoft;
        ctx.fillRect(x, y - (dp.target.side === "above" ? 0 : 36), w, 36);
      }
    }
    ctx.restore();
  }

  private paintSnapLine(t: Ticks) {
    const { ctx, palette: p } = this;
    const x = Math.round(this.tx(t));
    ctx.fillStyle = p.snap;
    ctx.fillRect(x, RULER_H, 1.5, this.height);
  }

  // ------------------------------------------------------------------------------ interação

  private bind() {
    const c = this.canvas;
    const on = <K extends keyof HTMLElementEventMap>(
      type: K,
      fn: (e: HTMLElementEventMap[K]) => void,
      opts?: AddEventListenerOptions,
    ) => {
      c.addEventListener(type, fn as EventListener, opts);
      this.cleanup.push(() => {
        c.removeEventListener(type, fn as EventListener, opts);
      });
    };
    on("pointerdown", (e) => {
      this.pointerDown(e);
    });
    on("pointermove", (e) => {
      this.pointerMove(e);
    });
    on("pointerup", (e) => {
      this.pointerUp(e);
    });
    on("pointercancel", () => {
      this.cancelGesture();
    });
    on("dblclick", (e) => {
      const hit = this.hitClip(this.local(e));
      if (hit?.clip.content.type === "nested") this.cb.onOpenNested(hit.clip.id);
    });
    on("contextmenu", (e) => {
      e.preventDefault();
      const { x, y } = this.local(e);
      const hit = this.hitClip({ x, y });
      const row = this.rowAtY(y);
      this.cb.onContextMenu(
        {
          clip: hit?.clip.id ?? null,
          track: row?.track.id ?? null,
          time: pxToTicks(x, this.state.pps, this.state.origin),
        },
        e.clientX,
        e.clientY,
      );
    });
    on(
      "wheel",
      (e) => {
        e.preventDefault();
        this.wheel(e);
      },
      { passive: false },
    );
    const dnd = "application/x-capia-asset";
    const dndSeq = "application/x-capia-sequence-nested";
    on("dragover", (e) => {
      const types = e.dataTransfer?.types;
      if (types?.includes(dnd) || types?.includes(dndSeq)) {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
        const { x, y } = this.local(e);
        this.showDropPreview({
          target: this.dropTargetAt(x, y),
          span: this.providers.dragSpan() ?? TICKS_PER_SECOND * 3,
        });
      }
    });
    on("drop", (e) => {
      const asset = e.dataTransfer?.getData(dnd);
      const seq = e.dataTransfer?.getData(dndSeq);
      if (!asset && !seq) return;
      e.preventDefault();
      const { x, y } = this.local(e);
      const target = this.dropTargetAt(x, y);
      if (asset) this.cb.onDropAsset(asset, target);
      else if (seq) this.cb.onDropSequence(seq, target);
      this.showDropPreview(null);
    });
    on("dragleave", () => {
      this.showDropPreview(null);
    });
  }

  private local(e: { clientX: number; clientY: number }): { x: number; y: number } {
    const r = this.canvas.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  private contentY(y: number): number {
    return y - RULER_H + this.state.scrollY;
  }

  private rowAtY(y: number): Row | null {
    const d = this.data;
    if (!d || y < RULER_H) return null;
    return rowAt(d.rows, this.contentY(y));
  }

  /** Destino de um drop de mídia na posição do ponteiro (linha existente ou nova track). */
  dropTargetAt(x: number, y: number): DropTarget {
    const d = this.data;
    const time = pxToTicks(Math.max(0, x), this.state.pps, this.state.origin);
    const t = Math.max(0, time);
    if (!d || d.rows.length === 0) return { kind: "new-track", side: "above", time: t };
    const cy = this.contentY(y);
    if (cy < 0) return { kind: "new-track", side: "above", time: t };
    if (cy >= d.totalHeight) return { kind: "new-track", side: "below", time: t };
    const row = rowAt(d.rows, cy);
    return row
      ? { kind: "track", track: row.track.id, time: t }
      : { kind: "new-track", side: "below", time: t };
  }

  private hitClip(p: {
    x: number;
    y: number;
  }): { clip: Clip; row: Row; edge: "in" | "out" | null } | null {
    const d = this.data;
    if (!d || p.y < RULER_H) return null;
    const row = rowAt(d.rows, this.contentY(p.y));
    if (!row) return null;
    const t = pxToTicks(p.x, this.state.pps, this.state.origin);
    const list = d.byTrack.get(row.track.id) ?? [];
    // tolerância de borda: procura também o clip vizinho dentro de EDGE_PX
    const slack = Math.round((EDGE_PX / this.state.pps) * TICKS_PER_SECOND);
    const c = clipAt(list, t) ?? clipAt(list, t + slack) ?? clipAt(list, t - slack);
    if (!c) return null;
    const x0 = this.tx(c.start);
    const x1 = this.tx(c.start + c.duration);
    const wide = x1 - x0 >= EDGE_PX * 3;
    let edge: "in" | "out" | null = null;
    if (wide && Math.abs(p.x - x0) <= EDGE_PX) edge = "in";
    else if (wide && Math.abs(p.x - x1) <= EDGE_PX) edge = "out";
    if (!edge && (p.x < x0 || p.x > x1)) return null;
    return { clip: c, row, edge };
  }

  private snapThreshold(): Ticks {
    const core = this.core();
    if (core) {
      try {
        return core.threshold(SNAP_PX, this.state.pps);
      } catch {
        /* cai no cálculo local abaixo */
      }
    }
    return Math.round((SNAP_PX / this.state.pps) * TICKS_PER_SECOND);
  }

  private pointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const p = this.local(e);
    this.canvas.setPointerCapture(e.pointerId);
    if (p.y < RULER_H) {
      this.gesture = { type: "scrub" };
      this.scrubTo(p.x);
      return;
    }
    const hit = this.hitClip(p);
    if (!hit) {
      this.gesture = { type: "marquee", x0: p.x, y0: p.y, x1: p.x, y1: p.y, additive: e.shiftKey };
      if (!e.shiftKey) this.cb.onSelect([], "replace");
      return;
    }
    if (this.state.tool === "blade") {
      const t = this.snapTime(pxToTicks(p.x, this.state.pps, this.state.origin), [hit.clip.id]);
      this.cb.onBladeCut?.(hit.clip.id, t);
      return;
    }
    const sel = this.state.selection;
    if (hit.edge) {
      if (!sel.has(hit.clip.id)) this.cb.onSelect([hit.clip.id], "replace");
      const to = hit.edge === "in" ? hit.clip.start : hit.clip.start + hit.clip.duration;
      this.gesture = {
        type: "trim",
        clip: hit.clip.id,
        edge: hit.edge,
        to,
        x0: p.x,
        moved: false,
        snapAt: null,
      };
      return;
    }
    if (e.shiftKey) {
      this.cb.onSelect([hit.clip.id], "toggle");
    } else if (!sel.has(hit.clip.id)) {
      this.cb.onSelect([hit.clip.id], "replace");
    }
    const base = sel.has(hit.clip.id) && !e.shiftKey ? [...sel] : [hit.clip.id];
    const members = this.expandGroups(base);
    this.gesture = {
      type: "move",
      primary: hit.clip.id,
      members,
      x0: p.x,
      y0: p.y,
      moved: false,
      duplicate: e.altKey,
      dt: 0,
      dTracks: 0,
      destTrack: hit.row.track.id,
      reorderBefore: undefined,
      snapAt: null,
    };
  }

  private expandGroups(ids: string[]): string[] {
    const d = this.data;
    if (!d) return ids;
    const groups = new Set<string>();
    for (const id of ids) {
      const g = d.seq.clips[id]?.group;
      if (g) groups.add(g);
    }
    if (groups.size === 0) return ids;
    const out = new Set(ids);
    for (const c of Object.values(d.seq.clips)) if (c.group && groups.has(c.group)) out.add(c.id);
    return [...out];
  }

  private snapTime(t: Ticks, exclude: string[]): Ticks {
    const core = this.core();
    if (!core || !this.state.snapping) return t;
    try {
      const hit = core.snapPoint(t, this.snapThreshold(), exclude, this.state.playhead);
      return hit ? hit.t : t;
    } catch {
      return t;
    }
  }

  private scrubTo(x: number) {
    let t = Math.max(0, pxToTicks(x, this.state.pps, this.state.origin));
    const core = this.core();
    if (core) {
      try {
        t = core.alignToFrame(t);
      } catch {
        /* sem alinhamento */
      }
    }
    this.cb.onScrub(t);
  }

  private pointerMove(e: PointerEvent) {
    const p = this.local(e);
    const g = this.gesture;
    if (!g) {
      this.updateHover(p);
      return;
    }
    if (g.type === "scrub") this.scrubTo(p.x);
    else if (g.type === "marquee") {
      g.x1 = p.x;
      g.y1 = p.y;
      this.invalidate();
    } else if (g.type === "trim") this.trimMove(g, p.x);
    else this.moveMove(g, p, e.altKey);
  }

  private updateHover(p: { x: number; y: number }) {
    let cursor = "default";
    if (p.y >= RULER_H) {
      const hit = this.hitClip(p);
      if (hit?.edge) cursor = "ew-resize";
      else if (hit) cursor = this.state.tool === "blade" ? "crosshair" : "grab";
    } else cursor = "col-resize";
    if (cursor !== this.hoverCursor) {
      this.hoverCursor = cursor;
      this.canvas.style.cursor = cursor;
    }
  }

  private moveMove(
    g: Extract<Gesture, { type: "move" }>,
    p: { x: number; y: number },
    alt: boolean,
  ) {
    const d = this.data;
    if (!d) return;
    if (!g.moved && Math.hypot(p.x - g.x0, p.y - g.y0) < DRAG_THRESHOLD_PX) return;
    g.moved = true;
    g.duplicate = alt;
    this.canvas.style.cursor = "grabbing";
    const primary = d.seq.clips[g.primary];
    const srcRow = primary ? d.rowOf.get(primary.track) : undefined;
    if (!primary || !srcRow) return;
    const rawDt = Math.round(((p.x - g.x0) / this.state.pps) * TICKS_PER_SECOND);
    const destRow = this.rowAtY(p.y) ?? srcRow;
    const sameFamily = destRow.track.kind === srcRow.track.kind;
    const effective = sameFamily ? destRow : srcRow;
    g.destTrack = effective.track.id;
    // reorder de/para track magnética (um clip): fronteira mais próxima do ponteiro
    const anyMagnetic = g.members.some((id) => {
      const c = d.seq.clips[id];
      return c ? d.rowOf.get(c.track)?.track.magnetic === true : false;
    });
    if (anyMagnetic || effective.track.magnetic) {
      g.members = [g.primary];
      g.dTracks = 0;
      g.dt = 0;
      g.snapAt = null;
      if (effective.track.magnetic) {
        const t = pxToTicks(p.x, this.state.pps, this.state.origin);
        g.reorderBefore = this.boundaryBefore(d, effective.track.id, t, g.primary);
        const before = g.reorderBefore ? d.seq.clips[g.reorderBefore] : undefined;
        const end = (d.byTrack.get(effective.track.id) ?? [])
          .filter((c) => c.id !== g.primary)
          .reduce((m, c) => Math.max(m, c.start + c.duration), 0);
        g.dt = (before ? before.start : end) - primary.start;
      } else {
        g.reorderBefore = undefined;
        g.dt = rawDt;
      }
      this.invalidate();
      return;
    }
    g.reorderBefore = undefined;
    // índice entre as tracks da mesma família, na ordem do documento
    const fam = d.seq.tracks.filter((t) => t.kind === srcRow.track.kind);
    const si = fam.findIndex((t) => t.id === srcRow.track.id);
    const di = fam.findIndex((t) => t.id === effective.track.id);
    let dTracks = di - si;
    let dt = rawDt;
    const core = this.core();
    g.snapAt = null;
    if (core) {
      try {
        const th = this.snapThreshold();
        const res = core.groupMove(
          g.members,
          dt,
          dTracks,
          this.state.snapping ? { threshold: th, playhead: this.state.playhead } : null,
        );
        dt = res.delta_time;
        dTracks = res.delta_tracks;
        if (this.state.snapping) {
          const s = core.snapClip(g.primary, primary.start + dt, th, this.state.playhead, true);
          if (s.start === primary.start + dt && s.snapped_to) g.snapAt = s.snapped_to.t;
        }
      } catch {
        /* sem snap: usa o delta bruto (o engine valida no commit) */
      }
    }
    g.dt = dt;
    g.dTracks = dTracks;
    this.invalidate();
  }

  /** Clip antes do qual inserir para a posição `t` (null = fim da track magnética). */
  private boundaryBefore(d: TimelineData, track: string, t: Ticks, exclude: string): string | null {
    const list = (d.byTrack.get(track) ?? []).filter((c) => c.id !== exclude);
    for (const c of list) {
      if (t < c.start + c.duration / 2) return c.id;
    }
    return null;
  }

  private destRowFor(d: TimelineData, c: Clip, dTracks: number): Row | undefined {
    const g = this.gesture;
    if (g?.type === "move" && g.reorderBefore !== undefined && g.destTrack)
      return d.rowOf.get(g.destTrack);
    const row = d.rowOf.get(c.track);
    if (!row || dTracks === 0) return row;
    const fam = d.seq.tracks.filter((t) => t.kind === row.track.kind);
    const i = fam.findIndex((t) => t.id === c.track);
    const dest = fam[i + dTracks];
    return dest ? d.rowOf.get(dest.id) : row;
  }

  private trimMove(g: Extract<Gesture, { type: "trim" }>, x: number) {
    const d = this.data;
    const c = d?.seq.clips[g.clip];
    if (!d || !c) return;
    if (!g.moved && Math.abs(x - g.x0) < DRAG_THRESHOLD_PX) return;
    g.moved = true;
    let t = pxToTicks(x, this.state.pps, this.state.origin);
    g.snapAt = null;
    const core = this.core();
    if (core && this.state.snapping) {
      try {
        const hit = core.snapPoint(t, this.snapThreshold(), [c.id], this.state.playhead);
        if (hit) {
          t = hit.t;
          g.snapAt = hit.t;
        }
      } catch {
        /* sem snap */
      }
    }
    if (core) {
      try {
        t = core.alignToFrame(t);
      } catch {
        /* sem alinhamento */
      }
    }
    const frame = d.seq.frame_ticks;
    const end = c.start + c.duration;
    if (g.edge === "in") t = Math.min(t, end - frame);
    else t = Math.max(t, c.start + frame);
    // limites de fonte (mídia finita): não estica além do que a fonte tem
    const speed = parseSpeed(c.speed) || 1;
    const aid = clipAssetId(c);
    const asset = aid ? d.assets[aid] : undefined;
    if (c.content.type === "media" && asset?.duration != null && !c.reversed) {
      if (g.edge === "out") t = Math.min(t, c.start + (asset.duration - c.source_in) / speed);
      else t = Math.max(t, c.start - c.source_in / speed);
    }
    t = Math.max(0, t);
    // vizinhos em track livre: não invade o clip ao lado
    const row = d.rowOf.get(c.track);
    if (row && !row.track.magnetic) {
      const list = d.byTrack.get(c.track) ?? [];
      const i = list.findIndex((k) => k.id === c.id);
      const prev = list[i - 1];
      const next = list[i + 1];
      if (g.edge === "in" && prev) t = Math.max(t, prev.start + prev.duration);
      if (g.edge === "out" && next) t = Math.min(t, next.start);
    }
    g.to = t;
    this.invalidate();
  }

  private pointerUp(e: PointerEvent) {
    const g = this.gesture;
    this.gesture = null;
    this.canvas.style.cursor = this.hoverCursor;
    if (this.canvas.hasPointerCapture(e.pointerId)) this.canvas.releasePointerCapture(e.pointerId);
    if (!g) return;
    const d = this.data;
    if (g.type === "marquee" && d) {
      const [xa, xb] = [Math.min(g.x0, g.x1), Math.max(g.x0, g.x1)];
      const [ya, yb] = [Math.min(g.y0, g.y1), Math.max(g.y0, g.y1)];
      if (xb - xa > 3 || yb - ya > 3) {
        const ids: string[] = [];
        const [from, to] = [
          pxToTicks(xa, this.state.pps, this.state.origin),
          pxToTicks(xb, this.state.pps, this.state.origin),
        ];
        for (const row of d.rows) {
          const top = RULER_H + row.y - this.state.scrollY;
          if (top + row.h < ya || top > yb) continue;
          for (const c of visibleClips(d.byTrack.get(row.track.id) ?? [], from, to)) ids.push(c.id);
        }
        this.cb.onSelect(ids, g.additive ? "add" : "replace");
      }
    } else if (g.type === "trim" && g.moved) {
      const c = d?.seq.clips[g.clip];
      if (c) {
        const cur = g.edge === "in" ? c.start : c.start + c.duration;
        if (g.to !== cur) this.cb.onTrim(g.clip, g.edge, g.to);
      }
    } else if (g.type === "move" && g.moved && d) {
      this.commitMove(g, d);
    }
    this.invalidate();
  }

  private commitMove(g: Extract<Gesture, { type: "move" }>, d: TimelineData) {
    const primary = d.seq.clips[g.primary];
    if (!primary) return;
    if (g.reorderBefore !== undefined && g.destTrack) {
      this.cb.onMove({
        moves: [],
        reorder: { clip: g.primary, track: g.destTrack, before: g.reorderBefore, start: null },
        duplicate: g.duplicate,
      });
      return;
    }
    const srcRow = d.rowOf.get(primary.track);
    const destRow = g.destTrack ? d.rowOf.get(g.destTrack) : srcRow;
    // saída de uma track magnética para uma track livre
    if (srcRow?.track.magnetic && destRow && !destRow.track.magnetic) {
      this.cb.onMove({
        moves: [],
        reorder: {
          clip: g.primary,
          track: destRow.track.id,
          before: null,
          start: Math.max(0, primary.start + g.dt),
        },
        duplicate: g.duplicate,
      });
      return;
    }
    if (g.dt === 0 && g.dTracks === 0 && !g.duplicate) return;
    const moves: MoveItem[] = [];
    for (const id of g.members) {
      const c = d.seq.clips[id];
      if (!c) continue;
      const row = d.rowOf.get(c.track);
      if (!row) continue;
      const fam = d.seq.tracks.filter((t) => t.kind === row.track.kind);
      const i = fam.findIndex((t) => t.id === c.track);
      const dest = fam[i + g.dTracks];
      moves.push({ clip: id, track: (dest ?? row.track).id, start: Math.max(0, c.start + g.dt) });
    }
    this.cb.onMove({ moves, reorder: null, duplicate: g.duplicate });
  }

  /** Cancela o gesto em curso (Esc): nada é enviado ao engine. */
  cancelGesture() {
    if (this.gesture) {
      this.gesture = null;
      this.canvas.style.cursor = this.hoverCursor;
      this.invalidate();
    }
  }

  get gestureActive(): boolean {
    return this.gesture !== null;
  }

  private wheel(e: WheelEvent) {
    const { x } = this.local(e);
    const s = this.state;
    if (e.ctrlKey || e.metaKey) {
      const pps = clampPps(s.pps * Math.exp(-e.deltaY * 0.0018));
      const tCursor = pxToTicks(x, s.pps, s.origin);
      const origin = Math.round(tCursor - (x / pps) * TICKS_PER_SECOND);
      this.cb.onViewport({ pps, origin: Math.max(0, origin), scrollY: s.scrollY });
    } else if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      const dx = e.shiftKey && e.deltaX === 0 ? e.deltaY : e.deltaX;
      const origin = Math.max(0, s.origin + Math.round((dx / s.pps) * TICKS_PER_SECOND));
      this.cb.onViewport({ pps: s.pps, origin, scrollY: s.scrollY });
    } else {
      const max = Math.max(0, (this.data?.totalHeight ?? 0) - (this.height - RULER_H));
      this.cb.onViewport({
        pps: s.pps,
        origin: s.origin,
        scrollY: Math.min(max, Math.max(0, s.scrollY + e.deltaY)),
      });
    }
  }
}

// ------------------------------------------------------------------------------------ utilitários

function parseSpeed(s: string): number {
  const [n, d] = s.split("/");
  const num = Number(n);
  const den = d === undefined ? 1 : Number(d);
  return den > 0 ? num / den : num;
}

function defaultName(c: Clip): string {
  switch (c.content.type) {
    case "text":
      return c.content.text.slice(0, 40);
    case "solid":
      return "Solid";
    case "nested":
      return c.content.sequence;
    default:
      return c.id;
  }
}

function withAlpha(hex: string, a: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return hex;
  const n = parseInt(m[1] ?? "000000", 16);
  return `rgba(${String((n >> 16) & 255)},${String((n >> 8) & 255)},${String(n & 255)},${String(a)})`;
}

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const rr = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function ellipsize(ctx: CanvasRenderingContext2D, text: string, maxW: number): string {
  if (maxW <= 6) return "";
  if (ctx.measureText(text).width <= maxW) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(`${text.slice(0, mid)}…`).width <= maxW) lo = mid;
    else hi = mid - 1;
  }
  return lo > 0 ? `${text.slice(0, lo)}…` : "";
}
