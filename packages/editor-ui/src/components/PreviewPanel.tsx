import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, IconButton, Select } from "@capia/ui-kit";
import type { Clip } from "@capia/engine-bindings";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { formatBinding, resolveBindings } from "../lib/keymap";
import { formatTimecode } from "../lib/timecode";
import { CanvasPresenter } from "../preview/presenter";
import {
  EMPTY_METRICS,
  FrameScheduler,
  pickAutoHeight,
  previewSize,
  type PreviewMetrics,
} from "../preview/scheduler";
import type { PreviewQuality } from "../lib/prefs";

const E2E = typeof location !== "undefined" && new URLSearchParams(location.search).has("e2e");

/** Valor estático (ou o primeiro keyframe) de uma propriedade do clip. */
function propValue(c: Clip, name: string, dflt: number): number {
  const p = c.properties[name];
  if (!p) return dflt;
  if ("static" in p) return p.static;
  return p.animated[0]?.value ?? dflt;
}

type Drag =
  | { kind: "move"; x0: number; y0: number; px: number; py: number }
  | { kind: "scale"; cx: number; cy: number; d0: number; s0: number };

export function PreviewPanel() {
  const c = useController();
  const t = useT();
  const { seqId, summary, playhead, playing, revision, assets, prefs, busy } = useUi((s) => ({
    seqId: s.active,
    summary: s.active ? (s.model.sequences[s.active] ?? null) : null,
    playhead: s.playhead,
    playing: s.playing,
    revision: s.model.revision,
    assets: s.model.assets,
    prefs: s.prefs,
    busy: s.busy,
  }));
  const selectedClip = useUi((s) => {
    const id = s.selection.length === 1 ? s.selection[0] : undefined;
    const seq = s.active ? s.model.models[s.active] : undefined;
    return id && seq ? (seq.clips[id] ?? null) : null;
  });
  const keymap = useMemo(() => resolveBindings(prefs.keymap), [prefs.keymap]);

  const stageRef = useRef<HTMLDivElement | null>(null);
  const frameRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const schedRef = useRef<FrameScheduler | null>(null);
  const [metrics, setMetrics] = useState<PreviewMetrics>(EMPTY_METRICS);
  const [unavailable, setUnavailable] = useState(false);
  const [autoH, setAutoH] = useState<540 | 720>(720);
  const [fullscreen, setFullscreen] = useState(false);
  const [box, setBox] = useState<{ w: number; h: number }>({ w: 0, h: 0 });
  const [drag, setDrag] = useState<{ dx: number; dy: number; k: number } | null>(null);
  const dragRef = useRef<Drag | null>(null);

  const seqW = summary?.width ?? 1920;
  const seqH = summary?.height ?? 1080;
  const frameMs = summary ? summary.frame_ticks / 705_600 : 33;
  const { quality, proxy, safeAreas } = prefs.preview;
  const size = previewSize(seqW, seqH, quality, autoH, proxy);

  // agendador + apresentador (um por canvas; descartados ao desmontar)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const presenter = new CanvasPresenter(canvas);
    const sched = new FrameScheduler(
      (req) => {
        if (!c.state.active) return Promise.reject(new Error("no sequence"));
        return c.frames.render(c.state.active, req);
      },
      presenter,
      (m) => {
        if (m.presented > 0) c.perf.record("preview", m.lastLatencyMs);
        setMetrics(m);
        setUnavailable(m.presented === 0 && m.failed > 0);
        if (E2E)
          (window as unknown as { __capiaPreviewMetrics?: PreviewMetrics }).__capiaPreviewMetrics =
            m;
      },
    );
    schedRef.current = sched;
    return () => {
      schedRef.current = null;
      sched.dispose();
    };
  }, [c]);

  // qualidade automática: ajusta pela latência média (histerese)
  useEffect(() => {
    if (quality !== "auto" || metrics.presented < 4) return;
    const next = pickAutoHeight(autoH, metrics.avgLatencyMs, frameMs);
    // eslint-disable-next-line react-hooks/set-state-in-effect -- reação a métrica medida
    if (next !== autoH) setAutoH(next);
  }, [quality, autoH, metrics.avgLatencyMs, metrics.presented, frameMs]);

  // novo pedido de quadro a cada mudança relevante (playhead, documento, tamanho, assets)
  useEffect(() => {
    if (!seqId || !summary) return;
    // com um comando/undo em andamento o quadro seria descartado de qualquer modo (o documento
    // vai mudar) e disputaria a sessão do engine: pede o quadro quando ficar ocioso
    if (busy > 0) return;
    schedRef.current?.request({ at: playhead, width: size.width, height: size.height });
  }, [seqId, summary, playhead, revision, assets, size.width, size.height, busy]);

  // tamanho do quadro na palco (razão da sequence, cabe no espaço disponível)
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const measure = () => {
      const pad = 16;
      const aw = Math.max(1, el.clientWidth - pad * 2);
      const ah = Math.max(1, el.clientHeight - pad * 2);
      const k = Math.min(aw / seqW, ah / seqH);
      setBox({ w: Math.floor(seqW * k), h: Math.floor(seqH * k) });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => {
      ro.disconnect();
    };
  }, [seqW, seqH, fullscreen]);

  useEffect(() => {
    const on = () => {
      setFullscreen(document.fullscreenElement !== null);
    };
    document.addEventListener("fullscreenchange", on);
    return () => {
      document.removeEventListener("fullscreenchange", on);
    };
  }, []);

  const toggleFullscreen = () => {
    const el = stageRef.current?.parentElement;
    if (!el) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void el.requestFullscreen().catch(() => undefined);
  };

  // ---- overlay de transformação do clip selecionado (visual com content cobrindo o quadro)
  const visual =
    selectedClip &&
    selectedClip.content.type !== "nested" &&
    selectedClip.content.type !== "text" &&
    !(selectedClip.content.type === "media" && !selectedClip.content.has_video);
  const showBox =
    !!selectedClip &&
    !!visual &&
    playhead >= selectedClip.start &&
    playhead < selectedClip.start + selectedClip.duration;
  const k = box.w / seqW || 1;
  const px = selectedClip ? propValue(selectedClip, "position_x", 0) : 0;
  const py = selectedClip ? propValue(selectedClip, "position_y", 0) : 0;
  const sc = selectedClip ? propValue(selectedClip, "scale", 1) : 1;
  const dpx = px + (drag?.dx ?? 0);
  const dpy = py + (drag?.dy ?? 0);
  const dsc = sc * (drag?.k ?? 1);

  const onPointerDown = (e: React.PointerEvent, kind: "move" | "scale") => {
    if (!selectedClip) return;
    e.preventDefault();
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const rect = frameRef.current?.getBoundingClientRect();
    if (kind === "move") dragRef.current = { kind, x0: e.clientX, y0: e.clientY, px, py };
    else if (rect) {
      const cx = rect.left + rect.width / 2 + px * k;
      const cy = rect.top + rect.height / 2 + py * k;
      dragRef.current = {
        kind,
        cx,
        cy,
        d0: Math.max(4, Math.hypot(e.clientX - cx, e.clientY - cy)),
        s0: sc,
      };
    }
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    if (d.kind === "move")
      setDrag({ dx: (e.clientX - d.x0) / k, dy: (e.clientY - d.y0) / k, k: 1 });
    else {
      const dist = Math.max(4, Math.hypot(e.clientX - d.cx, e.clientY - d.cy));
      setDrag({ dx: 0, dy: 0, k: dist / d.d0 });
    }
  };
  const endDrag = (commit: boolean) => {
    const d = dragRef.current;
    dragRef.current = null;
    const cur = drag;
    setDrag(null);
    if (!commit || !d || !cur || !selectedClip) return;
    // uma única transação ⇒ um único passo de undo, mesmo mexendo em duas propriedades
    if (d.kind === "move") {
      void c.setProperties(selectedClip, [
        ["position_x", Math.round(px + cur.dx)],
        ["position_y", Math.round(py + cur.dy)],
      ]);
    } else {
      void c.setProperties(selectedClip, [["scale", Math.round(sc * cur.k * 1000) / 1000]]);
    }
  };

  const qualityOptions: PreviewQuality[] = ["auto", "540", "720"];
  const noSeq = !seqId || !summary || summary.clip_count === 0;

  return (
    <section className="ed-preview" aria-label={t("preview.label")} data-testid="preview">
      <div className="ed-preview-stage" ref={stageRef}>
        <div
          ref={frameRef}
          className="ed-preview-frame"
          data-testid="preview-frame"
          style={{ width: box.w || undefined, height: box.h || undefined }}
        >
          <canvas
            ref={canvasRef}
            data-testid="preview-canvas"
            data-mode={metrics.mode}
            data-presented={metrics.presented}
            data-transport={c.frames.kind}
            width={size.width}
            height={size.height}
          />
          {safeAreas && (
            <div className="ed-safe" data-testid="safe-areas" aria-hidden="true">
              <i style={{ inset: "5%" }} />
              <i style={{ inset: "10%" }} />
            </div>
          )}
          {showBox && (
            <div
              className="ed-xform"
              data-testid="xform-box"
              onPointerMove={onPointerMove}
              onPointerUp={() => {
                endDrag(true);
              }}
              onPointerCancel={() => {
                endDrag(false);
              }}
              style={{
                left: `calc(50% + ${(dpx * k).toFixed(1)}px)`,
                top: `calc(50% + ${(dpy * k).toFixed(1)}px)`,
                width: `${(seqW * k * dsc).toFixed(1)}px`,
                height: `${(seqH * k * dsc).toFixed(1)}px`,
              }}
            >
              <div
                className="ed-xform-body"
                onPointerDown={(e) => {
                  onPointerDown(e, "move");
                }}
              />
              <div
                className="ed-xform-handle"
                data-testid="xform-scale"
                onPointerDown={(e) => {
                  onPointerDown(e, "scale");
                }}
              />
            </div>
          )}
          {noSeq && <div className="ed-preview-empty">{t("preview.empty")}</div>}
          {unavailable && (
            <div className="ed-preview-empty" role="alert">
              {t("preview.unavailable")}
            </div>
          )}
        </div>
      </div>
      <div className="ed-preview-bar">
        <IconButton
          icon="skipBack"
          label={t("playback.goStart")}
          shortcut={formatBinding(keymap.goStart[0] ?? "")}
          data-testid="go-start"
          onClick={() => {
            c.goStart();
          }}
        />
        <IconButton
          icon="stepBack"
          label={t("playback.stepBack")}
          shortcut={formatBinding(keymap.stepBack[0] ?? "")}
          data-testid="step-back"
          onClick={() => {
            c.step(-1);
          }}
        />
        <IconButton
          icon={playing ? "pause" : "play"}
          label={playing ? t("playback.pause") : t("playback.play")}
          shortcut={formatBinding(keymap.playPause[0] ?? "")}
          data-testid="play"
          onClick={() => {
            c.togglePlay();
          }}
        />
        <IconButton
          icon="stepForward"
          label={t("playback.stepForward")}
          shortcut={formatBinding(keymap.stepForward[0] ?? "")}
          data-testid="step-forward"
          onClick={() => {
            c.step(1);
          }}
        />
        <IconButton
          icon="skipForward"
          label={t("playback.goEnd")}
          shortcut={formatBinding(keymap.goEnd[0] ?? "")}
          data-testid="go-end"
          onClick={() => {
            c.goEnd();
          }}
        />
        <span className="ed-timecode" data-testid="preview-timecode" aria-live="off">
          {formatTimecode(playhead, summary?.frame_rate ?? "30")}
        </span>
        <div className="ed-spacer" />
        <span className="ed-metrics" data-testid="preview-metrics" aria-label="preview metrics">
          {metrics.fps} fps · {t("preview.latency", { ms: Math.round(metrics.lastLatencyMs) })} ·{" "}
          {t("preview.dropped", { count: metrics.dropped })} · {size.width}×{size.height} ·{" "}
          {metrics.mode}
        </span>
        {proxy && <Badge>{t("preview.proxy")}</Badge>}
        <Select
          aria-label={t("preview.quality")}
          value={quality}
          data-testid="preview-quality"
          onChange={(e) => {
            c.setPreview({ quality: e.currentTarget.value as PreviewQuality });
          }}
        >
          {qualityOptions.map((q) => (
            <option key={q} value={q}>
              {t(`preview.quality.${q}`)}
            </option>
          ))}
        </Select>
        <IconButton
          icon="proxy"
          label={t("preview.proxy")}
          pressed={proxy}
          data-testid="preview-proxy"
          onClick={() => {
            c.setPreview({ proxy: !proxy });
          }}
        />
        <IconButton
          icon="safeArea"
          label={t("preview.safeAreas")}
          pressed={safeAreas}
          data-testid="preview-safe"
          onClick={() => {
            c.setPreview({ safeAreas: !safeAreas });
          }}
        />
        <IconButton
          icon="fullscreen"
          label={fullscreen ? t("preview.exitFullscreen") : t("preview.fullscreen")}
          data-testid="preview-fullscreen"
          onClick={toggleFullscreen}
        />
      </div>
    </section>
  );
}
