import { useEffect, useRef } from "react";
import { Button, EmptyState } from "@capia/ui-kit";
import { useController, useUi } from "../context";
import { useT } from "../i18n";

/** Painel de histórico: lista as transações; clicar numa entrada desfaz/refaz até ela. */
export function HistoryPopover({ open, onClose }: { open: boolean; onClose: () => void }) {
  const c = useController();
  const t = useT();
  const { history, revision } = useUi((s) => ({ history: s.history, revision: s.model.revision }));
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (open) void c.loadHistory();
  }, [open, revision, c]);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (
        ref.current &&
        target &&
        !ref.current.contains(target) &&
        !target.closest("[data-testid=history-btn]")
      )
        onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  if (!open) return null;
  const entries = history?.entries ?? [];
  const cursor = history?.cursor ?? 0;

  const jumpTo = async (index: number) => {
    // `cursor` = quantas entradas estão aplicadas; alvo = aplicadas até `index` inclusive
    const target = index + 1;
    let cur = cursor;
    while (cur > target) {
      await c.undo();
      cur -= 1;
    }
    while (cur < target) {
      await c.redo();
      cur += 1;
    }
  };

  return (
    <div
      className="ed-history"
      ref={ref}
      role="dialog"
      aria-label={t("history.title")}
      data-testid="history-panel"
    >
      <div className="ed-panel-head">{t("history.title")}</div>
      {entries.length === 0 ? (
        <EmptyState title={t("history.empty")} />
      ) : (
        <ol className="ed-history-list" data-testid="history-list">
          {[...entries].reverse().map((e, i) => {
            const index = entries.length - 1 - i;
            const current = index + 1 === cursor;
            return (
              <li key={e.id} data-applied={e.applied} data-current={current}>
                <Button
                  variant="ghost"
                  aria-current={current ? "step" : undefined}
                  data-testid={`history-entry-${String(index)}`}
                  onClick={() => {
                    void jumpTo(index);
                  }}
                >
                  <span className={e.applied ? undefined : "ed-undone"}>{e.label}</span>
                  <small>
                    {e.actor.kind}
                    {e.applied ? "" : ` · ${t("history.undone")}`}
                    {current ? ` · ${t("history.current")}` : ""}
                  </small>
                </Button>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
}
