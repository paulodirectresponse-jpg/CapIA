import { useEffect } from "react";
import { Button } from "@capia/ui-kit";
import { useController, useSupport } from "../context";
import { useT } from "../i18n";

/**
 * Boas-vindas mínimas (Fase 6): um cartão dispensável, SEM bloquear nada e sem tour forçado. Fica na tela
 * inicial; "Dispensar" é lembrado pelo app. Sem o serviço do desktop não aparece.
 */
export function Onboarding() {
  const c = useController();
  const t = useT();
  const show = useSupport(
    (s) => s.available && s.status !== null && !s.status.onboarding_dismissed,
  );

  useEffect(() => {
    void c.support.refresh();
  }, [c]);

  if (!show) return null;
  return (
    <aside className="ed-onboarding" data-testid="onboarding" aria-label={t("onboarding.title")}>
      <strong>{t("onboarding.title")}</strong>
      <ul>
        <li>{t("onboarding.tip1")}</li>
        <li>{t("onboarding.tip2")}</li>
        <li>{t("onboarding.tip3")}</li>
      </ul>
      <Button
        variant="ghost"
        data-testid="onboarding-dismiss"
        onClick={() => {
          void c.support.dismissOnboarding();
        }}
      >
        {t("onboarding.dismiss")}
      </Button>
    </aside>
  );
}
