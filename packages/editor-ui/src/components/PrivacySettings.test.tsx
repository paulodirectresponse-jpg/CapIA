import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  EditorClient,
  SupportClient,
  SupportStatus,
  UpdateCheck,
} from "@capia/engine-bindings";
import { ControllerProvider } from "../context";
import { EditorController } from "../store/controller";
import { Onboarding } from "./Onboarding";
import { PrivacySettings } from "./PrivacySettings";

const status = (over: Partial<SupportStatus> = {}): SupportStatus => ({
  version: "0.6.0-rc.1",
  build: {
    display: "0.6.0-rc.1 (windows/x86_64)",
    os: "windows",
    arch: "x86_64",
    dev_build: true,
    build_id: null,
  },
  crash: {
    opt_in: false,
    decided: false,
    status: { opt_in: false, sink_configured: false, local_records: 0, pending_upload: 0 },
  },
  update_channel: "stable",
  onboarding_dismissed: false,
  ...over,
});

function setup(over: Partial<Record<keyof SupportClient, unknown>> = {}) {
  const support = {
    status: vi.fn(() => Promise.resolve(status())),
    setCrashReporting: vi.fn((enabled: boolean) =>
      Promise.resolve(
        status({
          crash: {
            opt_in: enabled,
            decided: true,
            status: {
              opt_in: enabled,
              sink_configured: false,
              local_records: 0,
              pending_upload: 0,
            },
          },
        }),
      ),
    ),
    setChannel: vi.fn((c: "stable" | "beta") => Promise.resolve(status({ update_channel: c }))),
    dismissOnboarding: vi.fn(() => Promise.resolve(status({ onboarding_dismissed: true }))),
    diagnosticPreview: vi.fn(() =>
      Promise.resolve({
        entries: [{ path: "app.json", bytes: 120, description: "App version" }],
        total_bytes: 120,
        never_included: ["API keys, tokens and any credential"],
      }),
    ),
    createDiagnostic: vi.fn(() => Promise.resolve({ path: "~/diag.zip", bytes: 10, entries: [] })),
    checkUpdates: vi.fn(() =>
      Promise.resolve({
        state: "not_configured",
        reason: "no_trusted_keys",
        channel: "stable",
        current: "0.6.0-rc.1",
      } satisfies UpdateCheck),
    ),
    ...over,
  };
  const editor = { support, ai: {} };
  const controller = new EditorController(editor as unknown as EditorClient, {
    storage: null,
    pollMs: 100000,
  });
  return { support, controller };
}

const ui = (controller: EditorController, el: React.ReactElement) =>
  render(<ControllerProvider controller={controller}>{el}</ControllerProvider>);

describe("Privacidade e atualizações", () => {
  afterEach(cleanup);

  it("crash report começa desligado e só liga por clique explícito; pode desligar", async () => {
    const { support, controller } = setup();
    ui(controller, <PrivacySettings />);
    const box = await screen.findByTestId<HTMLInputElement>("privacy-crash");
    expect(box.checked).toBe(false);
    expect(support.setCrashReporting).not.toHaveBeenCalled();
    fireEvent.click(box);
    await waitFor(() => {
      expect(support.setCrashReporting).toHaveBeenCalledWith(true);
    });
    await waitFor(() => {
      expect(screen.getByTestId<HTMLInputElement>("privacy-crash").checked).toBe(true);
    });
    expect(screen.getByTestId("privacy-crash-local")).toBeTruthy();
    fireEvent.click(screen.getByTestId("privacy-crash"));
    await waitFor(() => {
      expect(support.setCrashReporting).toHaveBeenLastCalledWith(false);
    });
  });

  it("diagnóstico: mostra o que entra e o que nunca entra ANTES de salvar", async () => {
    const { support, controller } = setup();
    ui(controller, <PrivacySettings />);
    fireEvent.click(await screen.findByTestId("privacy-diag-preview"));
    expect(await screen.findByTestId("privacy-diag-list")).toBeTruthy();
    expect(screen.getByTestId("privacy-diag-never").textContent).toMatch(/API keys/);
    expect(support.createDiagnostic).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("privacy-diag-create"));
    expect(await screen.findByTestId("privacy-diag-done")).toBeTruthy();
    expect(support.createDiagnostic).toHaveBeenCalledTimes(1);
  });

  it("atualizações: sem configuração mostra 'não configurado' e o canal é escolhível", async () => {
    const { support, controller } = setup();
    ui(controller, <PrivacySettings />);
    fireEvent.click(await screen.findByTestId("privacy-update-check"));
    const r = await screen.findByTestId("privacy-update-result");
    expect(r.textContent).toMatch(/not configured|não estão configuradas/);
    fireEvent.change(screen.getByTestId("privacy-channel"), { target: { value: "beta" } });
    await waitFor(() => {
      expect(support.setChannel).toHaveBeenCalledWith("beta");
    });
  });

  it("resultado assinado disponível aparece com a versão", async () => {
    const { controller } = setup({
      checkUpdates: vi.fn(() =>
        Promise.resolve({
          state: "available",
          version: "0.7.0",
          notes: "",
          signature: "verified",
          channel: "stable",
          current: "0.6.0-rc.1",
        }),
      ),
    });
    ui(controller, <PrivacySettings />);
    fireEvent.click(await screen.findByTestId("privacy-update-check"));
    expect((await screen.findByTestId("privacy-update-result")).textContent).toMatch(/0\.7\.0/);
  });

  it("sem o serviço (navegador) a seção só avisa e o editor segue", async () => {
    const { controller } = setup({
      status: vi.fn(() => Promise.reject(new Error("UNKNOWN_METHOD"))),
    });
    ui(controller, <PrivacySettings />);
    expect(await screen.findByTestId("privacy-unavailable")).toBeTruthy();
    expect(screen.queryByTestId("privacy-crash")).toBeNull();
  });
});

describe("Onboarding", () => {
  afterEach(cleanup);

  it("aparece, é dispensável, não bloqueia e lembra a escolha", async () => {
    const { support, controller } = setup();
    ui(controller, <Onboarding />);
    const card = await screen.findByTestId("onboarding");
    expect(card.getAttribute("role")).not.toBe("dialog");
    fireEvent.click(screen.getByTestId("onboarding-dismiss"));
    await waitFor(() => {
      expect(screen.queryByTestId("onboarding")).toBeNull();
    });
    expect(support.dismissOnboarding).toHaveBeenCalledTimes(1);
  });

  it("não aparece se já foi dispensado nem sem o serviço", async () => {
    const a = setup({
      status: vi.fn(() => Promise.resolve(status({ onboarding_dismissed: true }))),
    });
    ui(a.controller, <Onboarding />);
    await waitFor(() => {
      expect(a.support.status).toHaveBeenCalled();
    });
    expect(screen.queryByTestId("onboarding")).toBeNull();
    cleanup();
    const b = setup({ status: vi.fn(() => Promise.reject(new Error("x"))) });
    ui(b.controller, <Onboarding />);
    await waitFor(() => {
      expect(b.support.status).toHaveBeenCalled();
    });
    expect(screen.queryByTestId("onboarding")).toBeNull();
  });
});
