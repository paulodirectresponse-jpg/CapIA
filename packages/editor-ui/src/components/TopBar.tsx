import { useState } from "react";
import { Badge, Button, IconButton, Spinner } from "@capia/ui-kit";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { formatBinding, resolveBindings } from "../lib/keymap";

export function TopBar({
  onExport,
  onSettings,
  onHistory,
}: {
  onExport: () => void;
  onSettings: () => void;
  onHistory: () => void;
}) {
  const c = useController();
  const t = useT();
  const { name, canUndo, canRedo, save, imports, keymap } = useUi((s) => ({
    name: s.model.project?.name ?? "",
    canUndo: s.model.canUndo,
    canRedo: s.model.canRedo,
    save: s.save,
    imports: s.pendingImports,
    keymap: s.prefs.keymap,
  }));
  const exporting = useUi((s) => s.exportRun !== null && !s.exportRun.finished);
  const b = resolveBindings(keymap);
  const [closing, setClosing] = useState(false);
  return (
    <header className="ed-topbar" role="banner">
      <div className="ed-topbar-title">
        <strong data-testid="project-name">{name}</strong>
      </div>
      <IconButton
        icon="undo"
        label={t("topbar.undo")}
        shortcut={formatBinding(b.undo[0] ?? "")}
        disabled={!canUndo}
        data-testid="undo"
        onClick={() => {
          void c.undo();
        }}
      />
      <IconButton
        icon="redo"
        label={t("topbar.redo")}
        shortcut={formatBinding(b.redo[0] ?? "")}
        disabled={!canRedo}
        data-testid="redo"
        onClick={() => {
          void c.redo();
        }}
      />
      <IconButton
        icon="history"
        label={t("topbar.history")}
        data-testid="history-btn"
        onClick={onHistory}
      />
      <div className="ed-spacer" />
      {imports > 0 && (
        <Badge>
          <Spinner label={t("media.importing")} /> {t("topbar.jobs", { count: imports })}
        </Badge>
      )}
      {exporting && <Badge tone="warning">{t("export.progress", { done: "…", total: "…" })}</Badge>}
      <span
        data-testid="save-state"
        role="status"
        style={{
          fontSize: "var(--fs-sm)",
          color: save === "error" ? "var(--danger)" : "var(--text-muted)",
        }}
      >
        {save === "saving"
          ? t("topbar.saving")
          : save === "error"
            ? t("topbar.saveError")
            : t("topbar.saved")}
      </span>
      <IconButton
        icon="settings"
        label={t("topbar.settings")}
        data-testid="settings-btn"
        onClick={onSettings}
      />
      <Button
        variant="primary"
        data-testid="export-btn"
        onClick={onExport}
        title={formatBinding(b.export[0] ?? "")}
      >
        {t("topbar.export")}
      </Button>
      <Button
        variant="ghost"
        disabled={closing}
        data-testid="close-project"
        onClick={() => {
          setClosing(true);
          void c.closeProject().finally(() => {
            setClosing(false);
          });
        }}
      >
        {t("common.close")}
      </Button>
    </header>
  );
}
