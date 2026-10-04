import { useEffect, useMemo, useState } from "react";
import { Button, Dialog, Select } from "@capia/ui-kit";
import { useController, useUi } from "../context";
import { LANGUAGES, useT } from "../i18n";
import {
  ACTION_IDS,
  DEFAULT_BINDINGS,
  eventToBinding,
  findConflicts,
  formatBinding,
  rebind,
  resetBindings,
  resolveBindings,
  type ActionId,
} from "../lib/keymap";
import type { Language } from "../lib/prefs";

export function SettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const c = useController();
  const t = useT();
  const { language, keymap, recovered } = useUi((s) => ({
    language: s.prefs.language,
    keymap: s.prefs.keymap,
    recovered: s.prefsRecovered,
  }));
  const [recording, setRecording] = useState<ActionId | null>(null);
  const bindings = useMemo(() => resolveBindings(keymap), [keymap]);
  const conflicts = useMemo(() => findConflicts(bindings), [bindings]);

  // grava a próxima tecla (Escape cancela); captura antes dos atalhos globais
  useEffect(() => {
    if (!recording) return;
    const on = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(null);
        return;
      }
      const b = eventToBinding(e);
      if (!b) return;
      const next = rebind(keymap, recording, [b]);
      c.setPrefs({ keymap: next.custom });
      setRecording(null);
    };
    window.addEventListener("keydown", on, true);
    return () => {
      window.removeEventListener("keydown", on, true);
    };
  }, [recording, keymap, c]);

  return (
    <Dialog
      title={t("settings.title")}
      open={open}
      onClose={onClose}
      footer={<Button onClick={onClose}>{t("common.close")}</Button>}
    >
      <div className="ed-settings" data-testid="settings-dialog">
        {recovered && (
          <p role="status" className="ed-warn">
            {t("settings.prefsRecovered")}
          </p>
        )}
        <Select
          label={t("settings.language")}
          value={language}
          data-testid="settings-language"
          onChange={(e) => {
            c.setLanguage(e.currentTarget.value as Language);
          }}
        >
          {LANGUAGES.map((l) => (
            <option key={l.id} value={l.id}>
              {l.label}
            </option>
          ))}
        </Select>
        <div className="ed-row">
          <Button
            data-testid="copy-diagnostics"
            onClick={() => {
              const text = JSON.stringify(c.diagnostics(), null, 2);
              void navigator.clipboard
                .writeText(text)
                .then(() => {
                  c.toast("success", t("settings.diagnosticsCopied"));
                })
                .catch(() => {
                  c.toast("error", t("settings.diagnosticsFailed"), text.slice(0, 200));
                });
            }}
          >
            {t("settings.copyDiagnostics")}
          </Button>
          <span className="ed-hint">{t("settings.diagnosticsHint")}</span>
        </div>
        <div className="ed-row">
          <h3 className="ed-subhead">{t("settings.keymap")}</h3>
          <Button
            data-testid="keymap-reset-all"
            onClick={() => {
              c.setPrefs({ keymap: resetBindings(keymap) });
            }}
          >
            {t("settings.resetKeymap")}
          </Button>
        </div>
        {conflicts.length > 0 && (
          <p role="alert" className="ed-warn" data-testid="keymap-conflicts">
            {conflicts.map((cf) => (
              <span key={cf.binding}>
                {t("settings.conflict", {
                  key: formatBinding(cf.binding),
                  actions: cf.actions.map((a) => t(`action.${a}`)).join(", "),
                })}{" "}
              </span>
            ))}
          </p>
        )}
        <ul className="ed-keymap" data-testid="keymap-list">
          {ACTION_IDS.map((id) => {
            const custom = id in keymap;
            return (
              <li key={id}>
                <span>{t(`action.${id}`)}</span>
                <Button
                  data-testid={`keymap-${id}`}
                  aria-pressed={recording === id}
                  onClick={() => {
                    setRecording(recording === id ? null : id);
                  }}
                >
                  {recording === id
                    ? t("settings.pressKeys")
                    : bindings[id].map(formatBinding).join(" / ") || "—"}
                </Button>
                {custom && (
                  <>
                    <Button
                      variant="ghost"
                      title={DEFAULT_BINDINGS[id].map(formatBinding).join(" / ")}
                      onClick={() => {
                        c.setPrefs({ keymap: resetBindings(keymap, id) });
                      }}
                    >
                      {t("settings.resetOne")}
                    </Button>
                  </>
                )}
              </li>
            );
          })}
        </ul>
      </div>
    </Dialog>
  );
}
