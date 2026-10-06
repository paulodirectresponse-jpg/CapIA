import { trackLabels as trackLabelsOf } from "../lib/trackNames";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Icon, IconButton, Slider, Tabs, type MenuEntry } from "@capia/ui-kit";
import { TICKS_PER_SECOND, type Clip } from "@capia/engine-bindings";
import {
  RULER_H,
  TimelineView,
  buildRows,
  fitPps,
  type DropTarget,
  type MovePlan,
} from "@capia/ui-timeline";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { formatBinding, resolveBindings } from "../lib/keymap";
import { formatTimecode } from "../lib/timecode";
import { FORMAT_PRESETS, type FormatPreset } from "../store/edit";
import { useContextMenu } from "./Menus";

const ROLE_VAR: Record<string, string> = {
  main: "--role-main",
  overlay: "--role-overlay",
  text: "--role-text",
  captions: "--role-captions",
  voice: "--role-voice",
  music: "--role-music",
  sfx: "--role-sfx",
};

declare global {
  interface Window {
    /** Gancho de teste E2E (só quando a página abre com `?e2e=1`). */
    __capiaTimeline?: {
      clipRect(id: string): { x: number; y: number; w: number; h: number } | null;
      rowRect(track: string): { y: number; h: number } | null;
      canvasOrigin(): { x: number; y: number };
      stats(): {
        fps: number;
        paintMs: number;
        visibleClips: number;
        frames: number;
        paintSamples: number[];
        gestureSamples: number[];
      };
      resetSamples(): void;
      viewState(): { pps: number; origin: number; scrollY: number; playhead: number };
    };
  }
}

export function TimelinePanel() {
  const c = useController();
  const t = useT();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const areaRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<TimelineView | null>(null);
  const openContextRef = useRef<
    (t: { clip: string | null; track: string | null; time: number }, x: number, y: number) => void
  >(() => undefined);
  const hScroll = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ origin: 0, scrollY: 0 });
  const [size, setSize] = useState({ w: 800, h: 240 });
  const menu = useContextMenu();
  const presetMenu = useContextMenu();

  const {
    seqId,
    seq,
    summary,
    assets,
    selection,
    playhead,
    prefs,
    tool,
    tabs,
    active,
    sequences,
    selectedTrack,
    nonce,
  } = useUi((s) => ({
    seqId: s.active,
    seq: s.active ? (s.model.models[s.active] ?? null) : null,
    summary: s.active ? (s.model.sequences[s.active] ?? null) : null,
    assets: s.model.assets,
    selection: s.selection,
    playhead: s.playhead,
    prefs: s.prefs,
    tool: s.tool,
    tabs: s.tabs,
    active: s.active,
    sequences: s.model.sequences,
    selectedTrack: s.selectedTrack,
    nonce: s.zoomFitNonce,
  }));
  const pps = prefs.timeline.pxPerSecond;
  const heights = prefs.timeline.trackHeights;
  const rateText = summary?.frame_rate ?? "30";
  const b = resolveBindings(prefs.keymap);
  const kb = (a: keyof typeof b) => formatBinding(b[a][0] ?? "");

  const trackLabels = useMemo(() => {
    return trackLabelsOf(seq ? seq.tracks : [], {
      video: t("track.video"),
      audio: t("track.audio"),
      captions: t("track.captions"),
    });
  }, [seq, t]);
  const rows = useMemo(
    () => (seq ? buildRows(seq.tracks, heights) : { rows: [], totalHeight: 0 }),
    [seq, heights],
  );

  // ---- cria/destrói a view (uma por montagem)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const view = new TimelineView(
      canvas,
      {
        onScrub: (tk) => {
          c.pause();
          c.seek(tk);
        },
        onSelect: (ids, mode) => {
          c.select(ids, mode);
        },
        onMove: (plan: MovePlan) => {
          void c.moveClips(plan);
        },
        onKeyframesMove: (clip, moves) => {
          void c.moveKeyframes(clip, moves);
        },
        onTrim: (clip, edge, to) => {
          void c.trimClip(clip, edge, to);
        },
        onOpenNested: (clip) => {
          void c.openNested(clip);
        },
        onContextMenu: (target, x, y) => {
          openContextRef.current(target, x, y);
        },
        onDropAsset: (asset, target: DropTarget) => {
          void c.dropAsset(asset, target);
        },
        onDropSequence: (sequence, target: DropTarget) => {
          void c.dropSequence(sequence, target);
        },
        onViewport: (v) => {
          c.setZoom(v.pps);
          setViewport({ origin: v.origin, scrollY: v.scrollY });
        },
        onBladeCut: (clip, at) => {
          void c.bladeCut(clip, at);
        },
      },
      {
        peaks: (asset, buckets) => c.visuals.waveform(asset, buckets),
        thumbnail: (asset) => c.visuals.thumbnail(asset),
        formatTime: (tk) =>
          formatTimecode(
            tk,
            c.state.active ? (c.state.model.sequences[c.state.active]?.frame_rate ?? "30") : "30",
          ),
        dragSpan: () => c.dragSpan(),
      },
      () => c.getCore(),
    );
    viewRef.current = view;
    const unsub = c.visuals.subscribe(() => {
      view.invalidate();
    });
    if (new URLSearchParams(window.location.search).has("e2e")) {
      window.__capiaTimeline = {
        clipRect: (id) => view.clipRect(id),
        rowRect: (track) => view.rowRect(track),
        canvasOrigin: () => {
          const r = canvas.getBoundingClientRect();
          return { x: r.left, y: r.top };
        },
        stats: () => ({
          ...view.stats,
          paintSamples: [...view.stats.paintSamples],
          gestureSamples: [...view.stats.gestureSamples],
        }),
        resetSamples: () => {
          view.stats.paintSamples.length = 0;
          view.stats.gestureSamples.length = 0;
        },
        viewState: () => ({
          pps: view.viewState.pps,
          origin: view.viewState.origin,
          scrollY: view.viewState.scrollY,
          playhead: view.viewState.playhead,
        }),
      };
    }
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape" && view.gestureActive) {
        view.cancelGesture();
        e.stopPropagation();
        e.preventDefault();
      }
    };
    window.addEventListener("keydown", esc, true);
    return () => {
      window.removeEventListener("keydown", esc, true);
      unsub();
      view.dispose();
      viewRef.current = null;
      delete window.__capiaTimeline;
    };
    // a view vive durante toda a montagem; callbacks usam `c` (estável)
  }, [c]);

  // ---- tamanho
  useEffect(() => {
    const el = areaRef.current;
    if (!el) return;
    const apply = () => {
      const r = el.getBoundingClientRect();
      setSize({ w: Math.max(1, r.width), h: Math.max(1, r.height) });
      viewRef.current?.resize(r.width, r.height, window.devicePixelRatio || 1);
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    return () => {
      ro.disconnect();
    };
  }, []);

  // ---- dados e estado → view
  useEffect(() => {
    viewRef.current?.setSequence(seq, assets, heights);
  }, [seq, assets, heights]);

  useEffect(() => {
    const markers = seq ? Object.values(seq.markers) : [];
    viewRef.current?.setState({
      pps,
      origin: viewport.origin,
      scrollY: viewport.scrollY,
      playhead,
      selection: new Set(selection),
      snapping: prefs.timeline.snapping,
      tool,
      markers,
      focusClip: selection.length === 1 ? (selection[0] ?? null) : null,
    });
  }, [seq, pps, viewport, playhead, selection, prefs.timeline.snapping, tool]);

  // ---- ajustar à timeline
  useEffect(() => {
    if (nonce === 0 || !seq) return;
    const dur = summary?.duration ?? 0;
    c.setZoom(fitPps(dur, size.w));
    // eslint-disable-next-line react-hooks/set-state-in-effect -- reação ao pedido de ajuste (nonce)
    setViewport((v) => ({ ...v, origin: 0 }));
    // só reage ao pedido
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nonce]);

  // ---- mantém o playhead visível durante a reprodução
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const right = viewport.origin + (size.w / pps) * TICKS_PER_SECOND;
    if (playhead > right || playhead < viewport.origin) {
      if (c.state.playing)
        // eslint-disable-next-line react-hooks/set-state-in-effect -- segue o playhead
        setViewport((v) => ({
          ...v,
          origin: Math.max(0, playhead - Math.round(TICKS_PER_SECOND * 0.5)),
        }));
    }
  }, [playhead, pps, size.w, viewport.origin, c]);

  // ---- rolagem horizontal (proxy nativo)
  const totalW = Math.max(
    size.w,
    ((summary?.duration ?? 0) / TICKS_PER_SECOND + 30) * pps + size.w * 0.5,
  );
  const syncing = useRef(false);
  useEffect(() => {
    const el = hScroll.current;
    if (!el) return;
    const want = (viewport.origin / TICKS_PER_SECOND) * pps;
    if (Math.abs(el.scrollLeft - want) > 1) {
      syncing.current = true;
      el.scrollLeft = want;
    }
  }, [viewport.origin, pps]);
  const onHScroll = useCallback(() => {
    if (syncing.current) {
      syncing.current = false;
      return;
    }
    const el = hScroll.current;
    if (!el) return;
    setViewport((v) => ({ ...v, origin: Math.round((el.scrollLeft / pps) * TICKS_PER_SECOND) }));
  }, [pps]);

  const maxScrollY = Math.max(0, rows.totalHeight - (size.h - RULER_H - 14));
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- clamp ao encolher o conteúdo
    setViewport((v) => (v.scrollY > maxScrollY ? { ...v, scrollY: maxScrollY } : v));
  }, [maxScrollY]);

  // ---- menu de contexto da timeline
  const openContext = (
    target: { clip: string | null; track: string | null; time: number },
    x: number,
    y: number,
  ) => {
    const st = c.state;
    const model = st.active ? st.model.models[st.active] : undefined;
    const clip: Clip | undefined = target.clip ? model?.clips[target.clip] : undefined;
    if (clip && !st.selection.includes(clip.id)) c.select([clip.id], "replace");
    const entries: MenuEntry[] = clip
      ? [
          { type: "item", id: "split", label: t("timeline.split"), shortcut: kb("split") },
          { type: "item", id: "delete", label: t("timeline.delete"), shortcut: kb("delete") },
          {
            type: "item",
            id: "ripple",
            label: t("timeline.rippleDelete"),
            shortcut: kb("rippleDelete"),
          },
          { type: "separator" },
          { type: "item", id: "copy", label: t("timeline.copy"), shortcut: kb("copy") },
          {
            type: "item",
            id: "duplicate",
            label: t("timeline.duplicate"),
            shortcut: kb("duplicate"),
          },
          {
            type: "item",
            id: "group",
            label: t("timeline.group"),
            shortcut: kb("group"),
            disabled: st.selection.length < 2,
          },
          {
            type: "item",
            id: "ungroup",
            label: t("timeline.ungroup"),
            shortcut: kb("ungroup"),
            disabled: clip.group == null,
          },
          ...(clip.content.type === "media" && clip.content.has_video && clip.content.has_audio
            ? ([{ type: "item", id: "detach", label: t("timeline.detachAudio") }] as MenuEntry[])
            : []),
          ...(clip.content.type === "nested"
            ? ([
                { type: "separator" },
                { type: "item", id: "open", label: t("timeline.openNested") },
                { type: "item", id: "unique", label: t("tabs.makeUnique") },
              ] as MenuEntry[])
            : []),
        ]
      : [
          {
            type: "item",
            id: "paste",
            label: t("timeline.paste"),
            shortcut: kb("paste"),
            disabled: st.clipboard === null,
          },
          { type: "item", id: "marker", label: t("timeline.addMarker"), shortcut: kb("addMarker") },
        ];
    menu.open({
      x,
      y,
      label: t("timeline.label"),
      entries,
      onSelect: (id) => {
        if (target.track) c.selectTrack(target.track);
        if (id === "split") void c.split();
        else if (id === "delete") void c.deleteSelection(false);
        else if (id === "ripple") void c.deleteSelection(true);
        else if (id === "copy") c.copy();
        else if (id === "duplicate") void c.duplicate();
        else if (id === "group") void c.group();
        else if (id === "ungroup") void c.ungroup();
        else if (id === "detach") void c.detachAudio();
        else if (id === "open" && clip) void c.openNested(clip.id);
        else if (id === "unique" && clip) void c.makeUnique(clip.id);
        else if (id === "paste") {
          c.seek(target.time);
          void c.paste();
        } else if (id === "marker") {
          c.seek(target.time);
          void c.addMarker();
        }
      },
    });
  };
  useEffect(() => {
    openContextRef.current = openContext;
  });

  const breadcrumb = c.breadcrumb();
  const usage = summary?.nested_usage ?? 0;
  const noSeq = !seqId || !seq;

  return (
    <section
      className="ed-timeline"
      aria-label={t("timeline.label")}
      data-testid="timeline"
      style={{ height: prefs.panels.timelineHeight }}
    >
      <Tabs
        label={t("tabs.sequences")}
        active={active}
        items={tabs.map((id) => ({
          id,
          label: sequences[id]?.name ?? id,
          closable: true,
          title: sequences[id]?.name ?? id,
        }))}
        onSelect={(id) => {
          c.setActive(id);
        }}
        onClose={(id) => {
          c.closeTab(id);
        }}
        onContextMenu={(id, x, y) => {
          menu.open({
            x,
            y,
            label: sequences[id]?.name ?? id,
            entries: [
              { type: "item", id: "rename", label: t("common.rename") },
              { type: "item", id: "duplicate", label: t("common.duplicate") },
              { type: "item", id: "close", label: t("tabs.close") },
              {
                type: "item",
                id: "others",
                label: t("tabs.closeOthers"),
                disabled: tabs.length < 2,
              },
            ],
            onSelect: (a) => {
              if (a === "rename") {
                c.setActive(id);
                c.store.set({ renaming: id });
              } else if (a === "duplicate") void c.duplicateSequence(id);
              else if (a === "close") c.closeTab(id);
              else if (a === "others") c.closeOtherTabs(id);
            },
          });
        }}
        trailing={
          <>
            <IconButton
              icon="plus"
              label={t("tabs.newSequence")}
              shortcut={kb("newSequence")}
              data-testid="tab-new-sequence"
              onClick={() => {
                void c.createSequence();
              }}
            />
            <IconButton
              icon="chevronDown"
              label={t("preset.newFrom")}
              data-testid="tab-new-preset"
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                presetMenu.open({
                  x: r.left,
                  y: r.bottom + 4,
                  label: t("preset.newFrom"),
                  entries: [
                    { type: "item", id: "active", label: t("preset.fromActive") },
                    { type: "separator" },
                    ...(Object.keys(FORMAT_PRESETS) as FormatPreset[]).map((p): MenuEntry => ({
                      type: "item",
                      id: p,
                      label: t(`preset.${p}`),
                    })),
                    { type: "separator" },
                    { type: "item", id: "dup", label: t("common.duplicate"), disabled: !active },
                  ],
                  onSelect: (id) => {
                    if (id === "dup" && active) void c.duplicateSequence(active);
                    else void c.createSequence({ preset: id as FormatPreset | "active" });
                  },
                });
              }}
            />
          </>
        }
      />
      {breadcrumb.length > 1 && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            padding: "2px 10px",
            background: "var(--surface-2)",
            fontSize: "var(--fs-sm)",
          }}
          data-testid="breadcrumb"
          aria-label={t("tabs.breadcrumb")}
        >
          {breadcrumb.map((id, i) => (
            <span key={id} style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
              {i > 0 && <Icon name="chevronRight" size={10} />}
              <button
                type="button"
                className="cp-btn"
                data-variant="ghost"
                style={{ height: 20, padding: "0 6px" }}
                onClick={() => {
                  c.setActive(id);
                }}
              >
                {sequences[id]?.name ?? id}
              </button>
            </span>
          ))}
          {usage > 1 && <Badge tone="warning">{t("tabs.sharedMaster", { count: usage })}</Badge>}
        </div>
      )}
      <div className="ed-tl-toolbar" role="toolbar" aria-label={t("timeline.label")}>
        <IconButton
          icon="grid"
          label={t("timeline.tool.select")}
          pressed={tool === "select"}
          data-testid="tool-select"
          onClick={() => {
            c.setTool("select");
          }}
        />
        <IconButton
          icon="scissors"
          label={t("timeline.tool.blade")}
          pressed={tool === "blade"}
          data-testid="tool-blade"
          onClick={() => {
            c.setTool("blade");
          }}
        />
        <span className="ed-tl-sep" />
        <IconButton
          icon="scissors"
          label={t("timeline.split")}
          shortcut={kb("split")}
          disabled={noSeq}
          data-testid="split"
          onClick={() => {
            void c.split();
          }}
        />
        <IconButton
          icon="trash"
          label={t("timeline.delete")}
          shortcut={kb("delete")}
          disabled={noSeq}
          data-testid="delete"
          onClick={() => {
            void c.deleteSelection(false);
          }}
        />
        <IconButton
          icon="trash"
          label={t("timeline.rippleDelete")}
          shortcut={kb("rippleDelete")}
          disabled={noSeq}
          data-testid="ripple-delete"
          onClick={() => {
            void c.deleteSelection(true);
          }}
        />
        <IconButton
          icon="stepBack"
          label={t("timeline.trimLeft")}
          shortcut={kb("trimLeft")}
          disabled={noSeq}
          data-testid="trim-left"
          onClick={() => {
            void c.trimToPlayhead("in");
          }}
        />
        <IconButton
          icon="stepForward"
          label={t("timeline.trimRight")}
          shortcut={kb("trimRight")}
          disabled={noSeq}
          data-testid="trim-right"
          onClick={() => {
            void c.trimToPlayhead("out");
          }}
        />
        <span className="ed-tl-sep" />
        <IconButton
          icon="group"
          label={t("timeline.group")}
          shortcut={kb("group")}
          disabled={noSeq || selection.length < 2}
          data-testid="group"
          onClick={() => {
            void c.group();
          }}
        />
        <IconButton
          icon="link"
          label={t("timeline.ungroup")}
          shortcut={kb("ungroup")}
          disabled={noSeq || selection.length < 1}
          data-testid="ungroup"
          onClick={() => {
            void c.ungroup();
          }}
        />
        <IconButton
          icon="duplicate"
          label={t("timeline.duplicate")}
          shortcut={kb("duplicate")}
          disabled={noSeq || selection.length < 1}
          data-testid="duplicate"
          onClick={() => {
            void c.duplicate();
          }}
        />
        <IconButton
          icon="music"
          label={t("timeline.detachAudio")}
          disabled={noSeq || selection.length < 1}
          data-testid="detach-audio"
          onClick={() => {
            void c.detachAudio();
          }}
        />
        <span className="ed-tl-sep" />
        <IconButton
          icon="magnet"
          label={t("timeline.snapping")}
          shortcut={kb("toggleSnapping")}
          pressed={prefs.timeline.snapping}
          data-testid="snapping"
          onClick={() => {
            c.setSnapping(!prefs.timeline.snapping);
          }}
        />
        <IconButton
          icon="keyframe"
          label={t("timeline.addMarker")}
          shortcut={kb("addMarker")}
          disabled={noSeq}
          data-testid="add-marker"
          onClick={() => {
            void c.addMarker();
          }}
        />
        <div className="ed-spacer" />
        <span className="ed-timecode" data-testid="timecode" aria-label={t("timeline.timecode")}>
          {formatTimecode(playhead, rateText)}
        </span>
        <IconButton
          icon="zoomOut"
          label={t("timeline.zoomOut")}
          shortcut={kb("zoomOut")}
          data-testid="zoom-out"
          onClick={() => {
            c.runAction("zoomOut");
          }}
        />
        <div style={{ width: 110 }}>
          <Slider
            label={t("timeline.zoom")}
            min={Math.log(2)}
            max={Math.log(4000)}
            step={0.01}
            value={Math.log(pps)}
            onChange={(v) => {
              c.setZoom(Math.exp(v));
            }}
          />
        </div>
        <IconButton
          icon="zoomIn"
          label={t("timeline.zoomIn")}
          shortcut={kb("zoomIn")}
          data-testid="zoom-in"
          onClick={() => {
            c.runAction("zoomIn");
          }}
        />
        <IconButton
          icon="fit"
          label={t("timeline.zoomFit")}
          shortcut={kb("zoomFit")}
          data-testid="zoom-fit"
          onClick={() => {
            c.runAction("zoomFit");
          }}
        />
      </div>
      <div className="ed-tl-body">
        <div
          className="ed-tl-headers"
          aria-label={t("timeline.tracksLabel")}
          style={{ paddingTop: RULER_H }}
        >
          <div style={{ transform: `translateY(${String(-viewport.scrollY)}px)` }}>
            {rows.rows.map((row) => {
              const tr = row.track;
              const role = typeof tr.role === "string" ? tr.role : "custom";
              const label = trackLabels.get(tr.id) ?? "";
              return (
                <div
                  key={tr.id}
                  className="ed-tl-header"
                  style={{ height: row.h }}
                  data-testid={`track-header-${tr.id}`}
                  data-selected={selectedTrack === tr.id}
                  onClick={() => {
                    c.selectTrack(tr.id);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    menu.open({
                      x: e.clientX,
                      y: e.clientY,
                      label: tr.name || tr.id,
                      entries: [
                        {
                          type: "item",
                          id: "magnetic",
                          label: t("track.magnetic"),
                          disabled: tr.kind !== "visual",
                        },
                        { type: "item", id: "delete", label: t("track.delete"), danger: true },
                      ],
                      onSelect: (id) => {
                        if (id === "magnetic")
                          void c.setTrackFlags(tr.id, { magnetic: !tr.magnetic });
                        else void c.deleteTrack(tr.id);
                      },
                    });
                  }}
                >
                  <span
                    className="ed-tl-role"
                    style={{ background: `var(${ROLE_VAR[role] ?? "--text-muted"})` }}
                  />
                  <div className="ed-tl-header-name">
                    <Icon name={tr.kind === "audio" ? "music" : "film"} size={12} />
                    <span title={label}>{label}</span>
                    {tr.magnetic && <Badge>{t("track.magnetic")}</Badge>}
                  </div>
                  <div className="ed-tl-header-btns">
                    <IconButton
                      icon={tr.locked ? "lock" : "unlock"}
                      label={tr.locked ? t("track.unlock") : t("track.lock")}
                      pressed={tr.locked}
                      data-testid={`lock-${tr.id}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        void c.setTrackFlags(tr.id, { locked: !tr.locked });
                      }}
                    />
                    {tr.kind === "visual" && (
                      <IconButton
                        icon={tr.hidden ? "eyeOff" : "eye"}
                        label={tr.hidden ? t("track.show") : t("track.hide")}
                        pressed={tr.hidden}
                        data-testid={`hide-${tr.id}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          void c.setTrackFlags(tr.id, { hidden: !tr.hidden });
                        }}
                      />
                    )}
                    {tr.kind === "audio" && (
                      <>
                        <IconButton
                          icon={tr.muted ? "mute" : "volume"}
                          label={tr.muted ? t("track.unmute") : t("track.mute")}
                          pressed={tr.muted}
                          data-testid={`mute-${tr.id}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            void c.setTrackFlags(tr.id, { muted: !tr.muted });
                          }}
                        />
                        <IconButton
                          icon="solo"
                          label={t("track.solo")}
                          pressed={tr.solo}
                          data-testid={`solo-${tr.id}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            void c.setTrackFlags(tr.id, { solo: !tr.solo });
                          }}
                        />
                      </>
                    )}
                  </div>
                  <div
                    className="ed-tl-resize"
                    role="separator"
                    aria-label={t("track.resize")}
                    aria-orientation="horizontal"
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      const startY = e.clientY;
                      const startH = row.h;
                      const el = e.currentTarget;
                      el.setPointerCapture(e.pointerId);
                      const move = (ev: PointerEvent) => {
                        c.setTrackHeight(tr.id, startH + ev.clientY - startY);
                      };
                      const up = () => {
                        el.removeEventListener("pointermove", move);
                        el.removeEventListener("pointerup", up);
                      };
                      el.addEventListener("pointermove", move);
                      el.addEventListener("pointerup", up);
                    }}
                  />
                </div>
              );
            })}
          </div>
        </div>
        <div className="ed-tl-canvas" ref={areaRef}>
          <canvas ref={canvasRef} data-testid="timeline-canvas" aria-label={t("timeline.label")} />
          {noSeq && (
            <div
              style={{
                position: "absolute",
                inset: 0,
                display: "grid",
                placeItems: "center",
                color: "var(--text-muted)",
                pointerEvents: "none",
              }}
            >
              {t("timeline.noSequence")}
            </div>
          )}
          {!noSeq && rows.rows.length > 0 && (summary?.clip_count ?? 0) === 0 && (
            <div
              style={{
                position: "absolute",
                left: 0,
                right: 0,
                top: RULER_H + 8,
                textAlign: "center",
                color: "var(--text-muted)",
                pointerEvents: "none",
              }}
            >
              {t("timeline.empty")}
            </div>
          )}
          <div ref={hScroll} className="ed-tl-hscroll" onScroll={onHScroll} aria-hidden="true">
            <div style={{ width: totalW, height: 1 }} />
          </div>
          <div
            className="ed-tl-vscroll"
            aria-hidden="true"
            onScroll={(e) => {
              const y = e.currentTarget.scrollTop;
              setViewport((v) => (Math.abs(v.scrollY - y) > 1 ? { ...v, scrollY: y } : v));
            }}
          >
            <div style={{ height: rows.totalHeight + 40, width: 1 }} />
          </div>
        </div>
      </div>
      {menu.node}
      {presetMenu.node}
      <span className="cp-sr-only">
        <Button
          onClick={() => {
            c.runAction("zoomFit");
          }}
        >
          {t("timeline.zoomFit")}
        </Button>
      </span>
    </section>
  );
}
