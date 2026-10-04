import { useState } from "react";
import {
  Button,
  EmptyState,
  IconButton,
  NumberField,
  Select,
  Slider,
  Tabs,
  TextInput,
} from "@capia/ui-kit";
import {
  TICKS_PER_SECOND,
  type Clip,
  type SequenceSummary,
  type TextStyle,
  type TransitionKind,
} from "@capia/engine-bindings";
import { useController, useUi } from "../context";
import { useT, type MessageKey } from "../i18n";
import {
  ANIMATABLE,
  contentTimeAt,
  evalAt,
  interpFromKind,
  interpKind,
  keyframesOf,
} from "../lib/anim";
import { formatTimecode, frameTicks, nominalFps } from "../lib/timecode";
import { FORMAT_PRESETS, type FormatPreset } from "../store/edit";

type Tab = "basic" | "animation" | "audio" | "speed";

const PROP_LABEL: Record<string, MessageKey> = {
  position_x: "inspector.positionX",
  position_y: "inspector.positionY",
  scale: "inspector.scale",
  rotation: "inspector.rotation",
  opacity: "inspector.opacity",
  volume_db: "inspector.volume",
};

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="ed-field-row">
      <span className="ed-field-label">{label}</span>
      <div className="ed-field-input">{children}</div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="ed-section">
      <h3>{title}</h3>
      {children}
    </section>
  );
}

/** `#RRGGBB[AA]` ↔ seletor nativo (RGB) + campo hex (com alfa). */
function ColorField({
  label,
  value,
  allowNone,
  onCommit,
}: {
  label: string;
  value: string | null | undefined;
  allowNone?: boolean;
  onCommit: (v: string | null) => void;
}) {
  const t = useT();
  const rgb = value && /^#[0-9a-fA-F]{6}/.test(value) ? value.slice(0, 7) : "#ffffff";
  return (
    <div className="ed-color">
      <input
        type="color"
        aria-label={label}
        value={rgb.toLowerCase()}
        onChange={(e) => {
          const alpha = value && value.length === 9 ? value.slice(7) : "";
          onCommit(e.currentTarget.value.toUpperCase() + alpha);
        }}
      />
      <TextInput
        aria-label={`${label} hex`}
        value={value ?? ""}
        placeholder="#RRGGBB"
        onChange={() => undefined}
        onBlur={(e) => {
          const v = e.currentTarget.value.trim();
          if (v === "" && allowNone) onCommit(null);
          else if (/^#([0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(v))
            onCommit(v.toUpperCase());
        }}
        defaultValue={value ?? ""}
        key={value ?? "none"}
      />
      {allowNone && value && (
        <IconButton
          icon="close"
          label={t("common.none")}
          onClick={() => {
            onCommit(null);
          }}
        />
      )}
    </div>
  );
}

export function Inspector() {
  const c = useController();
  const t = useT();
  const { selection, seqId, playhead } = useUi((s) => ({
    selection: s.selection,
    seqId: s.active,
    playhead: s.playhead,
  }));
  const seq = useUi((s) => (s.active ? s.model.models[s.active] : undefined));
  const summary = useUi((s) => (s.active ? s.model.sequences[s.active] : undefined));
  const assets = useUi((s) => s.model.assets);
  const [tab, setTab] = useState<Tab>("basic");

  const clips = seq ? selection.map((id) => seq.clips[id]).filter((x): x is Clip => !!x) : [];

  let body: React.ReactNode;
  let heading = t("inspector.title");
  if (!seqId || !seq || !summary) {
    body = <EmptyState title={t("inspector.empty")} />;
  } else if (clips.length === 0) {
    heading = t("inspector.sequence");
    body = <SequenceInspector id={seqId} summary={summary} />;
  } else if (clips.length > 1) {
    body = (
      <div className="ed-panel-body" data-testid="inspector-multi">
        <p>{t("inspector.multiple", { count: clips.length })}</p>
        <Row label={t("inspector.enabled")}>
          <Button
            onClick={() => {
              for (const cl of clips) void c.setClipEnabled(cl.id, !cl.enabled);
            }}
          >
            {t("inspector.enabled")}
          </Button>
        </Row>
      </div>
    );
  } else {
    const clip = clips[0];
    if (clip) {
      heading = t("inspector.clip");
      const media = clip.content.type === "media" ? clip.content : null;
      const hasAudio =
        (media?.has_audio ?? false) ||
        (clip.content.type === "media" && assets[clip.content.asset]?.kind === "audio");
      const tabs: { id: Tab; label: string }[] = [
        { id: "basic", label: t("inspector.tab.basic") },
        { id: "animation", label: t("inspector.tab.animation") },
        ...(hasAudio ? [{ id: "audio" as Tab, label: t("inspector.tab.audio") }] : []),
        ...(clip.content.type === "media" || clip.content.type === "nested"
          ? [{ id: "speed" as Tab, label: t("inspector.tab.speed") }]
          : []),
      ];
      const active = tabs.some((x) => x.id === tab) ? tab : "basic";
      body = (
        <>
          <Tabs
            label={t("inspector.title")}
            items={tabs}
            active={active}
            onSelect={(id) => {
              setTab(id as Tab);
            }}
          />
          <div className="ed-panel-body" data-testid={`inspector-${active}`}>
            {active === "basic" && <BasicTab clip={clip} summary={summary} playhead={playhead} />}
            {active === "animation" && <AnimationTab clip={clip} playhead={playhead} />}
            {active === "audio" && <AudioTab clip={clip} playhead={playhead} />}
            {active === "speed" && <SpeedTab clip={clip} />}
          </div>
        </>
      );
    }
  }
  return (
    <aside className="ed-inspector-inner" aria-label={t("inspector.title")} data-testid="inspector">
      <div className="ed-panel-head">{heading}</div>
      {body}
    </aside>
  );
}

// -------------------------------------------------------------------------------- sequence

function SequenceInspector({ id, summary }: { id: string; summary: SequenceSummary }) {
  const c = useController();
  const t = useT();
  const preset = (Object.keys(FORMAT_PRESETS) as FormatPreset[]).find(
    (k) => FORMAT_PRESETS[k].width === summary.width && FORMAT_PRESETS[k].height === summary.height,
  );
  return (
    <div className="ed-panel-body" data-testid="inspector-sequence">
      <Row label={t("common.name")}>
        <TextInput
          aria-label={t("common.name")}
          defaultValue={summary.name}
          key={summary.name}
          data-testid="seq-name"
          onBlur={(e) => {
            const v = e.currentTarget.value.trim();
            if (v && v !== summary.name) void c.renameSequence(id, v);
          }}
        />
      </Row>
      <Row label={t("inspector.format")}>
        <Select
          aria-label={t("inspector.format")}
          value={preset ?? "custom"}
          data-testid="seq-format"
          onChange={(e) => {
            const p = FORMAT_PRESETS[e.currentTarget.value as FormatPreset];
            void c.setSequenceFormat(id, p.width, p.height);
          }}
        >
          {preset === undefined && (
            <option value="custom">
              {summary.width}×{summary.height}
            </option>
          )}
          {(Object.keys(FORMAT_PRESETS) as FormatPreset[]).map((k) => (
            <option key={k} value={k}>
              {t(`preset.${k}`)} ({FORMAT_PRESETS[k].width}×{FORMAT_PRESETS[k].height})
            </option>
          ))}
        </Select>
      </Row>
      <Row label={t("inspector.frameRate")}>
        <span>{summary.frame_rate} fps</span>
      </Row>
      <Row label={t("inspector.duration")}>
        <span>{formatTimecode(summary.duration, summary.frame_rate)}</span>
      </Row>
    </div>
  );
}

// ------------------------------------------------------------------------------------ basic

function BasicTab({
  clip,
  summary,
  playhead,
}: {
  clip: Clip;
  summary: SequenceSummary;
  playhead: number;
}) {
  const c = useController();
  const t = useT();
  const ct = contentTimeAt(clip, playhead);
  const val = (name: string) => {
    const spec = ANIMATABLE.find((a) => a.name === name);
    return evalAt(clip.properties[name], ct, spec?.dflt ?? 0);
  };
  const animated = (name: string) => {
    const p = clip.properties[name];
    return !!p && "animated" in p;
  };
  const visual =
    clip.content.type !== "nested"
      ? !(clip.content.type === "media" && !clip.content.has_video)
      : true;
  const field = (name: string, scale = 1, label?: MessageKey) => {
    const spec = ANIMATABLE.find((a) => a.name === name);
    if (!spec) return null;
    return (
      <Row label={t(label ?? PROP_LABEL[name] ?? "inspector.position")}>
        <NumberField
          label={t(label ?? PROP_LABEL[name] ?? "inspector.position")}
          value={val(name) * scale}
          min={spec.min * scale}
          max={spec.max * scale}
          step={spec.step * scale}
          onCommit={(v) => {
            void c.setProperty(clip, name, v / scale);
          }}
        />
        {animated(name) && (
          <span className="ed-kf-dot" title={t("inspector.animatedHint")}>
            ◆
          </span>
        )}
      </Row>
    );
  };
  const trans = clip.transition_in;
  const frame = frameTicks(summary.frame_rate);
  return (
    <>
      <Row label={t("inspector.name")}>
        <TextInput
          aria-label={t("inspector.name")}
          defaultValue={clip.name}
          key={clip.name + clip.id}
          data-testid="clip-name"
          onBlur={(e) => {
            const v = e.currentTarget.value.trim();
            if (v && v !== clip.name) void c.renameClip(clip.id, v);
          }}
        />
      </Row>
      <Row label={t("inspector.enabled")}>
        <input
          type="checkbox"
          aria-label={t("inspector.enabled")}
          checked={clip.enabled}
          data-testid="clip-enabled"
          onChange={(e) => {
            void c.setClipEnabled(clip.id, e.currentTarget.checked);
          }}
        />
      </Row>
      <Row label={t("inspector.start")}>
        <span data-testid="clip-start">{formatTimecode(clip.start, summary.frame_rate)}</span>
      </Row>
      <Row label={t("inspector.duration")}>
        <span data-testid="clip-duration">{formatTimecode(clip.duration, summary.frame_rate)}</span>
      </Row>
      {visual && (
        <Section title={t("inspector.position")}>
          {field("position_x")}
          {field("position_y")}
          {field("scale", 100)}
          {field("rotation")}
          {field("opacity", 100)}
        </Section>
      )}
      {clip.content.type === "text" && (
        <TextSection clip={clip as Clip & { content: { type: "text" } }} />
      )}
      {visual && (
        <Section title={t("inspector.transition")}>
          <Row label={t("inspector.transition")}>
            <Select
              aria-label={t("inspector.transition")}
              value={trans?.kind ?? "none"}
              data-testid="transition-kind"
              onChange={(e) => {
                const v = e.currentTarget.value;
                void c.setTransition(
                  clip.id,
                  v === "none"
                    ? null
                    : {
                        kind: v as TransitionKind,
                        duration:
                          trans?.duration ?? Math.round(TICKS_PER_SECOND / 2 / frame) * frame,
                      },
                );
              }}
            >
              {(["none", "dissolve", "fade", "slide_in"] as const).map((k) => (
                <option key={k} value={k}>
                  {t(`inspector.transition.${k}`)}
                </option>
              ))}
            </Select>
          </Row>
          {trans && (
            <Row label={t("inspector.transitionDuration")}>
              <NumberField
                label={t("inspector.transitionDuration")}
                value={Math.round(trans.duration / frame)}
                min={1}
                max={nominalFps(summary.frame_rate) * 10}
                onCommit={(v) => {
                  void c.setTransition(clip.id, {
                    kind: trans.kind,
                    duration: Math.round(v) * frame,
                  });
                }}
              />
            </Row>
          )}
        </Section>
      )}
    </>
  );
}

function TextSection({
  clip,
}: {
  clip: Clip & { content: { type: "text"; text: string; style?: TextStyle } };
}) {
  const c = useController();
  const t = useT();
  const style: TextStyle = {
    font_family: "sans",
    size_permille: 60,
    weight: 400,
    align: "center",
    color: "#FFFFFF",
    ...clip.content.style,
  };
  const set = (patch: Partial<TextStyle>) => {
    void c.setText(clip.id, { style: { ...style, ...patch } });
  };
  return (
    <Section title={t("inspector.text")}>
      <textarea
        className="cp-input ed-textarea"
        aria-label={t("inspector.text")}
        data-testid="clip-text"
        defaultValue={clip.content.text}
        key={clip.content.text + clip.id}
        rows={3}
        onBlur={(e) => {
          const v = e.currentTarget.value;
          if (v !== clip.content.text) void c.setText(clip.id, { text: v });
        }}
      />
      <Row label={t("inspector.fontSize")}>
        <NumberField
          label={t("inspector.fontSize")}
          value={style.size_permille}
          min={10}
          max={400}
          onCommit={(v) => {
            set({ size_permille: Math.round(v) });
          }}
        />
      </Row>
      <Row label={t("inspector.fontWeight")}>
        <input
          type="checkbox"
          aria-label={t("inspector.fontWeight")}
          checked={style.weight >= 600}
          onChange={(e) => {
            set({ weight: e.currentTarget.checked ? 700 : 400 });
          }}
        />
      </Row>
      <Row label={t("inspector.align")}>
        <Select
          aria-label={t("inspector.align")}
          value={style.align}
          onChange={(e) => {
            set({ align: e.currentTarget.value as TextStyle["align"] });
          }}
        >
          {(["left", "center", "right"] as const).map((a) => (
            <option key={a} value={a}>
              {t(`inspector.align.${a}`)}
            </option>
          ))}
        </Select>
      </Row>
      <Row label={t("inspector.color")}>
        <ColorField
          label={t("inspector.color")}
          value={style.color}
          onCommit={(v) => {
            if (v) set({ color: v });
          }}
        />
      </Row>
      <Row label={t("inspector.background")}>
        <ColorField
          label={t("inspector.background")}
          value={style.background}
          allowNone
          onCommit={(v) => {
            set({ background: v });
          }}
        />
      </Row>
      <Row label={t("inspector.stroke")}>
        <ColorField
          label={t("inspector.stroke")}
          value={style.stroke}
          allowNone
          onCommit={(v) => {
            set({ stroke: v, stroke_permille: v ? (style.stroke_permille ?? 4) : 0 });
          }}
        />
      </Row>
    </Section>
  );
}

// -------------------------------------------------------------------------------- animation

function AnimationTab({ clip, playhead }: { clip: Clip; playhead: number }) {
  const c = useController();
  const t = useT();
  const ct = contentTimeAt(clip, playhead);
  const inside = playhead >= clip.start && playhead < clip.start + clip.duration;
  const seqSummary = useUi((s) => (s.active ? s.model.sequences[s.active] : undefined));
  const rate = seqSummary?.frame_rate ?? "30";
  const hasVideo = clip.content.type !== "media" || clip.content.has_video;
  const props = ANIMATABLE.filter((a) =>
    a.name === "volume_db" ? clip.content.type === "media" && clip.content.has_audio : hasVideo,
  );
  return (
    <div data-testid="animation-tab">
      <p className="ed-hint">{t("inspector.animatedHint")}</p>
      {props.map((spec) => {
        const kfs = keyframesOf(clip, spec.name);
        const here = kfs.find((k) => Math.abs(k.at - playhead) < frameTicks(rate) / 2);
        const value = evalAt(clip.properties[spec.name], ct, spec.dflt);
        return (
          <Section key={spec.name} title={t(PROP_LABEL[spec.name] ?? "inspector.position")}>
            <Row label={t("inspector.keyframes")}>
              <span className="ed-value">{Number(value.toFixed(3))}</span>
              {here ? (
                <IconButton
                  icon="keyframe"
                  pressed
                  label={t("inspector.removeKeyframe")}
                  data-testid={`kf-remove-${spec.name}`}
                  onClick={() => {
                    void c.deleteKeyframe(clip.id, spec.name, here.at);
                  }}
                />
              ) : (
                <IconButton
                  icon="keyframe"
                  label={t("inspector.addKeyframe")}
                  disabled={!inside}
                  data-testid={`kf-add-${spec.name}`}
                  onClick={() => {
                    void c.addKeyframe(clip, spec.name, value);
                  }}
                />
              )}
            </Row>
            {kfs.length === 0 ? (
              <p className="ed-hint">{t("inspector.noKeyframes")}</p>
            ) : (
              <ul className="ed-kf-list" data-testid={`kf-list-${spec.name}`}>
                {kfs.map((k, i) => (
                  <li key={k.at} className={here === k ? "is-here" : undefined}>
                    <button
                      className="ed-kf-time"
                      onClick={() => {
                        c.seek(k.at);
                      }}
                    >
                      {formatTimecode(k.at, rate)}
                    </button>
                    <NumberField
                      label={`${t(PROP_LABEL[spec.name] ?? "inspector.position")} @${String(i + 1)}`}
                      value={k.value}
                      min={spec.min}
                      max={spec.max}
                      step={spec.step}
                      onCommit={(v) => {
                        void c.addKeyframe(clip, spec.name, v, k.at);
                      }}
                    />
                    <Select
                      aria-label={t("inspector.interp")}
                      value={interpKind(k.interp)}
                      onChange={(e) => {
                        void c.setKeyframeInterp(
                          clip.id,
                          spec.name,
                          k.at,
                          interpFromKind(e.currentTarget.value as "linear" | "hold" | "ease"),
                        );
                      }}
                    >
                      {(["linear", "hold", "ease"] as const).map((x) => (
                        <option key={x} value={x}>
                          {t(`inspector.interp.${x}`)}
                        </option>
                      ))}
                    </Select>
                    <IconButton
                      icon="close"
                      label={t("inspector.removeKeyframe")}
                      onClick={() => {
                        void c.deleteKeyframe(clip.id, spec.name, k.at);
                      }}
                    />
                  </li>
                ))}
              </ul>
            )}
          </Section>
        );
      })}
    </div>
  );
}

// ------------------------------------------------------------------------------------ audio

function AudioTab({ clip, playhead }: { clip: Clip; playhead: number }) {
  const c = useController();
  const t = useT();
  const ct = contentTimeAt(clip, playhead);
  const db = evalAt(clip.properties.volume_db, ct, 0);
  const [live, setLive] = useState<number | null>(null);
  const fade = (n: "fade_in" | "fade_out") => evalAt(clip.properties[n], ct, 0);
  const canDetach =
    clip.content.type === "media" && clip.content.has_video && clip.content.has_audio;
  return (
    <>
      <Row label={t("inspector.volume")}>
        <Slider
          label={t("inspector.volume")}
          min={-60}
          max={12}
          step={0.5}
          value={live ?? Math.max(-60, db)}
          onChange={setLive}
          onCommit={(v) => {
            setLive(null);
            void c.setProperty(clip, "volume_db", v);
          }}
        />
        <NumberField
          label={`${t("inspector.volume")} (dB)`}
          value={live ?? db}
          min={-120}
          max={24}
          step={0.5}
          onCommit={(v) => {
            void c.setProperty(clip, "volume_db", v);
          }}
        />
      </Row>
      <Row label={t("inspector.fadeIn")}>
        <NumberField
          label={t("inspector.fadeIn")}
          value={fade("fade_in")}
          min={0}
          max={60}
          step={0.1}
          onCommit={(v) => {
            void c.setProperty(clip, "fade_in", v);
          }}
        />
      </Row>
      <Row label={t("inspector.fadeOut")}>
        <NumberField
          label={t("inspector.fadeOut")}
          value={fade("fade_out")}
          min={0}
          max={60}
          step={0.1}
          onCommit={(v) => {
            void c.setProperty(clip, "fade_out", v);
          }}
        />
      </Row>
      {canDetach && (
        <Button
          data-testid="detach-audio"
          onClick={() => {
            void c.detachAudio();
          }}
        >
          {t("timeline.detachAudio")}
        </Button>
      )}
    </>
  );
}

// ------------------------------------------------------------------------------------ speed

const SPEEDS = ["1/4", "1/2", "3/4", "1", "5/4", "3/2", "2", "3", "4"];

function SpeedTab({ clip }: { clip: Clip }) {
  const c = useController();
  const t = useT();
  const [custom, setCustom] = useState("");
  return (
    <>
      <Row label={t("inspector.speed")}>
        <Select
          aria-label={t("inspector.speed")}
          value={SPEEDS.includes(clip.speed) ? clip.speed : "custom"}
          data-testid="clip-speed"
          onChange={(e) => {
            if (e.currentTarget.value !== "custom")
              void c.setClipSpeed(clip.id, e.currentTarget.value);
          }}
        >
          {!SPEEDS.includes(clip.speed) && <option value="custom">{clip.speed}×</option>}
          {SPEEDS.map((s) => (
            <option key={s} value={s}>
              {s}×
            </option>
          ))}
        </Select>
      </Row>
      <Row label={`${t("inspector.speed")} (a/b)`}>
        <TextInput
          aria-label={`${t("inspector.speed")} (a/b)`}
          value={custom}
          placeholder="7/5"
          onChange={(e) => {
            setCustom(e.currentTarget.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && /^\d+(\/\d+)?$/.test(custom.trim())) {
              void c.setClipSpeed(clip.id, custom.trim());
              setCustom("");
            }
          }}
        />
      </Row>
      <Row label={t("inspector.reversed")}>
        <span>{clip.reversed ? t("common.yes") : t("common.no")}</span>
      </Row>
    </>
  );
}
