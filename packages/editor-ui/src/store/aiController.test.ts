import { describe, expect, it, vi } from "vitest";
import type { AiClient, AiStatus, AiTaskEvent } from "@capia/engine-bindings";
import { AiController } from "./aiController";

const KEY = "sk-CANARY-ui-0123456789abcdef";

const status = (over: Partial<AiStatus> = {}): AiStatus => ({
  enabled: true,
  any_usable_model: true,
  secret_backend: "windows-credential-manager",
  providers: [],
  models: [],
  profiles: [],
  active_profile: null,
  presets: [],
  ...over,
});

function fake(over: Partial<Record<keyof AiClient, unknown>> = {}) {
  const calls: Record<string, unknown[][]> = {};
  const rec = (name: string, ret: unknown) =>
    vi.fn((...args: unknown[]) => {
      (calls[name] ??= []).push(args);
      return Promise.resolve(ret);
    });
  const client = {
    status: rec("status", status()),
    saveProvider: rec("saveProvider", {
      provider: { id: "p", credential_configured: true },
    }),
    assistantSend: rec("assistantSend", { task_id: "t1", conversation_id: "cv1" }),
    assistantApprove: rec("assistantApprove", { task: { final_text: "Aplicado." } }),
    assistantReject: rec("assistantReject", { task: {} }),
    planCaptions: rec("planCaptions", { task_id: "cap1" }),
    planSilence: rec("planSilence", { task_id: "sil1" }),
    applyPlan: rec("applyPlan", { revision: 9 }),
    analyzeReference: rec("analyzeReference", { task_id: "ref1" }),
    interpretDemand: rec("interpretDemand", { task_id: "dem1" }),
    cancelTask: rec("cancelTask", { cancelled: true }),
    ...over,
  } as unknown as AiClient;
  return { client, calls };
}

const ev = (task_id: string, phase: string, data: Record<string, unknown> = {}): AiTaskEvent =>
  ({ kind: "ai_task", task_id, phase, data }) as AiTaskEvent;

describe("AiController", () => {
  it("a chave de API sai uma vez para o engine e nunca fica no estado da UI", async () => {
    const { client, calls } = fake();
    const c = new AiController(client);
    const ok = await c.saveProvider(
      { id: "p", kind: "open_ai_compatible", display_name: "P" },
      KEY,
    );
    expect(ok).toBe(true);
    expect(calls.saveProvider?.[0]?.[1]).toBe(KEY);
    // nada do estado (nem um dump completo) contém a chave
    expect(JSON.stringify(c.state)).not.toContain(KEY);
    // erro do engine também não a carrega para toasts/estado
    const bad = fake({
      saveProvider: vi.fn(() => Promise.reject(new Error("invalid provider"))),
    });
    const notify = vi.fn();
    const c2 = new AiController(bad.client, notify);
    await c2.saveProvider({ id: "p", kind: "anthropic", display_name: "P" }, KEY);
    expect(JSON.stringify(c2.state)).not.toContain(KEY);
    expect(JSON.stringify(notify.mock.calls)).not.toContain(KEY);
  });

  it("transmite o texto do assistente em streaming e finaliza a mensagem", async () => {
    const { client } = fake();
    const c = new AiController(client);
    await c.send("oi");
    expect(c.state.chatTask).toBe("t1");
    c.handleEvent(ev("t1", "text", { delta: "Olá, " }));
    c.handleEvent(ev("t1", "text", { delta: "tudo bem?" }));
    c.handleEvent(ev("t1", "tool", { name: "timeline.get_state", state: "started" }));
    c.handleEvent(ev("t1", "tool", { name: "timeline.get_state", state: "finished", ok: true }));
    c.handleEvent(
      ev("t1", "done", { result: { status: "completed", final_text: "Olá, tudo bem?" } }),
    );
    const msgs = c.state.messages;
    expect(msgs.map((m) => m.role)).toEqual(["user", "assistant", "tool", "tool"]);
    expect(msgs[1]?.text).toBe("Olá, tudo bem?");
    expect(msgs[1]?.streaming).toBe(false);
    expect(c.state.chatTask).toBeNull();
  });

  it("reset (nova tentativa/fallback) descarta o texto parcial", async () => {
    const { client } = fake();
    const c = new AiController(client);
    await c.send("oi");
    c.handleEvent(ev("t1", "text", { delta: "parcial" }));
    c.handleEvent(ev("t1", "reset", { reason: "retry" }));
    c.handleEvent(ev("t1", "text", { delta: "final" }));
    expect(c.state.messages.at(-1)?.text).toBe("final");
  });

  it("aprovação: o plano fica pendente até o usuário aprovar ou recusar", async () => {
    const { client, calls } = fake();
    const c = new AiController(client);
    await c.send("renomeie");
    c.handleEvent(
      ev("t1", "approval", {
        plan: { plan_token: "plan_1.abc", label: "Renomear", operations: 1 },
      }),
    );
    c.handleEvent(ev("t1", "done", { result: { status: "awaiting_approval" } }));
    expect(c.state.pending?.plan.plan_token).toBe("plan_1.abc");
    expect(c.state.chatTask).toBe("t1");
    await c.approve();
    expect(calls.assistantApprove?.[0]).toEqual(["t1", "plan_1.abc"]);
    expect(c.state.pending).toBeNull();
    expect(c.state.messages.at(-1)?.text).toBe("Aplicado.");

    await c.send("de novo");
    c.handleEvent(
      ev("t1", "approval", { plan: { plan_token: "plan_2.def", label: "X", operations: 2 } }),
    );
    await c.reject();
    expect(calls.assistantReject).toHaveLength(1);
    expect(c.state.pending).toBeNull();
  });

  it("legendas/silêncio viram uma oferta que só se aplica quando o usuário pede", async () => {
    const { client, calls } = fake();
    const c = new AiController(client);
    await c.planCaptions("S", "asset1");
    expect(c.state.jobs.cap1?.state).toBe("running");
    c.handleEvent(ev("cap1", "progress", { stage: "transcribe", chunk: 1, chunks: 3 }));
    expect(c.state.jobs.cap1).toMatchObject({ done: 1, total: 3 });
    c.handleEvent(ev("cap1", "done", { result: { cue_count: 12, plan_token: "plan_7.tok" } }));
    expect(c.state.offer).toMatchObject({ token: "plan_7.tok", label: "captions" });
    expect(calls.applyPlan).toBeUndefined();
    expect(await c.applyOffer()).toBe(true);
    expect(calls.applyPlan?.[0]).toEqual(["plan_7.tok"]);
    expect(c.state.offer).toBeNull();
    // descartar não chama o engine
    await c.planSilence("S", "asset1");
    c.handleEvent(ev("sil1", "done", { result: { cut_count: 3, plan_token: "plan_8.tok" } }));
    c.discardOffer();
    expect(c.state.offer).toBeNull();
    expect(calls.applyPlan).toHaveLength(1);
  });

  it("tarefa que falha vira erro estruturado e notifica; cancelada não é erro", async () => {
    const { client } = fake();
    const notify = vi.fn();
    const c = new AiController(client, notify);
    await c.analyzeReference("a");
    c.handleEvent(ev("ref1", "error", { code: "NO_VIDEO", message: "sem vídeo" }));
    expect(c.state.jobs.ref1).toMatchObject({ state: "failed", error: { code: "NO_VIDEO" } });
    expect(notify).toHaveBeenCalledWith("error", "NO_VIDEO", "sem vídeo");
    await c.interpretDemand(["/tmp/b.docx"], []);
    c.handleEvent(ev("dem1", "cancelled"));
    expect(c.state.jobs.dem1?.state).toBe("cancelled");
    expect(notify).toHaveBeenCalledTimes(1);
  });

  it("referência e DemandSpec concluídos entram no estado", async () => {
    const { client } = fake();
    const c = new AiController(client);
    await c.analyzeReference("a1");
    c.handleEvent(
      ev("ref1", "done", {
        result: { grammar: { provenance: { asset_id: "a1" }, cut_rhythm: { shot_count: 4 } } },
      }),
    );
    expect(c.state.references.a1).toBeDefined();
    await c.interpretDemand([], ["v1"]);
    c.handleEvent(ev("dem1", "done", { result: { spec: { id: "ds_1", version: 1 } } }));
    expect(c.state.demand?.id).toBe("ds_1");
  });

  it("depois de descartado, eventos tardios são ignorados", async () => {
    const { client } = fake();
    const c = new AiController(client);
    await c.send("oi");
    c.dispose();
    c.handleEvent(ev("t1", "text", { delta: "tarde" }));
    expect(c.state.messages.map((m) => m.role)).toEqual(["user"]);
  });
});
