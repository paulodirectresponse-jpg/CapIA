import { useEffect } from "react";
import { Button, Select } from "@capia/ui-kit";
import type { UpdateCheck } from "@capia/engine-bindings";
import { useController, useSupport } from "../context";
import { useT } from "../i18n";

function updateText(t: ReturnType<typeof useT>, u: UpdateCheck): string {
  switch (u.state) {
    case "not_configured":
      return t("privacy.update.notConfigured");
    case "up_to_date":
      return t("privacy.update.upToDate", { version: u.current });
    case "available":
      return t("privacy.update.available", { version: u.version });
    case "requires_intermediate":
      return t("privacy.update.intermediate", { version: u.min_version });
    case "rejected":
      return t("privacy.update.rejected", { reason: u.reason });
    case "invalid":
      return t("privacy.update.invalid", { reason: u.reason });
  }
}

/** Seção "Privacidade e atualizações" das configurações (Fase 6). Só aparece com o serviço do app desktop. */
export function PrivacySettings() {
  const c = useController();
  const t = useT();
  const s = useSupport((x) => x);

  useEffect(() => {
    void c.support.refresh();
  }, [c]);

  return (
    <section className="ed-privacy" data-testid="privacy-settings" aria-labelledby="privacy-title">
      <h3 className="ed-subhead" id="privacy-title">
        {t("privacy.title")}
      </h3>
      {!s.available || !s.status ? (
        <p className="ed-hint" data-testid="privacy-unavailable">
          {t("privacy.unavailable")}
        </p>
      ) : (
        <>
          <p className="ed-hint" data-testid="privacy-version">
            {t("privacy.version", { build: s.status.build.display })}
            {s.status.build.dev_build ? ` · ${t("privacy.devBuild")}` : ""}
          </p>

          <label className="ed-row">
            <input
              type="checkbox"
              data-testid="privacy-crash"
              checked={s.status.crash.opt_in}
              disabled={s.busy}
              onChange={(e) => {
                void c.support.setCrashReporting(e.currentTarget.checked);
              }}
            />
            <span>{t("privacy.crash.label")}</span>
          </label>
          <p className="ed-hint">{t("privacy.crash.hint")}</p>
          {s.status.crash.opt_in && !s.status.crash.status.sink_configured && (
            <p className="ed-hint" data-testid="privacy-crash-local">
              {t("privacy.crash.localOnly")}
            </p>
          )}

          <div className="ed-row">
            <Button
              data-testid="privacy-diag-preview"
              disabled={s.busy}
              onClick={() => {
                void c.support.loadPreview();
              }}
            >
              {t("privacy.diag.generate")}
            </Button>
            <span className="ed-hint">{t("privacy.diag.hint")}</span>
          </div>
          {s.preview && (
            <div data-testid="privacy-diag-list">
              <p>{t("privacy.diag.willInclude")}</p>
              <ul>
                {s.preview.entries.map((e) => (
                  <li key={e.path}>
                    <code>{e.path}</code> — {e.description} ({e.bytes} B)
                  </li>
                ))}
              </ul>
              <p>{t("privacy.diag.neverIncluded")}</p>
              <ul data-testid="privacy-diag-never">
                {s.preview.never_included.map((x) => (
                  <li key={x}>{x}</li>
                ))}
              </ul>
              <Button
                variant="primary"
                data-testid="privacy-diag-create"
                disabled={s.busy}
                onClick={() => {
                  void c.support.createDiagnostic();
                }}
              >
                {t("privacy.diag.save")}
              </Button>
            </div>
          )}
          {s.diagnostic && (
            <p role="status" data-testid="privacy-diag-done">
              {t("privacy.diag.saved", { path: s.diagnostic.path })}
            </p>
          )}

          <div className="ed-row">
            <Select
              label={t("privacy.channel")}
              value={s.status.update_channel}
              data-testid="privacy-channel"
              onChange={(e) => {
                void c.support.setChannel(e.currentTarget.value === "beta" ? "beta" : "stable");
              }}
            >
              <option value="stable">{t("privacy.channel.stable")}</option>
              <option value="beta">{t("privacy.channel.beta")}</option>
            </Select>
            <Button
              data-testid="privacy-update-check"
              disabled={s.busy}
              onClick={() => {
                void c.support.checkUpdates();
              }}
            >
              {t("privacy.update.check")}
            </Button>
          </div>
          {s.update && (
            <p role="status" data-testid="privacy-update-result">
              {updateText(t, s.update)}
            </p>
          )}
        </>
      )}
      {s.error && (
        <p role="alert" className="ed-warn" data-testid="privacy-error">
          {s.error}
        </p>
      )}
    </section>
  );
}
