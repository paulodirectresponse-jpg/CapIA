import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiStatus, EditorClient } from "@capia/engine-bindings";
import { ControllerProvider } from "../context";
import { EditorController } from "../store/controller";
import { AiPanel } from "./AiPanel";
import { AiSettingsDialog } from "./AiSettingsDialog";

const KEY = "sk-CANARY-component-0123456789";

const status = (over: Partial<AiStatus> = {}): AiStatus => ({
  enabled: true,
  any_usable_model: true,
  secret_backend: "windows-credential-manager",
  providers: [],
  models: [],
  profiles: [],
  active_profile: null,
  presets: [
    {
      key: "openai",
      display_name: "OpenAI",
      kind: "open_ai_compatible",
      base_url: "https://api.openai.com/v1",
    },
  ],
  ...over,
});

function setup(st: AiStatus, extra: Record<string, unknown> = {}) {
  const ai = {
    status: vi.fn(() => Promise.resolve(st)),
    usage: vi.fn(() =>
      Promise.resolve({
        calls: 0,
        failed_calls: 0,
        cache_hits: 0,
        input_tokens: 0,
        output_tokens: 0,
        known_cost_micros: 0,
        currency: null,
        unknown_cost_calls: 0,
      }),
    ),
    getDemand: vi.fn(() => Promise.resolve({ specs: [] })),
    saveProvider: vi.fn((...args: [unknown, string?]) =>
      Promise.resolve({ provider: { id: "openai" }, argc: args.length }),
    ),
    assistantSend: vi.fn(() => Promise.resolve({ task_id: "t1", conversation_id: "c1" })),
    assistantApprove: vi.fn(() => Promise.resolve({ task: { final_text: "Aplicado." } })),
    assistantReject: vi.fn(() => Promise.resolve({ task: {} })),
    ...extra,
  };
  const client = { ai } as unknown as EditorClient;
  const controller = new EditorController(client, { storage: null, pollMs: 100000 });
  return { ai, controller };
}

function ui(controller: EditorController, node: React.ReactNode) {
  return render(<ControllerProvider controller={controller}>{node}</ControllerProvider>);
}

describe("painel de IA", () => {
  afterEach(cleanup);

  it("com a IA desligada mostra o aviso e não deixa enviar mensagem", async () => {
    const { controller } = setup(status({ enabled: false, any_usable_model: false }));
    ui(controller, <AiPanel onOpenSettings={() => undefined} />);
    expect(await screen.findByTestId("ai-off-banner")).toBeTruthy();
    expect(screen.getByTestId("ai-not-configured")).toBeTruthy();
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "oi" } });
    expect(screen.getByTestId<HTMLButtonElement>("ai-send").disabled).toBe(true);
  });

  it("envia, mostra o streaming e pede aprovação antes de aplicar", async () => {
    const { ai, controller } = setup(status());
    ui(controller, <AiPanel onOpenSettings={() => undefined} />);
    await waitFor(() => {
      expect(controller.ai.state.status?.any_usable_model).toBe(true);
    });
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "renomeie o clip" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    await waitFor(() => {
      expect(ai.assistantSend).toHaveBeenCalledWith(
        "renomeie o clip",
        "ask",
        undefined,
        expect.objectContaining({ selected_clips: expect.any(Array) as string[] }),
      );
    });
    controller.ai.handleEvent({
      kind: "ai_task",
      task_id: "t1",
      phase: "text",
      data: { delta: "Posso renomear." },
    });
    expect(await screen.findByText("Posso renomear.")).toBeTruthy();
    controller.ai.handleEvent({
      kind: "ai_task",
      task_id: "t1",
      phase: "approval",
      data: { plan: { plan_token: "plan_1.abc", label: "Renomear", operations: 1 } },
    });
    expect(await screen.findByTestId("ai-approval")).toBeTruthy();
    fireEvent.click(screen.getByTestId("ai-approve"));
    await waitFor(() => {
      expect(ai.assistantApprove).toHaveBeenCalledWith("t1", "plan_1.abc");
    });
  });
});

describe("configuração de IA", () => {
  afterEach(cleanup);

  it("o campo da chave é só de escrita: password, enviado uma vez e esvaziado", async () => {
    const { ai, controller } = setup(status());
    ui(controller, <AiSettingsDialog open onClose={() => undefined} />);
    const key = await screen.findByTestId<HTMLInputElement>("ai-prov-key");
    expect(key.type).toBe("password");
    expect(key.autocomplete).toBe("off");
    fireEvent.change(screen.getByTestId("ai-prov-id"), { target: { value: "openai" } });
    fireEvent.change(screen.getByTestId("ai-prov-name"), { target: { value: "OpenAI" } });
    fireEvent.change(key, { target: { value: KEY } });
    fireEvent.click(screen.getByTestId("ai-prov-save"));
    await waitFor(() => {
      expect(ai.saveProvider).toHaveBeenCalledTimes(1);
    });
    const [input, sent] = ai.saveProvider.mock.calls[0] as unknown as [
      Record<string, unknown>,
      string,
    ];
    expect(sent).toBe(KEY);
    expect(JSON.stringify(input)).not.toContain(KEY);
    // o campo foi esvaziado e a chave não está em lugar nenhum do DOM nem do estado
    await waitFor(() => {
      expect(screen.getByTestId<HTMLInputElement>("ai-prov-key").value).toBe("");
    });
    expect(document.body.innerHTML).not.toContain(KEY);
    expect(JSON.stringify(controller.ai.state)).not.toContain(KEY);
  });

  it("mostra só se há credencial salva (nunca a chave) e não oferece copiar", async () => {
    const st = status({
      providers: [
        {
          id: "openai",
          kind: "open_ai_compatible",
          display_name: "OpenAI",
          base_url: "https://api.openai.com/v1",
          enabled: true,
          local: false,
          needs_credential: true,
          credential_configured: true,
          bound_host: "api.openai.com",
          extra_headers: {},
          timeout_s: 120,
          max_concurrency: 4,
          allow_loopback: false,
        },
      ],
    });
    const { controller } = setup(st);
    ui(controller, <AiSettingsDialog open onClose={() => undefined} />);
    const badge = await screen.findByTestId("ai-cred-openai");
    expect(badge.textContent).toBe("Key saved");
    expect(screen.queryByText(/copy/i)).toBeNull();
    expect(document.body.innerHTML).not.toMatch(/sk-/);
  });
});
