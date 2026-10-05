import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  AiStatus,
  EditorClient,
  MemoryItemView,
  RunSnapshot,
  RunSummary,
} from "@capia/engine-bindings";
import { ControllerProvider } from "../context";
import { EditorController } from "../store/controller";
import { AiRunsPanel } from "./AiRunsPanel";

const usage = {
  cost_micros: 1_500_000,
  unknown_cost_calls: 1,
  tokens: 10,
  provider_calls: 3,
  generations: 0,
  review_loops: 1,
  replans: 0,
  wall_time_ms: 5,
};

const run = (over: Partial<RunSummary> = {}): RunSummary => ({
  id: "run-1",
  status: "waiting_user",
  stage: "validate_plan",
  revision: 3,
  created_ms: 1,
  updated_ms: 2,
  completed_ms: null,
  usage,
  budget: {
    max_cost_micros: null,
    max_tokens: null,
    max_provider_calls: null,
    max_generations: null,
    max_review_loops: 2,
    max_replans: 3,
    max_wall_time_ms: null,
  },
  pending: {
    id: "dec-1",
    kind: "plan_approval",
    question: "Aprovar o plano de edição?",
    options: [
      { id: "approve", label: "Aprovar" },
      { id: "reject", label: "Rejeitar" },
    ],
    consequences: "O plano será aplicado à timeline.",
    resume_stage: "validate_plan",
  },
  error: null,
  parent_run_id: null,
  variant_group_id: null,
  deliverables: ["main"],
  sequences: [],
  resume_class: "needs_user",
  ...over,
});

const snap = (r: RunSummary): RunSnapshot => ({
  run: r,
  usage,
  stages: [],
  provenance: [],
  last_event_seq: 4,
  resume_class: r.resume_class,
});

const mem = (over: Partial<MemoryItemView> = {}): MemoryItemView => ({
  id: "mem_1",
  scope: "user",
  client_id: null,
  kind: "preference",
  content: "CTAs curtos",
  source: "agent",
  status: "proposed",
  confidence: 0.7,
  evidence: [],
  origin_run: "run-1",
  ...over,
});

function setup(initial: RunSummary[], items: MemoryItemView[] = []) {
  let runs = initial;
  const ai = {
    status: vi.fn(() => Promise.resolve({ enabled: true, any_usable_model: true } as AiStatus)),
    runList: vi.fn(() => Promise.resolve({ runs })),
    runGet: vi.fn((id: string) => {
      const r = runs.find((x) => x.id === id) ?? run();
      return Promise.resolve(snap(r));
    }),
    runCreate: vi.fn(() => {
      runs = [run({ id: "run-new", status: "running", pending: null, stage: "plan" })];
      return Promise.resolve({ run: runs[0] });
    }),
    runDecide: vi.fn(() => {
      runs = runs.map((r) => ({ ...r, status: "completed" as const, pending: null }));
      return Promise.resolve({ run: runs[0] });
    }),
    runPause: vi.fn(() => Promise.resolve({ run: runs[0] })),
    runResume: vi.fn(() => Promise.resolve({ run: runs[0] })),
    runCancel: vi.fn(() => Promise.resolve({ run: runs[0] })),
    memoryList: vi.fn(() => Promise.resolve({ items })),
    memoryApprove: vi.fn(() => Promise.resolve({ item: mem({ status: "active" }) })),
    memoryReject: vi.fn(() => Promise.resolve({ item: mem({ status: "rejected" }) })),
    gatewayStatus: vi.fn(() =>
      Promise.resolve({ adapters: [], generation: { enabled: false, available: false } }),
    ),
    generationSetEnabled: vi.fn(() =>
      Promise.resolve({ adapters: [], generation: { enabled: true, available: false } }),
    ),
    undoReport: vi.fn(() => Promise.resolve({ entries: [3, 4], conflicts: [] })),
  };
  const editor = {
    ai,
    undoReport: ai.undoReport,
    undoSelective: vi.fn(() => Promise.resolve({ revision: 9, changes: [] })),
  };
  const controller = new EditorController(editor as unknown as EditorClient, {
    storage: null,
    pollMs: 100000,
  });
  return { ai, editor, controller };
}

function ui(controller: EditorController) {
  return render(
    <ControllerProvider controller={controller}>
      <AiRunsPanel />
    </ControllerProvider>,
  );
}

describe("painel de execuções de IA", () => {
  afterEach(cleanup);

  it("sem material bruto não deixa iniciar e explica por quê", async () => {
    const { controller } = setup([]);
    ui(controller);
    expect(await screen.findByTestId("ai-run-empty")).toBeTruthy();
    expect(screen.getByTestId("ai-run-assets").textContent).toMatch(/Import|Importe/);
    expect(screen.getByTestId<HTMLButtonElement>("ai-run-start").disabled).toBe(true);
  });

  it("mostra a decisão pendente, a aprovação vai amarrada ao id da decisão e o custo é honesto", async () => {
    const { ai, controller } = setup([run()]);
    ui(controller);
    fireEvent.click(await screen.findByTestId("ai-run-run-1"));
    expect(await screen.findByTestId("ai-run-decision")).toBeTruthy();
    expect(screen.getByTestId("ai-run-cost").textContent).toMatch(/1\.50/);
    expect(screen.getByTestId("ai-run-cost").textContent).toMatch(/1/); // 1 chamada sem preço
    fireEvent.click(screen.getByTestId("ai-run-option-approve"));
    await waitFor(() => {
      expect(ai.runDecide).toHaveBeenCalledWith("run-1", "dec-1", "approve", undefined);
    });
  });

  it("propostas de memória da IA ficam inativas até o clique explícito do usuário", async () => {
    const { ai, controller } = setup([], [mem()]);
    ui(controller);
    fireEvent.click(await screen.findByRole("tab", { name: /Memória|Memory/ }));
    expect(await screen.findByTestId("ai-memory-proposed")).toBeTruthy();
    expect(ai.memoryApprove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("ai-memory-approve-mem_1"));
    await waitFor(() => {
      expect(ai.memoryApprove).toHaveBeenCalledWith("mem_1");
    });
  });

  it("a geração por IA começa desligada e só liga por ação explícita", async () => {
    const { ai, controller } = setup([]);
    ui(controller);
    fireEvent.click(await screen.findByRole("tab", { name: /Fontes|Sources/ }));
    expect(await screen.findByTestId("ai-generation-toggle")).toBeTruthy();
    expect(ai.generationSetEnabled).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("ai-generation-toggle"));
    await waitFor(() => {
      expect(ai.generationSetEnabled).toHaveBeenCalledWith(true);
    });
  });

  it("desfazer a execução é undo seletivo (nova entrada), pelo controlador do editor", async () => {
    const done = run({
      status: "completed",
      pending: null,
      stage: "done",
      sequences: [{ deliverable: "main", sequence_id: "sq_1", role: "standalone" }],
    });
    const { editor, controller } = setup([done]);
    ui(controller);
    fireEvent.click(await screen.findByTestId("ai-run-run-1"));
    fireEvent.click(await screen.findByTestId("ai-run-undo"));
    await waitFor(() => {
      expect(editor.undoSelective).toHaveBeenCalledWith("run:run-1", "safe", expect.any(String));
    });
  });
});
