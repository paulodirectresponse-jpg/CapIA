import { useState } from "react";
import { Button, TextInput } from "@capia/ui-kit";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { Onboarding } from "./Onboarding";

export function Welcome() {
  const c = useController();
  const t = useT();
  const opening = useUi((s) => s.phase === "opening");
  const media = useUi((s) => s.engine?.mediaAvailable);
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
