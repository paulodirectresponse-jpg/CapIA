import { useEffect, useState } from "react";
import { Button, Dialog, Icon, Splitter, Toasts, type IconName } from "@capia/ui-kit";
import { useController, useUi } from "../context";
import type { PerfKey, PerfSummary } from "../store/perf";
import { useT, type MessageKey } from "../i18n";
import { ExportDialog } from "./ExportDialog";
import { HistoryPopover } from "./HistoryPopover";
import { Inspector } from "./Inspector";
import { MediaPanel } from "./MediaPanel";
import { PreviewPanel } from "./PreviewPanel";
import { ProjectPanel } from "./ProjectPanel";
import { AudioPanel, CaptionsPanel, TextPanel, TransitionsPanel } from "./RailPanels";
import { SettingsDialog } from "./SettingsDialog";
import { TimelinePanel } from "./TimelinePanel";
import { TopBar } from "./TopBar";

declare global {
  interface Window {
    __capiaPerf?: {
      summary(): Record<PerfKey, PerfSummary>;
      samples(key: PerfKey): number[];
      reset(): void;
    };
  }
}

export type RailId = "project" | "media" | "audio" | "text" | "captions" | "transitions";

const RAIL: { id: RailId; icon: IconName; label: MessageKey }[] = [
  { id: "project", icon: "folder", label: "rail.project" },
  { id: "media", icon: "film", label: "rail.media" },
  { id: "audio", icon: "music", label: "rail.audio" },
  { id: "text", icon: "text", label: "rail.text" },
  { id: "captions", icon: "caption", label: "rail.captions" },
  { id: "transitions", icon: "transition", label: "rail.transitions" },
];

function RailContent({ id }: { id: RailId }) {
  switch (id) {
    case "project":
      return <ProjectPanel />;
    case "media":
      return <MediaPanel />;
    case "audio":
      return <AudioPanel />;
    case "text":
      return <TextPanel />;
    case "captions":
      return <CaptionsPanel />;
    case "transitions":
      return <TransitionsPanel />;
  }
}

/** Layout do editor: rail + painel esquerdo | preview + timeline | inspector (todos redimensionáveis). */
export function EditorShell() {
  const c = useController();
  const t = useT();
  const { panels, toasts, exportNonce, lastError } = useUi((s) => ({
    panels: s.prefs.panels,
    toasts: s.toasts,
    exportNonce: s.exportNonce,
    lastError: s.lastError,
  }));
  const [rail, setRail] = useState<RailId>("project");
  const [exportOpen, setExportOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [diagOpen, setDiagOpen] = useState(false);
  // tamanhos "ao vivo" durante o arrasto (o prefs só grava ao soltar)
  const [live, setLive] = useState<Partial<typeof panels>>({});
  const w = { ...panels, ...live };

  // atalho/menu "Exportar" → abre o diálogo
  useEffect(() => {
    if (exportNonce > 0) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reação a pedido de UI (contador)
      setExportOpen(true);
    }
  }, [exportNonce]);

  // atalhos globais configuráveis
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      // com um diálogo modal aberto os atalhos de edição ficam suspensos
      if (document.querySelector("[role=dialog][aria-modal=true]")) return;
      if (c.handleKey(e)) e.preventDefault();
    };
    window.addEventListener("keydown", on);
    return () => {
      window.removeEventListener("keydown", on);
    };
  }, [c]);

  // ganchos de medição para E2E/benchmark (só com `?e2e=1`; sem efeito no produto)
  useEffect(() => {
    if (!new URLSearchParams(window.location.search).has("e2e")) return;
    window.__capiaPerf = {
      summary: () => c.perf.summary(),
      samples: (k) => c.perf.samples(k),
      reset: () => {
        c.perf.reset();
      },
    };
    return () => {
      delete window.__capiaPerf;
    };
  }, [c]);

  // voltar à janela (ex.: depois de reconectar um disco) revalida a disponibilidade da mídia
  useEffect(() => {
    const on = () => {
      void c.refreshAssets();
    };
    window.addEventListener("focus", on);
    return () => {
      window.removeEventListener("focus", on);
    };
  }, [c]);

  const commit = (patch: Partial<typeof panels>) => {
    setLive({});
    c.setPanels(patch);
  };

  return (
    <div className="ed-root" data-testid="editor">
      <TopBar
        onExport={() => {
          setExportOpen(true);
        }}
        onSettings={() => {
          setSettingsOpen(true);
        }}
        onHistory={() => {
          setHistoryOpen((v) => !v);
        }}
      />
      <HistoryPopover
        open={historyOpen}
        onClose={() => {
          setHistoryOpen(false);
        }}
      />
      <div className="ed-body">
        <div className="ed-left">
          <nav
            className="ed-rail"
            role="tablist"
            aria-label={t("rail.label")}
            aria-orientation="vertical"
          >
            {RAIL.map((r) => (
              <button
                key={r.id}
                role="tab"
                className="ed-rail-btn"
                aria-selected={rail === r.id}
                data-testid={`rail-${r.id}`}
                title={t(r.label)}
                onClick={() => {
                  setRail(r.id);
                  if (panels.leftCollapsed) c.setPanels({ leftCollapsed: false });
                }}
              >
                <Icon name={r.icon} size={18} />
                <span>{t(r.label)}</span>
              </button>
            ))}
          </nav>
          {!w.leftCollapsed && (
            <>
              <div
                className="ed-panel"
                style={{ width: w.leftWidth }}
                role="tabpanel"
                data-testid="left-panel"
              >
                <RailContent id={rail} />
              </div>
              <Splitter
                dir="col"
                label={t("rail.label")}
                size={w.leftWidth}
                min={220}
                max={560}
                onResize={(v) => {
                  setLive((p) => ({ ...p, leftWidth: v }));
                }}
                onCommit={(v) => {
                  commit({ leftWidth: v });
                }}
              />
            </>
          )}
        </div>
        <div className="ed-main">
          <div className="ed-upper">
            <PreviewPanel />
            {!w.rightCollapsed && (
              <>
                <Splitter
                  dir="col"
                  sign={-1}
                  label={t("inspector.title")}
                  size={w.rightWidth}
                  min={240}
                  max={560}
                  onResize={(v) => {
                    setLive((p) => ({ ...p, rightWidth: v }));
                  }}
                  onCommit={(v) => {
                    commit({ rightWidth: v });
                  }}
                />
                <div className="ed-inspector" style={{ width: w.rightWidth }}>
                  <Inspector />
                </div>
              </>
            )}
          </div>
          <Splitter
            dir="row"
            sign={-1}
            label={t("timeline.label")}
            size={w.timelineHeight}
            min={160}
            max={720}
            onResize={(v) => {
              setLive((p) => ({ ...p, timelineHeight: v }));
            }}
            onCommit={(v) => {
              commit({ timelineHeight: v });
            }}
          />
          <div className="ed-timeline" style={{ height: w.timelineHeight }}>
            <TimelinePanel />
          </div>
        </div>
      </div>
      {lastError && (
        <div className="ed-statusbar" data-testid="statusbar">
          <span role="status">{lastError.message}</span>
          <Button
            variant="ghost"
            data-testid="error-details"
            onClick={() => {
              setDiagOpen(true);
            }}
          >
            {t("common.details")}
          </Button>
        </div>
      )}
      <Dialog
        title={t("err.diagnostics")}
        open={diagOpen}
        onClose={() => {
          setDiagOpen(false);
        }}
        footer={
          <Button
            onClick={() => {
              setDiagOpen(false);
            }}
          >
            {t("common.close")}
          </Button>
        }
      >
        <pre className="ed-diag" data-testid="error-diagnostics">
          {JSON.stringify(lastError?.body ?? {}, null, 2)}
        </pre>
      </Dialog>
      <ExportDialog
        open={exportOpen}
        onClose={() => {
          setExportOpen(false);
        }}
      />
      <SettingsDialog
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
        }}
      />
      <Toasts
        items={toasts}
        onDismiss={(id) => {
          c.dismissToast(id);
        }}
      />
    </div>
  );
}
