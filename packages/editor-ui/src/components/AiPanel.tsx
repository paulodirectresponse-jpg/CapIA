import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, EmptyState, Icon, Select, Tabs } from "@capia/ui-kit";
import type { DemandSpec, SpecField, SpecItem } from "@capia/engine-bindings";
import { useAi, useController, useUi } from "../context";
import { useT, type MessageKey } from "../i18n";

/** Painel de IA do rail: chat pontual, ferramentas locais/assistidas, referência e briefing. */
export function AiPanel({ onOpenSettings }: { onOpenSettings: () => void }) {
  const c = useController();
  const t = useT();
  const [tab, setTab] = useState<"chat" | "tools" | "brief">("chat");
  const { status, offer } = useAi((s) => ({ status: s.status, offer: s.offer }));
  useEffect(() => {
    void c.ai.refresh();
  }, [c]);
  const off = status !== null && !status.enabled;
  return (
    <div className="ed-panel-body ed-ai" data-testid="ai-panel">
      {off && (
        <p role="status" className="ed-warn" data-testid="ai-off-banner">
          {t("ai.off.banner")}
        </p>
      )}
      <Tabs
        label={t("ai.title")}
        active={tab}
        onSelect={(v) => {
          setTab(v as "chat" | "tools" | "brief");
        }}
        items={[
          { id: "chat", label: t("ai.chat.title") },
          { id: "tools", label: t("ai.tools.title") },
          { id: "brief", label: t("ai.demand.title") },
        ]}
      />
      {offer && <OfferCard />}
      {tab === "chat" && <ChatTab onOpenSettings={onOpenSettings} />}
      {tab === "tools" && <ToolsTab />}
      {tab === "brief" && <BriefTab />}
      <div className="ed-row">
        <Button data-testid="ai-open-settings" onClick={onOpenSettings}>
          <Icon name="settings" /> {t("ai.settings.open")}
        </Button>
      </div>
    </div>
  );
}

function OfferCard() {
  const c = useController();
  const t = useT();
  const offer = useAi((s) => s.offer);
  if (!offer) return null;
  const count = Number(offer.summary.cue_count ?? offer.summary.cut_count ?? 0);
  const key: MessageKey = offer.label === "captions" ? "ai.offer.captions" : "ai.offer.silence";
  return (
    <div className="ed-ai-card" role="group" data-testid="ai-offer">
      <p>{t(key, { count })}</p>
      <div className="ed-row">
        <Button
          variant="primary"
          data-testid="ai-offer-apply"
          onClick={() => {
            void c.ai.applyOffer();
          }}
        >
          {t("ai.offer.apply")}
        </Button>
        <Button
          data-testid="ai-offer-discard"
          onClick={() => {
            c.ai.discardOffer();
          }}
        >
          {t("ai.offer.discard")}
        </Button>
      </div>
    </div>
  );
}

function ChatTab({ onOpenSettings }: { onOpenSettings: () => void }) {
  const c = useController();
  const t = useT();
  const { messages, chatTask, pending, mode, status } = useAi((s) => ({
    messages: s.messages,
    chatTask: s.chatTask,
    pending: s.pending,
    mode: s.mode,
    status: s.status,
  }));
  const [text, setText] = useState("");
  const logRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length]);
  const usable = status?.enabled === true && status.any_usable_model;
  const submit = () => {
    const v = text.trim();
    if (!v) return;
    setText("");
    void c.ai.send(v);
  };
  return (
    <div className="ed-ai-chat" data-testid="ai-chat">
      {!usable && (
        <p className="ed-hint" data-testid="ai-not-configured">
          {t("ai.notConfigured")}{" "}
          <Button variant="ghost" onClick={onOpenSettings}>
            {t("ai.settings.open")}
          </Button>
        </p>
      )}
      <div className="ed-row">
        <Select
          label={t("ai.chat.mode")}
          value={mode}
          data-testid="ai-mode"
          onChange={(e) => {
            c.ai.setMode(e.currentTarget.value === "auto" ? "auto" : "ask");
          }}
        >
          <option value="ask">{t("ai.chat.mode.ask")}</option>
          <option value="auto">{t("ai.chat.mode.auto")}</option>
        </Select>
        <Button
          variant="ghost"
          data-testid="ai-clear"
          onClick={() => {
            c.ai.clearChat();
          }}
        >
          {t("ai.chat.clear")}
        </Button>
      </div>
      <div ref={logRef} className="ed-ai-log" role="log" aria-live="polite" data-testid="ai-log">
        {messages.length === 0 && <EmptyState icon="sparkle" title={t("ai.chat.empty")} />}
        {messages.map((m) => (
          <div key={m.id} className={`ed-ai-msg ed-ai-${m.role}`} data-testid={`ai-msg-${m.role}`}>
            {m.role === "tool" ? (
              <span className="ed-ai-tool">
                <Icon name="sparkle" />{" "}
                {m.tool?.state === "started"
                  ? t("ai.chat.toolRunning", { name: m.text })
                  : t("ai.chat.toolDone", { name: m.text })}
                {m.tool?.state === "failed" && <Icon name="warning" />}
              </span>
            ) : (
              <span>{m.text}</span>
            )}
          </div>
        ))}
      </div>
      {pending && (
        <div className="ed-ai-card" role="alertdialog" data-testid="ai-approval">
          <strong>{t("ai.chat.approvalTitle", { label: pending.plan.label })}</strong>
          <p>{t("ai.chat.operations", { count: pending.plan.operations })}</p>
          <div className="ed-row">
            <Button
              variant="primary"
              data-testid="ai-approve"
              onClick={() => {
                void c.ai.approve();
              }}
            >
              {t("ai.chat.approve")}
            </Button>
            <Button
              data-testid="ai-reject"
              onClick={() => {
                void c.ai.reject();
              }}
            >
              {t("ai.chat.reject")}
            </Button>
          </div>
        </div>
      )}
      <div className="ed-ai-input">
        <textarea
          data-testid="ai-input"
          aria-label={t("ai.chat.title")}
          placeholder={t("ai.chat.placeholder")}
          value={text}
          rows={2}
          onChange={(e) => {
            setText(e.currentTarget.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
        />
        {chatTask && !pending ? (
          <Button
            data-testid="ai-stop"
            onClick={() => {
              void c.ai.cancel(chatTask);
            }}
          >
            {t("ai.chat.cancel")}
          </Button>
        ) : (
          <Button
            variant="primary"
            data-testid="ai-send"
            disabled={!usable || pending !== null || text.trim() === ""}
            onClick={submit}
          >
            {t("ai.chat.send")}
          </Button>
        )}
      </div>
    </div>
  );
}

function useVideoAssets() {
  const assets = useUi((s) => s.model.assets);
  return useMemo(
    () => Object.values(assets).filter((a) => a.kind === "video" && a.has_file),
    [assets],
  );
}

function JobsList() {
  const c = useController();
  const t = useT();
  const jobs = useAi((s) => s.jobs);
  const list = Object.values(jobs)
    .filter((j) => j.kind !== "assistant")
    .slice(-6)
    .reverse();
  if (list.length === 0) return null;
  const kindKey = (k: string) => `ai.job.kind.${k}` as MessageKey;
  const label = (k: string): string => {
    try {
      return t(kindKey(k));
    } catch {
      return k;
    }
  };
  return (
    <ul className="ed-ai-jobs" data-testid="ai-jobs">
      {list.map((j) => (
        <li key={j.id} data-state={j.state} data-testid={`ai-job-${j.kind}`}>
          <span>{label(j.kind)}</span>
          <Badge>
            {j.state === "running"
              ? j.total
                ? `${String(j.done ?? 0)}/${String(j.total)}`
                : t("ai.job.running")
              : j.state === "done"
                ? t("ai.job.done")
                : j.state === "cancelled"
                  ? t("ai.job.cancelled")
                  : t("ai.job.failed")}
          </Badge>
          {j.error && <small role="alert">{j.error.message}</small>}
          {j.state === "running" && (
            <Button
              variant="ghost"
              onClick={() => {
                void c.ai.cancel(j.id);
              }}
            >
              {t("ai.job.cancel")}
            </Button>
          )}
        </li>
      ))}
    </ul>
  );
}

function ToolsTab() {
  const c = useController();
  const t = useT();
  const videos = useVideoAssets();
  const active = useUi((s) => s.active);
  const references = useAi((s) => s.references);
  const [asset, setAsset] = useState("");
  const selected = asset || videos[0]?.id || "";
  const grammar = selected ? references[selected] : undefined;
  return (
    <div data-testid="ai-tools">
      {videos.length === 0 ? (
        <p className="ed-hint">{t("ai.tools.noAsset")}</p>
      ) : (
        <Select
          label={t("ai.tools.asset")}
          value={selected}
          data-testid="ai-asset"
          onChange={(e) => {
            setAsset(e.currentTarget.value);
          }}
        >
          {videos.map((a) => (
            <option key={a.id} value={a.id}>
              {a.name}
            </option>
          ))}
        </Select>
      )}
      <div className="ed-ai-actions">
        <Button
          disabled={!selected}
          data-testid="ai-transcribe"
          onClick={() => {
            void c.ai.transcribe(selected);
          }}
        >
          {t("ai.tools.transcribe")}
        </Button>
        <Button
          disabled={!selected || !active}
          data-testid="ai-captions"
          onClick={() => {
            if (active) void c.ai.planCaptions(active, selected);
          }}
        >
          <Icon name="caption" /> {t("ai.tools.captions")}
        </Button>
        <Button
          disabled={!selected || !active}
          data-testid="ai-silence"
          onClick={() => {
            if (active) void c.ai.planSilence(active, selected);
          }}
        >
          <Icon name="scissors" /> {t("ai.tools.silence")}
        </Button>
        <Button
          disabled={!selected}
          data-testid="ai-scenes"
          onClick={() => {
            void c.ai.detectScenes(selected);
          }}
        >
          {t("ai.tools.scenes")}
        </Button>
        <Button
          disabled={!selected}
          data-testid="ai-reference"
          onClick={() => {
            void c.ai.analyzeReference(selected);
          }}
        >
          {t("ai.tools.reference")}
        </Button>
      </div>
      <p className="ed-hint">{t("ai.tools.localHint")}</p>
      {!active && <p className="ed-hint">{t("ai.tools.needSequence")}</p>}
      <JobsList />
      {grammar && (
        <div className="ed-ai-card" data-testid="ai-grammar">
          <h3 className="ed-subhead">{t("ai.reference.title")}</h3>
          <p>
            {t("ai.reference.shots", {
              count: grammar.cut_rhythm.shot_count,
              cpm: grammar.cut_rhythm.cuts_per_minute,
              median: (grammar.cut_rhythm.median_shot_us / 1e6).toFixed(2),
            })}
          </p>
          <p>
            {t("ai.reference.transitions", {
              cuts: grammar.transitions.cuts,
              dissolves: grammar.transitions.dissolves,
              fades: grammar.transitions.fades,
            })}
          </p>
          {grammar.audio && (
            <p>
              {t("ai.reference.audio", {
                speech: Math.round(grammar.audio.speech_ratio_permille / 10),
                range: grammar.audio.dynamic_range_db,
              })}
            </p>
          )}
          {grammar.speech && (
            <p>{t("ai.reference.speech", { wpm: grammar.speech.words_per_minute })}</p>
          )}
          <ul>
            {grammar.structure.map((b) => (
              <li key={b.name}>
                {b.name}: {(b.start_us / 1e6).toFixed(1)}–{(b.end_us / 1e6).toFixed(1)} s ·{" "}
                {b.shots}
              </li>
            ))}
          </ul>
          {grammar.provenance.fully_local && <Badge>{t("ai.reference.local")}</Badge>}
        </div>
      )}
    </div>
  );
}

const FIELDS = [
  "product",
  "audience",
  "offer",
  "objective",
  "tone",
  "platform",
  "duration_and_format",
  "cta",
] as const;

function BasisBadge({ basis }: { basis: SpecField["basis"] }) {
  const t = useT();
  return <Badge>{t(`ai.demand.basis.${basis}` as MessageKey)}</Badge>;
}

function Sources({ sources }: { sources: SpecField["sources"] }) {
  const t = useT();
  if (sources.length === 0) return null;
  return (
    <details className="ed-ai-sources">
      <summary>{t("ai.demand.sources")}</summary>
      <ul>
        {sources.map((s, i) => (
          <li key={`${s.doc_id}-${s.unit_id}-${String(i)}`}>
            <strong>{s.doc_name}</strong>
            {s.page ? ` · p.${String(s.page)}` : ""}
            {s.t_us !== undefined ? ` · ${(s.t_us / 1e6).toFixed(1)} s` : ""}
            <q>{s.quote}</q>
          </li>
        ))}
      </ul>
    </details>
  );
}

function ItemList({ title, items }: { title: string; items: SpecItem[] }) {
  if (items.length === 0) return null;
  return (
    <div>
      <h4 className="ed-subhead">{title}</h4>
      <ul>
        {items.map((it, i) => (
          <li key={`${it.text}-${String(i)}`}>
            {it.text} <BasisBadge basis={it.basis} />
            <Sources sources={it.sources} />
          </li>
        ))}
      </ul>
    </div>
  );
}

function SpecView({ spec }: { spec: DemandSpec }) {
  const t = useT();
  const v = spec.verification;
  return (
    <div className="ed-ai-card" data-testid="ai-spec">
      <h3 className="ed-subhead">{spec.title ?? t("ai.demand.title")}</h3>
      <p className="ed-hint" data-testid="ai-spec-verification">
        {t("ai.demand.verification", {
          ok: v.sources_verified,
          total: v.sources_proposed,
          dropped: v.sources_dropped,
        })}
      </p>
      <dl>
        {FIELDS.map((f) => {
          const field = spec[f];
          return (
            <div key={f} data-testid={`ai-spec-${f}`}>
              <dt>{t(`ai.demand.field.${f}` as MessageKey)}</dt>
              <dd>
                {field.value ?? <em>{t("ai.demand.empty")}</em>} <BasisBadge basis={field.basis} />
                <Sources sources={field.sources} />
              </dd>
            </div>
          );
        })}
      </dl>
      <ItemList title={t("ai.demand.claims")} items={spec.key_claims} />
      <ItemList title={t("ai.demand.mustInclude")} items={spec.must_include} />
      <ItemList title={t("ai.demand.mustAvoid")} items={spec.must_avoid} />
      <ItemList title={t("ai.demand.constraints")} items={spec.constraints} />
      {spec.open_questions.length > 0 && (
        <div data-testid="ai-spec-questions">
          <h4 className="ed-subhead">{t("ai.demand.questions")}</h4>
          <ul>
            {spec.open_questions.map((q, i) => (
              <li key={`${q.question}-${String(i)}`}>{q.question}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function BriefTab() {
  const c = useController();
  const t = useT();
  const videos = useVideoAssets();
  const demand = useAi((s) => s.demand);
  const [paths, setPaths] = useState("");
  const [useVideo, setUseVideo] = useState(false);
  const [video, setVideo] = useState("");
  useEffect(() => {
    void c.ai.loadDemand();
  }, [c]);
  const list = paths
    .split("\n")
    .map((p) => p.trim())
    .filter(Boolean);
  const vid = video || videos[0]?.id || "";
  return (
    <div data-testid="ai-brief">
      <label className="ed-field">
        <span>{t("ai.demand.files")}</span>
        <textarea
          data-testid="ai-brief-paths"
          rows={3}
          value={paths}
          onChange={(e) => {
            setPaths(e.currentTarget.value);
          }}
        />
      </label>
      {c.platform.native && (
        <Button
          onClick={() => {
            void c.platform
              .pickFiles({
                title: t("ai.demand.pick"),
                multiple: true,
                filters: [{ name: "Brief", extensions: ["docx", "pdf", "txt", "md"] }],
              })
              .then((r) => {
                if (r) setPaths((p) => [...p.split("\n").filter(Boolean), ...r].join("\n"));
              });
          }}
        >
          {t("ai.demand.pick")}
        </Button>
      )}
      {videos.length > 0 && (
        <label className="ed-row">
          <input
            type="checkbox"
            data-testid="ai-brief-usevideo"
            checked={useVideo}
            onChange={(e) => {
              setUseVideo(e.currentTarget.checked);
            }}
          />
          <span>{t("ai.demand.useVideo")}</span>
          {useVideo && (
            <Select
              label={t("ai.tools.asset")}
              value={vid}
              onChange={(e) => {
                setVideo(e.currentTarget.value);
              }}
            >
              {videos.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </Select>
          )}
        </label>
      )}
      <Button
        variant="primary"
        data-testid="ai-brief-interpret"
        disabled={list.length === 0 && !useVideo}
        onClick={() => {
          void c.ai.interpretDemand(list, useVideo && vid ? [vid] : []);
        }}
      >
        {t("ai.demand.interpret")}
      </Button>
      <JobsList />
      {demand && <SpecView spec={demand} />}
    </div>
  );
}
