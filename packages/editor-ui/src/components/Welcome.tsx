import { useState } from "react";
import { Button, TextInput } from "@capia/ui-kit";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { Onboarding } from "./Onboarding";

/** `C:\\Videos\\anuncio.capia` → `anuncio`. */
export function projectName(path: string): string {
  const base = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return base.replace(/\.capia$/i, "");
}

function folderOf(path: string): string {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join("/") || "/";
}

export function Welcome() {
  const c = useController();
  const t = useT();
  const opening = useUi((s) => s.phase === "opening");
  const media = useUi((s) => s.engine?.mediaAvailable);
  const recent = useUi((s) => s.prefs.recent);
  const [path, setPath] = useState("");
  const submit = (mode: "create" | "open") => {
    const p = path.trim();
    if (!p) return;
    void (mode === "create" ? c.createProject(p) : c.openProject(p));
  };
  const browse = async (mode: "create" | "open") => {
    const picked =
      mode === "open"
        ? await c.platform.pickFiles({
            title: t("welcome.openProject"),
            filters: [{ name: "CapIA", extensions: ["capia"] }],
          })
        : await c.platform.pickSavePath({
            title: t("welcome.newProject"),
            defaultName: "project.capia",
            filters: [{ name: "CapIA", extensions: ["capia"] }],
          });
    const first = picked ? (Array.isArray(picked) ? picked[0] : picked) : null;
    if (first) setPath(first);
  };
  return (
    <div className="ed-welcome" data-testid="welcome">
      <form
        className="ed-welcome-card"
        onSubmit={(e) => {
          e.preventDefault();
          submit("create");
        }}
      >
        <div>
          <h1>{t("welcome.title")}</h1>
          <p style={{ color: "var(--text-muted)", margin: "4px 0 0" }}>{t("welcome.subtitle")}</p>
        </div>
        {recent.length > 0 && (
          <section aria-label={t("welcome.recent")} data-testid="recent-projects">
            <h2 className="ed-welcome-h2">{t("welcome.recent")}</h2>
            <ul className="ed-recent">
              {recent.map((r) => (
                <li key={r.path} className="ed-recent-card">
                  <button
                    type="button"
                    className="ed-recent-open"
                    disabled={opening}
                    data-testid="recent-open"
                    title={r.path}
                    onClick={() => {
                      void c.openProject(r.path);
                    }}
                  >
                    <strong>{projectName(r.path)}</strong>
                    <span className="ed-hint">{folderOf(r.path)}</span>
                  </button>
                  <button
                    type="button"
                    className="ed-recent-forget"
                    aria-label={t("welcome.recentForget")}
                    title={t("welcome.recentForget")}
                    onClick={() => {
                      c.forgetRecent(r.path);
                    }}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
        <TextInput
          label={t("welcome.path")}
          placeholder={t("welcome.pathHint")}
          value={path}
          data-testid="project-path"
          onChange={(e) => {
            setPath(e.currentTarget.value);
          }}
        />
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <Button
            variant="primary"
            type="submit"
            disabled={opening || path.trim() === ""}
            data-testid="project-create"
          >
            {t("welcome.create")}
          </Button>
          <Button
            disabled={opening || path.trim() === ""}
            data-testid="project-open"
            onClick={() => {
              submit("open");
            }}
          >
            {t("welcome.open")}
          </Button>
          {c.platform.native && (
            <>
              <Button
                onClick={() => {
                  void browse("create");
                }}
              >
                {t("welcome.newProject")}…
              </Button>
              <Button
                onClick={() => {
                  void browse("open");
                }}
              >
                {t("welcome.openProject")}…
              </Button>
            </>
          )}
        </div>
        {media === false && (
          <div role="alert" style={{ color: "var(--warning)" }}>
            {t("app.mediaMissing")}
          </div>
        )}
      </form>
      <Onboarding />
    </div>
  );
}
