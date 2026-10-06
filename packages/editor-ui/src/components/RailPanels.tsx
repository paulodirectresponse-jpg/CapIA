import { useMemo } from "react";
import { Button, EmptyState, Icon } from "@capia/ui-kit";
import type { Clip, TransitionKind } from "@capia/engine-bindings";
import { useController, useUi } from "../context";
import { useT, type MessageKey } from "../i18n";
import { formatTimecode } from "../lib/timecode";
import { type TextPreset } from "../store/edit";
import { MediaPanel } from "./MediaPanel";

export function TextPanel() {
  const c = useController();
  const t = useT();
  const ready = useUi((s) => s.active !== null);
  const add = (p: TextPreset, label: MessageKey, id: string) => (
    <Button
      className="ed-preset"
      disabled={!ready}
      data-testid={id}
      onClick={() => {
        void c.addText(p);
      }}
    >
      <Icon name={p === "caption" ? "caption" : "text"} /> {t(label)}
    </Button>
  );
  return (
    <div className="ed-text-panel">
      <div className="ed-panel-body" data-testid="text-panel" style={{ flex: "none" }}>
        {add("lowerThird", "text.addText", "text-add-lowerThird")}
        {add("title", "text.addTitle", "text-add-title")}
      </div>
      <h3 className="ed-subhead" style={{ padding: "0 12px" }}>
        {t("text.captionsHead")}
      </h3>
      <CaptionsPanel />
    </div>
  );
}

const CAPTION_STYLES: { id: string; label: string; style: Record<string, unknown> }[] = [
  {
    id: "classic",
    label: "Classic box",
    style: {
      font_family: "sans",
      size_permille: 55,
      weight: 700,
      align: "center",
      color: "#FFFFFF",
      background: "#000000B3",
      stroke: null,
      stroke_permille: 0,
    },
  },
  {
    id: "outline",
    label: "Bold outline",
    style: {
      font_family: "sans",
      size_permille: 65,
      weight: 700,
      align: "center",
      color: "#FFFFFF",
      background: null,
      stroke: "#000000",
      stroke_permille: 6,
    },
  },
  {
    id: "highlight",
    label: "Highlight",
    style: {
      font_family: "sans",
      size_permille: 70,
      weight: 700,
      align: "center",
      color: "#FFE600",
      background: null,
      stroke: "#000000",
      stroke_permille: 6,
    },
  },
];

function CaptionsPanel() {
  const c = useController();
  const t = useT();
  const { seq, playhead, summary } = useUi((s) => ({
    seq: s.active ? s.model.models[s.active] : undefined,
    playhead: s.playhead,
    summary: s.active ? s.model.sequences[s.active] : undefined,
  }));
  const captions = useMemo(() => {
    if (!seq) return [] as Clip[];
    const tracks = new Set(
      seq.tracks
        .filter((x) => (typeof x.role === "string" ? x.role : "") === "captions")
        .map((x) => x.id),
    );
    return Object.values(seq.clips)
      .filter((cl) => tracks.has(cl.track) && cl.content.type === "text")
      .sort((a, b) => a.start - b.start);
  }, [seq]);
  const rate = summary?.frame_rate ?? "30";
  return (
    <div className="ed-panel-body" data-testid="captions-panel">
      <Button
        variant="primary"
        disabled={!seq}
        data-testid="caption-add"
        onClick={() => {
          void c.addText("caption");
        }}
      >
        <Icon name="caption" /> {t("captions.add")}
      </Button>
      <h3 className="ed-subhead">{t("captions.style")}</h3>
      <div className="ed-row" style={{ flexWrap: "wrap" }}>
        {CAPTION_STYLES.map((s) => (
          <Button
            key={s.id}
            disabled={captions.length === 0}
            data-testid={`caption-style-${s.id}`}
            onClick={() => {
              void c.applyCaptionStyle(s.style);
            }}
          >
            {s.label}
          </Button>
        ))}
      </div>
      {captions.length === 0 ? (
        <EmptyState title={t("captions.default")} />
      ) : (
        <ol className="ed-captions" data-testid="caption-list">
          {captions.map((cl, i) => {
            const text = cl.content.type === "text" ? cl.content.text : "";
            const here = playhead >= cl.start && playhead < cl.start + cl.duration;
            return (
              <li key={cl.id} data-here={here}>
                <button
                  className="ed-kf-time"
                  onClick={() => {
                    c.select([cl.id]);
                    c.seek(cl.start);
                  }}
                >
                  {formatTimecode(cl.start, rate)}
                </button>
                <textarea
                  className="cp-input ed-textarea"
                  aria-label={`${t("captions.default")} ${String(i + 1)}`}
                  defaultValue={text}
                  key={text + cl.id}
                  rows={2}
                  onFocus={() => {
                    c.select([cl.id]);
                  }}
                  onBlur={(e) => {
                    if (e.currentTarget.value !== text)
                      void c.setText(cl.id, { text: e.currentTarget.value });
                  }}
                />
                <div className="ed-row">
                  <Button
                    variant="ghost"
                    disabled={!(cl.start < playhead && playhead < cl.start + cl.duration)}
                    onClick={() => {
                      void c.splitClipAt(cl.id);
                    }}
                  >
                    {t("captions.split")}
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={i === captions.length - 1}
                    onClick={() => {
                      void c.mergeCaptionWithNext(cl.id);
                    }}
                  >
                    {t("captions.merge")}
                  </Button>
                </div>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
}

const TRANSITIONS: TransitionKind[] = ["dissolve", "fade", "slide_in"];

export function TransitionsPanel() {
  const c = useController();
  const t = useT();
  const hasSelection = useUi((s) => s.selection.length > 0);
  return (
    <div className="ed-panel-body" data-testid="transitions-panel">
      <p className="ed-hint">{t("transitions.apply")}</p>
      {TRANSITIONS.map((k) => (
        <Button
          key={k}
          className="ed-preset"
          disabled={!hasSelection}
          data-testid={`transition-${k}`}
          onClick={() => {
            void c.applyTransitionToSelection(k);
          }}
        >
          <Icon name="transition" /> {t(`inspector.transition.${k}`)}
        </Button>
      ))}
    </div>
  );
}

export function AudioPanel() {
  const c = useController();
  const t = useT();
  const addTrack = (
    <Button
      data-testid="audio-add-track"
      onClick={() => {
        void c.addTrack("audio", "voice");
      }}
    >
      <Icon name="music" /> {t("audio.addTrack")}
    </Button>
  );
  return (
    <div className="ed-audio-panel">
      <div className="ed-panel-body" style={{ flex: "none" }}>
        <p className="ed-hint">{t("audio.panelHint")}</p>
        {addTrack}
      </div>
      <MediaPanel initialFilter="audio" />
    </div>
  );
}
