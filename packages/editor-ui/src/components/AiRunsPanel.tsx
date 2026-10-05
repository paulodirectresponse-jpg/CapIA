import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Select, Tabs } from "@capia/ui-kit";
import type {
  MemoryItemView,
  MemoryScopeName,
  RunStageName,
  RunStatusName,
  RunSummary,
} from "@capia/engine-bindings";
import { useAi, useController, useUi } from "../context";
import { useT, type MessageKey } from "../i18n";

const STAGES: RunStageName[] = [
  "understand",
  "plan",
  "validate_plan",
  "acquire",
  "edit",
  "review",
  "done",
];

/** Custo em micros da moeda → texto curto. Preço desconhecido é dito, nunca "zero". */
function money(micros: number): string {
  return (micros / 1_000_000).toFixed(2);
}

const tone = (s: RunStatusName): "warning" | "success" | "danger" | undefined => {
  switch (s) {
    case "completed":
      return "success";
    case "failed":
      return "danger";
    case "waiting_user":
      return "warning";
    default:
      return undefined;
  }
};

function StatusBadge({ status }: { status: RunStatusName }) {
  const t = useT();
  const label = t(`ai.runs.status.${status}` as MessageKey);
  const tn = tone(status);
  return tn ? <Badge tone={tn}>{label}</Badge> : <Badge>{label}</Badge>;
}

/** Execuções de IA (Fase 5): lista, nova execução, plano/aprovações, progresso e resultado. */
export function AiRunsPanel() {
  const c = useController();
  const t = useT();
  const [tab, setTab] = useState<"runs" | "memory" | "sources">("runs");
  useEffect(() => {
    void c.ai.refreshRuns();
    void c.ai.refreshMemory();
    void c.ai.refreshGateway();
    c.ai.startRunPolling();
    return () => {
      c.ai.stopRunPolling();
    };
  }, [c]);
  return (
    <div data-testid="ai-runs-panel">
      <Tabs
        label={t("ai.runs.title")}
        active={tab}
        onSelect={(v) => {
          setTab(v as "runs" | "memory" | "sources");
        }}
        items={[
          { id: "runs", label: t("ai.runs.title") },
          { id: "memory", label: t("ai.memory.title") },
          { id: "sources", label: t("ai.sources.title") },
        ]}
      />
      {tab === "runs" && <RunsTab />}
      {tab === "memory" && <MemoryTab />}
      {tab === "sources" && <SourcesTab />}
    </div>
  );
}

function RunsTab() {
  const selected = useAi((s) => s.selectedRun);
  return selected ? <RunDetail /> : <RunList />;
}

function RunList() {
  const c = useController();
  const t = useT();
  const runs = useAi((s) => s.runs);
  return (
    <div data-testid="ai-run-list">
      <NewRun />
      {runs.length === 0 ? (
        <p className="ed-hint" data-testid="ai-run-empty">
          {t("ai.runs.empty")}
        </p>
      ) : (
        <ul className="ed-ai-jobs">
          {runs.map((r) => (
            <li key={r.id}>
              <button
                type="button"
                className="ed-link"
                data-testid={`ai-run-${r.id}`}
                onClick={() => {
                  void c.ai.selectRun(r.id);
                }}
              >
                {r.id}
              </button>{" "}
              <StatusBadge status={r.status} />{" "}
              <span className="ed-hint">{t(`ai.runs.stage.${r.stage}` as MessageKey)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function NewRun() {
  const c = useController();
  const t = useT();
  const assets = useUi((s) => s.model.assets);
  const raw = useMemo(
    () =>
      Object.values(assets)
        .filter((a) => a.kind === "video" && a.has_file)
        .map((a) => a.id),
    [assets],
  );
  const [brief, setBrief] = useState("");
  const [duration, setDuration] = useState("30");
  const [approve, setApprove] = useState(true);
  const [gateway, setGateway] = useState(false);
  const [generation, setGeneration] = useState(false);
  const canStart = raw.length > 0 && brief.trim().length > 0;
  return (
    <div className="ed-ai-card" role="group" aria-label={t("ai.runs.new")} data-testid="ai-run-new">
      <label className="ed-field">
        <span>{t("ai.runs.brief")}</span>
        <textarea
          rows={4}
          value={brief}
          placeholder={t("ai.runs.briefPlaceholder")}
          data-testid="ai-run-brief"
          onChange={(e) => {
            setBrief(e.currentTarget.value);
          }}
        />
      </label>
      <label className="ed-field">
        <span>{t("ai.runs.duration")}</span>
        <input
          type="number"
          min={5}
          max={600}
          value={duration}
          data-testid="ai-run-duration"
          onChange={(e) => {
            setDuration(e.currentTarget.value);
          }}
        />
      </label>
      <p className="ed-hint" data-testid="ai-run-assets">
        {raw.length === 0 ? t("ai.runs.noAssets") : t("ai.runs.assets", { count: raw.length })}
      </p>
      <label className="ed-check">
        <input
          type="checkbox"
          checked={approve}
          data-testid="ai-run-approve-plan"
          onChange={(e) => {
            setApprove(e.currentTarget.checked);
          }}
        />{" "}
        {t("ai.runs.planApproval")}
      </label>
      <label className="ed-check">
        <input
          type="checkbox"
          checked={gateway}
          onChange={(e) => {
            setGateway(e.currentTarget.checked);
          }}
        />{" "}
        {t("ai.runs.allowGateway")}
      </label>
      <label className="ed-check">
        <input
          type="checkbox"
          checked={generation}
          onChange={(e) => {
            setGeneration(e.currentTarget.checked);
          }}
        />{" "}
        {t("ai.runs.allowGeneration")}
      </label>
      <p className="ed-hint">{t("ai.runs.privacy")}</p>
      <Button
        variant="primary"
        disabled={!canStart}
        data-testid="ai-run-start"
        onClick={() => {
          const secs = Number.parseInt(duration, 10);
          void c.ai.createRun({
            briefText: brief.trim(),
            assets: raw,
            deliverables: [{ key: "main", ...(secs > 0 ? { maxDurationS: secs } : {}) }],
            planApproval: approve ? "always" : "auto",
            allowGateway: gateway,
            allowGeneration: generation,
          });
        }}
      >
        {t("ai.runs.start")}
      </Button>
    </div>
  );
}

function Progress({ run }: { run: RunSummary }) {
  const t = useT();
  const idx = STAGES.indexOf(run.stage === "correct" ? "review" : run.stage);
  return (
    <ol className="ed-ai-progress" aria-label={t("ai.runs.progress")} data-testid="ai-run-progress">
      {STAGES.map((s, i) => (
        <li
          key={s}
          aria-current={s === run.stage ? "step" : undefined}
          data-done={i < idx || run.status === "completed" ? "true" : "false"}
        >
          {t(`ai.runs.stage.${s}` as MessageKey)}
        </li>
      ))}
    </ol>
  );
}

function RunDetail() {
  const c = useController();
  const t = useT();
  const detail = useAi((s) => s.runDetail);
  const [undoInfo, setUndoInfo] = useState<{ conflicts: number } | null>(null);
  if (!detail) return null;
  const run = detail.run;
  const active = run.status === "running" || run.status === "pending";
  const u = run.usage;
  return (
    <div data-testid="ai-run-detail">
      <Button
        variant="ghost"
        data-testid="ai-run-back"
        onClick={() => {
          setUndoInfo(null);
          void c.ai.selectRun(null);
        }}
      >
        {t("ai.runs.back")}
      </Button>
      <p>
        <StatusBadge status={run.status} /> <span className="ed-hint">{run.id}</span>
      </p>
      <Progress run={run} />
      <p className="ed-hint" data-testid="ai-run-cost">
        {t("ai.runs.cost", { cost: money(u.cost_micros) })}
        {u.unknown_cost_calls > 0 && (
          <> · {t("ai.runs.costUnknown", { count: u.unknown_cost_calls })}</>
        )}
      </p>
      <p className="ed-hint">
        {t("ai.runs.usage", { calls: u.provider_calls, loops: u.review_loops, replans: u.replans })}
      </p>
      {run.error && (
        <p role="alert" className="ed-warn" data-testid="ai-run-error">
          {t("ai.runs.error", { message: run.error.message })}
        </p>
      )}
      {run.pending && (
        <div className="ed-ai-card" role="group" data-testid="ai-run-decision">
          <strong>{t("ai.runs.decision")}</strong>
          <p>{run.pending.question}</p>
          {run.pending.consequences && (
            <p className="ed-hint">
              {t("ai.runs.consequences", { text: run.pending.consequences })}
            </p>
          )}
          <div className="ed-row">
            {run.pending.options.map((o) => (
              <Button
                key={o.id}
                variant={o.id === "approve" ? "primary" : "default"}
                data-testid={`ai-run-option-${o.id}`}
                onClick={() => {
                  void c.ai.decide(o.id);
                }}
              >
                {o.label}
              </Button>
            ))}
          </div>
        </div>
      )}
      <div className="ed-ai-actions">
        {active && (
          <Button
            data-testid="ai-run-pause"
            onClick={() => {
              void c.ai.pauseRun(run.id);
            }}
          >
            {t("ai.runs.pause")}
          </Button>
        )}
        {run.status === "paused" && (
          <Button
            variant="primary"
            data-testid="ai-run-resume"
            onClick={() => {
              void c.ai.resumeRun(run.id);
            }}
          >
            {t("ai.runs.resume")}
          </Button>
        )}
        {(active || run.status === "paused" || run.status === "waiting_user") && (
          <Button
            data-testid="ai-run-cancel"
            onClick={() => {
              void c.ai.cancelRun(run.id);
            }}
          >
            {t("ai.runs.cancel")}
          </Button>
        )}
        {(run.status === "completed" || run.status === "failed" || run.status === "cancelled") && (
          <Button
            data-testid="ai-run-rerun"
            onClick={() => {
              void c.ai.rerun(run.id);
            }}
          >
            {t("ai.runs.rerun")}
          </Button>
        )}
        {run.status === "completed" && (
          <Button
            data-testid="ai-run-variants"
            onClick={() => {
              void c.ai.makeVariants(run.id, 3, ["hooks"]);
            }}
          >
            {t("ai.runs.variants")}
          </Button>
        )}
      </div>
      {run.sequences.length > 0 && (
        <div data-testid="ai-run-sequences">
          <strong>{t("ai.runs.sequences")}</strong>
          <ul className="ed-ai-jobs">
            {run.sequences
              .filter((s) => s.role !== "superseded")
              .map((s) => (
                <li key={s.sequence_id}>
                  {s.deliverable} <span className="ed-hint">({s.role})</span>{" "}
                  <Button
                    variant="ghost"
                    data-testid={`ai-run-open-${s.sequence_id}`}
                    onClick={() => {
                      void c.openSequence(s.sequence_id);
                    }}
                  >
                    {t("ai.runs.openSequence")}
                  </Button>
                </li>
              ))}
          </ul>
          <div className="ed-row">
            <Button
              data-testid="ai-run-undo"
              onClick={() => {
                void (async () => {
                  const rep = await c.undoRunReport(run.id);
                  if (rep && rep.conflicts > 0) setUndoInfo({ conflicts: rep.conflicts });
                  else await c.undoRun(run.id, "safe");
                })();
              }}
            >
              {t("ai.runs.undo")}
            </Button>
            {undoInfo && (
              <>
                <span role="status" className="ed-warn" data-testid="ai-run-undo-conflicts">
                  {t("ai.runs.undoConflicts", { count: undoInfo.conflicts })}
                </span>
                <Button
                  data-testid="ai-run-undo-partial"
                  onClick={() => {
                    setUndoInfo(null);
                    void c.undoRun(run.id, "partial");
                  }}
                >
                  {t("ai.runs.undoPartial")}
                </Button>
              </>
            )}
          </div>
        </div>
      )}
      {detail.provenance.length > 0 && (
        <p className="ed-hint" data-testid="ai-run-provenance">
          {t("ai.runs.provenance", { count: detail.provenance.length })}
        </p>
      )}
    </div>
  );
}

const SCOPES: MemoryScopeName[] = ["project", "user"];

function MemoryTab() {
  const c = useController();
  const t = useT();
  const items = useAi((s) => s.memory);
  const [text, setText] = useState("");
  const [scope, setScope] = useState<MemoryScopeName>("project");
  const proposed = items.filter((i) => i.status === "proposed");
  const active = items.filter((i) => i.status === "active" && i.scope !== "system");
  return (
    <div data-testid="ai-memory">
      {items.length === 0 && <p className="ed-hint">{t("ai.memory.empty")}</p>}
      {proposed.length > 0 && (
        <section data-testid="ai-memory-proposed">
          <strong>{t("ai.memory.proposed")}</strong>
          <ul className="ed-ai-jobs">
            {proposed.map((i) => (
              <MemoryRow key={i.id} item={i} proposed />
            ))}
          </ul>
        </section>
      )}
      {active.length > 0 && (
        <section data-testid="ai-memory-active">
          <strong>{t("ai.memory.active")}</strong>
          <ul className="ed-ai-jobs">
            {active.map((i) => (
              <MemoryRow key={i.id} item={i} />
            ))}
          </ul>
        </section>
      )}
      <div className="ed-row">
        <Select
          label={t("ai.memory.title")}
          value={scope}
          onChange={(e) => {
            setScope(e.currentTarget.value as MemoryScopeName);
          }}
        >
          {SCOPES.map((s) => (
            <option key={s} value={s}>
              {t(`ai.memory.scope.${s}` as MessageKey)}
            </option>
          ))}
        </Select>
        <input
          value={text}
          placeholder={t("ai.memory.placeholder")}
          data-testid="ai-memory-input"
          onChange={(e) => {
            setText(e.currentTarget.value);
          }}
        />
        <Button
          data-testid="ai-memory-add"
          disabled={!text.trim()}
          onClick={() => {
            void c.ai.addMemory(scope, text);
            setText("");
          }}
        >
          {t("ai.memory.add")}
        </Button>
      </div>
    </div>
  );
}

function MemoryRow({ item, proposed = false }: { item: MemoryItemView; proposed?: boolean }) {
  const c = useController();
  const t = useT();
  return (
    <li data-testid={`ai-memory-${item.id}`}>
      <Badge>{t(`ai.memory.scope.${item.scope}` as MessageKey)}</Badge> {item.content}{" "}
      {proposed ? (
        <>
          <Button
            variant="primary"
            data-testid={`ai-memory-approve-${item.id}`}
            onClick={() => {
              void c.ai.approveMemory(item.id);
            }}
          >
            {t("ai.memory.approve")}
          </Button>
          <Button
            data-testid={`ai-memory-reject-${item.id}`}
            onClick={() => {
              void c.ai.rejectMemory(item.id);
            }}
          >
            {t("ai.memory.reject")}
          </Button>
        </>
      ) : (
        <>
          <Button
            variant="ghost"
            onClick={() => {
              void c.ai.archiveMemory(item.id);
            }}
          >
            {t("ai.memory.archive")}
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              void c.ai.deleteMemory(item.id);
            }}
          >
            {t("ai.memory.delete")}
          </Button>
        </>
      )}
    </li>
  );
}

function SourcesTab() {
  const c = useController();
  const t = useT();
  const gw = useAi((s) => s.gateway);
  return (
    <div data-testid="ai-sources">
      {gw && gw.adapters.length === 0 && <p className="ed-hint">{t("ai.sources.none")}</p>}
      <ul className="ed-ai-jobs">
        {gw?.adapters.map((a) => (
          <li key={a.id}>
            {a.id} {a.paid && <Badge tone="warning">{t("ai.sources.paid")}</Badge>}{" "}
            <Button
              variant="ghost"
              data-testid={`ai-source-${a.id}`}
              onClick={() => {
                void c.ai.setSourceEnabled(a.id, !a.enabled);
              }}
            >
              {a.enabled ? t("ai.sources.enabled") : t("ai.sources.disabled")}
            </Button>
          </li>
        ))}
      </ul>
      <div className="ed-row">
        <strong>{t("ai.sources.generation")}</strong>
        <Button
          data-testid="ai-generation-toggle"
          onClick={() => {
            void c.ai.setGenerationEnabled(!(gw?.generation.enabled ?? false));
          }}
        >
          {gw?.generation.enabled ? t("ai.sources.enabled") : t("ai.sources.disabled")}
        </Button>
      </div>
      <p className="ed-hint">{t("ai.sources.generationHint")}</p>
    </div>
  );
}
