import { Component, type ErrorInfo, type ReactNode } from "react";
import { Button } from "@capia/ui-kit";
import { useT } from "../i18n";

interface Props {
  children: ReactNode;
  /** Chamado ao capturar (ex.: log local redigido). */
  onError?: (error: Error, info: ErrorInfo) => void;
  /** Limites aninhados mostram só um cartão; o raiz ocupa a tela. */
  scope?: "app" | "panel";
}

interface State {
  error: Error | null;
}

function Fallback({
  error,
  scope,
  onRetry,
}: {
  error: Error;
  scope: "app" | "panel";
  onRetry: () => void;
}) {
  const t = useT();
  return (
    <div
      className={scope === "app" ? "ed-crash ed-crash--app" : "ed-crash"}
      role="alert"
      data-testid="error-boundary"
    >
      <h2>{t("crash.title")}</h2>
      <p>{t("crash.body")}</p>
      <p className="ed-hint">{t("crash.saved")}</p>
      <div className="ed-row">
        <Button variant="primary" onClick={onRetry} data-testid="error-boundary-retry">
          {t("crash.retry")}
        </Button>
        {scope === "app" && (
          <Button
            onClick={() => {
              window.location.reload();
            }}
          >
            {t("crash.reload")}
          </Button>
        )}
      </div>
      <details>
        <summary>{t("common.details")}</summary>
        <pre>{`${error.name}: ${error.message}`}</pre>
      </details>
    </div>
  );
}

/**
 * Nenhuma falha de renderização deixa tela preta ou branca: o usuário vê o que houve em português,
 * sabe que o projeto está salvo (toda edição é transacional) e pode tentar de novo.
 */
export class ErrorBoundary extends Component<Props, State> {
  override state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    this.props.onError?.(error, info);
  }

  override render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <Fallback
        error={error}
        scope={this.props.scope ?? "panel"}
        onRetry={() => {
          this.setState({ error: null });
        }}
      />
    );
  }
}
