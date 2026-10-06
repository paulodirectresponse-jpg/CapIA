import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { I18nProvider } from "../i18n";
import { ErrorBoundary } from "./ErrorBoundary";

let explode = true;
function Bomb() {
  if (explode) throw new Error("boom de teste");
  return <p data-testid="recovered" />;
}

describe("ErrorBoundary", () => {
  it("mostra mensagem humana em vez de tela vazia e permite tentar de novo", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const seen = vi.fn();
    render(
      <I18nProvider lang="pt-BR">
        <ErrorBoundary scope="app" onError={seen}>
          <Bomb />
        </ErrorBoundary>
      </I18nProvider>,
    );
    expect(screen.getByRole("alert").textContent).toContain("Algo deu errado");
    expect(screen.getByRole("alert").textContent).toContain("Seu projeto está salvo");
    expect(seen).toHaveBeenCalledOnce();
    explode = false;
    fireEvent.click(screen.getByTestId("error-boundary-retry"));
    expect(screen.getByTestId("recovered")).toBeTruthy();
    spy.mockRestore();
  });
});
