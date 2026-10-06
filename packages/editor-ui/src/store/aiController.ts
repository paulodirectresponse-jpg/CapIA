/**
 * Controlador da IA (Fase 4). Única peça da UI que fala com `ai.*`. Regras:
 *  - nenhum segredo fica em estado, `localStorage` ou toast: a chave entra como argumento de
 *    `saveProvider`, é enviada UMA vez e descartada (o engine só responde `credential_configured`);
 *  - a UI não monta comandos de IA: planos (legendas, silêncio, chat) voltam como `plan_token` e a
 *    aplicação passa pelo gate preview→apply_plan do engine;
 *  - o editor não depende deste controlador: sem ele (ou com a IA desligada) tudo segue igual.
 */
import {
  ApiError,
  type AiClient,
  type AiProviderInput,
  type AiStatus,
  type AiTaskEvent,
  type ApprovalMode,
  type BrainProfileView,
  type DemandSpec,
  type GatewayStatusView,
  type MemoryItemView,
  type MemoryScopeName,
  type PendingApproval,
  type ReferenceGrammar,
  type RunCreateInput,
  type RunSnapshot,
  type RunSummary,
  type UsageSummary,
} from "@capia/engine-bindings";
import { createStore, type Store } from "./createStore";

export interface ChatMessage {
  id: number;
  role: "user" | "assistant" | "tool" | "system";
  text: string;
  /** Mensagem de ferramenta: nome e resultado. */
  tool?: { name: string; state: "started" | "ok" | "failed" };
  /** O texto ainda está chegando (streaming). */
  streaming?: boolean;
}

export type JobState = "running" | "done" | "failed" | "cancelled";

export interface AiJob {
  id: string;
  kind: string;
  state: JobState;
  /** Progresso humano (etapa + contadores). */
  stage?: string | undefined;
  done?: number | undefined;
  total?: number | undefined;
  result?: Record<string, unknown> | undefined;
  error?: { code: string; message: string } | undefined;
}

export interface PlanOffer {
  token: string;
  label: string;
  jobId: string;
  summary: Record<string, unknown>;
}

export interface AiState {
  status: AiStatus | null;
  statusError: string | null;
  messages: ChatMessage[];
  conversationId: string | null;
  chatTask: string | null;
  pending: { taskId: string; plan: PendingApproval } | null;
  mode: ApprovalMode;
  jobs: Record<string, AiJob>;
  /** Plano de IA (legendas/silêncio) aguardando o usuário aplicar. */
  offer: PlanOffer | null;
  references: Record<string, ReferenceGrammar>;
  demand: DemandSpec | null;
  usage: UsageSummary | null;
  diagnostics: string | null;
  lastError: { code: string; message: string } | null;
  /** Fase 5: execuções de IA (Runs), memória e fontes. */
  runs: RunSummary[];
  selectedRun: string | null;
  runDetail: RunSnapshot | null;
  memory: MemoryItemView[];
  gateway: GatewayStatusView | null;
}

const MAX_MESSAGES = 200;

const text = (v: unknown, fallback = ""): string => (typeof v === "string" ? v : fallback);

function errOf(e: unknown): { code: string; message: string } {
  if (e instanceof ApiError) return { code: e.code, message: e.message };
  return { code: "ERROR", message: e instanceof Error ? e.message : String(e) };
}

export class AiController {
  readonly store: Store<AiState>;
  private msgId = 0;
  private disposed = false;
  private pollTimer: ReturnType<typeof setInterval> | null = null;
  /** Depois de uma ação do usuário o engine pode demorar a assumir: continua olhando por um tempo. */
  private hotUntil = 0;

  constructor(
    private readonly client: AiClient,
    /** Notifica o editor (ex.: toast); o conteúdo já vem sem segredo. */
    private readonly notify: (
      tone: "info" | "success" | "error",
      title: string,
      detail?: string,
    ) => void = () => {
      /* sem notificação */
    },
    /** O que o editor mostra agora: dá referente a "este clipe" no chat. */
    private readonly uiContext: () => {
      selected_clips: string[];
      playhead_ticks: number;
      sequence: string | null;
    } = () => ({ selected_clips: [], playhead_ticks: 0, sequence: null }),
  ) {
    this.store = createStore<AiState>({
      status: null,
      statusError: null,
      messages: [],
      conversationId: null,
      chatTask: null,
      pending: null,
      mode: "ask",
      jobs: {},
      offer: null,
      references: {},
      demand: null,
      usage: null,
      diagnostics: null,
      lastError: null,
      runs: [],
      selectedRun: null,
      runDetail: null,
      memory: [],
      gateway: null,
    });
  }

  get state(): AiState {
    return this.store.get();
  }

  dispose(): void {
    this.disposed = true;
    this.stopRunPolling();
  }

  // ------------------------------------------------------------------------- Fase 5: Runs

  async refreshRuns(): Promise<void> {
    const r = await this.wrap(() => this.client.runList());
    if (!r || this.disposed) return;
    this.store.set({ runs: r.runs });
    if (this.state.selectedRun) await this.refreshRunDetail();
  }

  async refreshRunDetail(): Promise<void> {
    const id = this.state.selectedRun;
    if (!id) return;
    const r = await this.wrap(() => this.client.runGet(id));
    // a seleção pode ter mudado enquanto a resposta voltava
    if (r && !this.disposed && this.state.selectedRun === id) this.store.set({ runDetail: r });
  }

  /** Cria (e inicia) uma Run. A UI só descreve o pedido; plano/edição são do engine. */
  async createRun(input: RunCreateInput): Promise<string | null> {
    this.hotUntil = Date.now() + 30_000;
    const r = await this.wrap(() => this.client.runCreate(input, true));
    if (!r) return null;
    this.store.set({ selectedRun: r.run.id, runDetail: null });
    await this.refreshRuns();
    return r.run.id;
  }

  async selectRun(id: string | null): Promise<void> {
    this.store.set({ selectedRun: id, runDetail: null });
    if (id) await this.refreshRunDetail();
  }

  /** Atualiza Runs enquanto o painel está aberto (reconecta pelo snapshot; nada em memória é fonte). */
  startRunPolling(intervalMs = 1000): void {
    if (this.pollTimer || this.disposed) return;
    this.pollTimer = setInterval(() => {
      const busy = this.state.runs.some((r) => r.status === "running" || r.status === "pending");
      if (busy || Date.now() < this.hotUntil) void this.refreshRuns();
    }, intervalMs);
  }

  stopRunPolling(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    this.pollTimer = null;
  }

  async decide(optionId: string, payload?: unknown): Promise<void> {
    this.hotUntil = Date.now() + 30_000;
    const run = this.state.runDetail?.run;
    const pending = run?.pending;
    if (!run || !pending) return;
    // a decisão vai amarrada ao id que a UI está vendo: decisão velha o engine recusa
    await this.wrap(() => this.client.runDecide(run.id, pending.id, optionId, payload));
    await this.refreshRuns();
  }

  async pauseRun(id: string): Promise<void> {
    await this.wrap(() => this.client.runPause(id));
    await this.refreshRuns();
  }

  async resumeRun(id: string): Promise<void> {
    this.hotUntil = Date.now() + 30_000;
    await this.wrap(() => this.client.runResume(id));
    await this.refreshRuns();
  }

  async cancelRun(id: string): Promise<void> {
    await this.wrap(() => this.client.runCancel(id));
    await this.refreshRuns();
  }

  async rerun(id: string): Promise<void> {
    this.hotUntil = Date.now() + 30_000;
    const r = await this.wrap(() => this.client.runRerun(id));
    if (r) await this.selectRun(r.run.id);
    await this.refreshRuns();
  }

  async makeVariants(id: string, count: number, axis: string[]): Promise<void> {
    this.hotUntil = Date.now() + 30_000;
    const r = await this.wrap(() => this.client.runVariants(id, count, axis));
    if (r) await this.selectRun(r.run.id);
    await this.refreshRuns();
  }

  // ----------------------------------------------------------------- Fase 5: memória e fontes

  async refreshMemory(): Promise<void> {
    const r = await this.wrap(() => this.client.memoryList());
    if (r && !this.disposed) this.store.set({ memory: r.items });
  }

  async addMemory(scope: MemoryScopeName, content: string): Promise<void> {
    const text = content.trim();
    if (!text) return;
    await this.wrap(() => this.client.memoryAdd(scope, text));
    await this.refreshMemory();
  }

  /** Só um clique explícito do usuário ativa uma proposta da IA. */
  async approveMemory(id: string): Promise<void> {
    await this.wrap(() => this.client.memoryApprove(id));
    await this.refreshMemory();
  }

  async rejectMemory(id: string): Promise<void> {
    await this.wrap(() => this.client.memoryReject(id));
    await this.refreshMemory();
  }

  async archiveMemory(id: string): Promise<void> {
    await this.wrap(() => this.client.memoryArchive(id));
    await this.refreshMemory();
  }

  async deleteMemory(id: string): Promise<void> {
    await this.wrap(() => this.client.memoryDelete(id));
    await this.refreshMemory();
  }

  async refreshGateway(): Promise<void> {
    const r = await this.wrap(() => this.client.gatewayStatus());
    if (r && !this.disposed) this.store.set({ gateway: r });
  }

  async setSourceEnabled(id: string, enabled: boolean): Promise<void> {
    const r = await this.wrap(() => this.client.gatewaySetEnabled(id, enabled));
    if (r) this.store.set({ gateway: r });
  }

  async setGenerationEnabled(enabled: boolean): Promise<void> {
    const r = await this.wrap(() => this.client.generationSetEnabled(enabled));
    if (r) this.store.set({ gateway: r });
  }

  // ------------------------------------------------------------------------------ configuração

  async refresh(): Promise<void> {
    try {
      const status = await this.client.status();
      this.store.set({ status, statusError: null });
    } catch (e) {
      this.store.set({ statusError: errOf(e).message });
    }
  }

  private async wrap<T>(fn: () => Promise<T>): Promise<T | null> {
    try {
      return await fn();
    } catch (e) {
      const err = errOf(e);
      this.store.set({ lastError: err });
      this.notify("error", err.code, err.message);
      return null;
    }
  }

  async setEnabled(enabled: boolean): Promise<void> {
    const status = await this.wrap(() => this.client.setEnabled(enabled));
    if (status) this.store.set({ status });
  }

  /** `apiKey` não é guardado em lugar nenhum: sai direto para o engine e some desta função. */
  async saveProvider(input: AiProviderInput, apiKey?: string): Promise<boolean> {
    const r = await this.wrap(() => this.client.saveProvider(input, apiKey));
    if (!r) return false;
    await this.refresh();
    return true;
  }

  async deleteProvider(id: string): Promise<void> {
    const status = await this.wrap(() => this.client.deleteProvider(id));
    if (status) this.store.set({ status });
  }

  async deleteCredential(providerId: string): Promise<void> {
    const status = await this.wrap(() => this.client.deleteCredential(providerId));
    if (status) this.store.set({ status });
  }

  async saveModel(endpoint: Record<string, unknown>): Promise<boolean> {
    const status = await this.wrap(() => this.client.saveModel(endpoint));
    if (status) this.store.set({ status });
    return status !== null;
  }

  async deleteModel(id: string): Promise<void> {
    const status = await this.wrap(() => this.client.deleteModel(id));
    if (status) this.store.set({ status });
  }

  async setBrain(profile: BrainProfileView): Promise<boolean> {
    const status = await this.wrap(() => this.client.setBrain(profile));
    if (status) this.store.set({ status });
    return status !== null;
  }

  /** `apiKey` sai direto para o engine (write-only). Resultado chega pelo job `connect`. */
  async connect(preset: string, apiKey: string, model?: string): Promise<void> {
    const r = await this.wrap(() => this.client.connect(preset, apiKey, model));
    if (r) this.track(r.task_id, "connect");
  }

  async probe(endpointId: string): Promise<void> {
    const r = await this.wrap(() => this.client.probe(endpointId));
    if (r) this.track(r.task_id, "probe");
  }

  async importModels(providerId: string): Promise<void> {
    const r = await this.wrap(() => this.client.importModels(providerId));
    if (r) this.track(r.task_id, "import_models");
  }

  async loadDiagnostics(): Promise<void> {
    const r = await this.wrap(() => this.client.diagnostics());
    if (r) this.store.set({ diagnostics: r.text });
  }

  async loadUsage(): Promise<void> {
    const r = await this.wrap(() => this.client.usage());
    if (r) this.store.set({ usage: r });
  }

  // ------------------------------------------------------------------------------ tarefas

  private track(id: string, kind: string): void {
    this.store.set((s) => ({ jobs: { ...s.jobs, [id]: { id, kind, state: "running" } } }));
  }

  async transcribe(assetId: string): Promise<void> {
    const r = await this.wrap(() => this.client.transcribe(assetId));
    if (r) this.track(r.task_id, "transcribe");
  }

  async planCaptions(sequence: string, assetId: string): Promise<void> {
    this.store.set({ offer: null });
    const r = await this.wrap(() => this.client.planCaptions(sequence, assetId));
    if (r) this.track(r.task_id, "captions");
  }

  async planSilence(sequence: string, assetId: string, clip?: string): Promise<void> {
    this.store.set({ offer: null });
    const r = await this.wrap(() =>
      this.client.planSilence(sequence, assetId, clip ? { clip } : {}),
    );
    if (r) this.track(r.task_id, "silence");
  }

  async detectScenes(assetId: string): Promise<void> {
    const r = await this.wrap(() => this.client.detectScenes(assetId));
    if (r) this.track(r.task_id, "scenes");
  }

  async analyzeReference(assetId: string): Promise<void> {
    const r = await this.wrap(() => this.client.analyzeReference(assetId));
    if (r) this.track(r.task_id, "reference");
  }

  async interpretDemand(documents: string[], assets: string[], note?: string): Promise<void> {
    const r = await this.wrap(() =>
      this.client.interpretDemand({ documents, assets, ...(note ? { note } : {}) }),
    );
    if (r) this.track(r.task_id, "demand");
  }

  async loadDemand(): Promise<void> {
    const r = await this.wrap(() => this.client.getDemand());
    const first = r?.specs[0];
    if (first) this.store.set({ demand: first });
  }

  async saveDemand(spec: DemandSpec): Promise<void> {
    const r = await this.wrap(() => this.client.saveDemand(spec));
    if (r) this.store.set({ demand: r.spec });
  }

  /** O usuário aplica o plano oferecido (legendas/silêncio): preview→apply_plan no engine. */
  async applyOffer(): Promise<boolean> {
    const offer = this.state.offer;
    if (!offer) return false;
    const r = await this.wrap(() => this.client.applyPlan(offer.token));
    if (!r) return false;
    this.store.set({ offer: null });
    this.notify("success", offer.label);
    return true;
  }

  discardOffer(): void {
    this.store.set({ offer: null });
  }

  async cancel(taskId: string): Promise<void> {
    await this.wrap(() => this.client.cancelTask(taskId));
  }

  // ------------------------------------------------------------------------------ chat

  setMode(mode: ApprovalMode): void {
    this.store.set({ mode });
  }

  private push(m: Omit<ChatMessage, "id">): number {
    const id = (this.msgId += 1);
    this.store.set((s) => ({ messages: [...s.messages, { ...m, id }].slice(-MAX_MESSAGES) }));
    return id;
  }

  private patchMsg(id: number, patch: Partial<ChatMessage>): void {
    this.store.set((s) => ({
      messages: s.messages.map((m) => (m.id === id ? { ...m, ...patch } : m)),
    }));
  }

  async send(text: string): Promise<void> {
    const t = text.trim();
    if (!t || this.state.chatTask) return;
    this.push({ role: "user", text: t });
    const r = await this.wrap(() =>
      this.client.assistantSend(
        t,
        this.state.mode,
        this.state.conversationId ?? undefined,
        this.uiContext(),
      ),
    );
    if (!r) return;
    this.store.set({ chatTask: r.task_id, conversationId: r.conversation_id, pending: null });
    this.track(r.task_id, "assistant");
  }

  async approve(): Promise<void> {
    const p = this.state.pending;
    if (!p) return;
    const r = await this.wrap(() => this.client.assistantApprove(p.taskId, p.plan.plan_token));
    this.store.set({ pending: null, chatTask: null });
    if (r) {
      this.push({ role: "assistant", text: text(r.task.final_text) });
    }
  }

  async reject(): Promise<void> {
    const p = this.state.pending;
    if (!p) return;
    await this.wrap(() => this.client.assistantReject(p.taskId));
    this.store.set({ pending: null, chatTask: null });
    this.push({ role: "system", text: "—" });
  }

  clearChat(): void {
    this.store.set({ messages: [], conversationId: null, pending: null });
  }

  // ------------------------------------------------------------------------------ eventos

  /** Evento `ai_task` vindo do `events.poll` do editor. */
  handleEvent(ev: AiTaskEvent): void {
    if (this.disposed) return;
    const job = this.state.jobs[ev.task_id];
    const isChat = ev.task_id === this.state.chatTask;
    const patchJob = (p: Partial<AiJob>) => {
      this.store.set((s) => {
        const cur = s.jobs[ev.task_id];
        if (!cur) return {};
        return { jobs: { ...s.jobs, [ev.task_id]: { ...cur, ...p } } };
      });
    };
    const d = ev.data;
    switch (ev.phase) {
      case "progress": {
        patchJob({
          stage: typeof d.stage === "string" ? d.stage : undefined,
          done:
            typeof d.chunk === "number" ? d.chunk : typeof d.step === "number" ? d.step : undefined,
          total:
            typeof d.chunks === "number"
              ? d.chunks
              : typeof d.steps === "number"
                ? d.steps
                : undefined,
        });
        break;
      }
      case "text": {
        if (!isChat) break;
        const delta = typeof d.delta === "string" ? d.delta : "";
        const msgs = this.state.messages;
        const last = msgs[msgs.length - 1];
        if (last?.role === "assistant" && last.streaming === true) {
          this.patchMsg(last.id, { text: last.text + delta });
        } else {
          this.push({ role: "assistant", text: delta, streaming: true });
        }
        break;
      }
      case "reset": {
        const msgs = this.state.messages;
        const last = msgs[msgs.length - 1];
        if (isChat && last?.role === "assistant" && last.streaming === true)
          this.patchMsg(last.id, { text: "" });
        break;
      }
      case "tool": {
        if (!isChat) break;
        const name = typeof d.name === "string" ? d.name : "?";
        const started = d.state === "started";
        this.push({
          role: "tool",
          text: name,
          tool: { name, state: started ? "started" : d.ok === true ? "ok" : "failed" },
        });
        break;
      }
      case "approval": {
        const plan = d.plan as PendingApproval | undefined;
        if (plan) this.store.set({ pending: { taskId: ev.task_id, plan } });
        break;
      }
      case "done":
      case "error":
      case "cancelled":
        this.finish(ev, job);
        break;
      default:
        break;
    }
  }

  private finishStreaming(): void {
    this.store.set((s) => ({
      messages: s.messages.map((m) => (m.streaming ? { ...m, streaming: false } : m)),
    }));
  }

  private finish(ev: AiTaskEvent, job: AiJob | undefined): void {
    const result = (ev.data.result ?? undefined) as Record<string, unknown> | undefined;
    const state: JobState =
      ev.phase === "cancelled"
        ? "cancelled"
        : ev.phase === "done" && !failedAtAll(result)
          ? "done"
          : "failed";
    // o assistente devolve a falha do provedor como registro `status: "failed"` (fase `done`):
    // também é erro para o usuário — nunca um silêncio
    const failed = result?.status === "failed" ? result.error : undefined;
    const failedPair = Array.isArray(failed)
      ? { code: text(failed[0], "ERROR"), message: text(failed[1]) }
      : typeof failed === "object" && failed !== null
        ? {
            code: text((failed as Record<string, unknown>).code, "ERROR"),
            message: text((failed as Record<string, unknown>).message),
          }
        : undefined;
    const error =
      ev.phase === "error"
        ? { code: text(ev.data.code, "ERROR"), message: text(ev.data.message) }
        : failedPair;
    this.store.set((s) => ({
      jobs: {
        ...s.jobs,
        [ev.task_id]: { id: ev.task_id, kind: job?.kind ?? "?", state, result, error },
      },
    }));
    if (ev.task_id === this.state.chatTask) {
      this.finishStreaming();
      const awaiting = this.state.pending?.taskId === ev.task_id;
      if (!awaiting) this.store.set({ chatTask: null });
      if (error) this.push({ role: "system", text: `${error.code}: ${error.message}` });
      // sem texto em streaming (ex.: resposta do cache) e com resposta final no resultado
      const final = typeof result?.final_text === "string" ? result.final_text : "";
      const msgs = this.state.messages;
      const lastUser = msgs.map((m) => m.role).lastIndexOf("user");
      const streamed = msgs.slice(lastUser + 1).some((m) => m.role === "assistant");
      if (final && !streamed) this.push({ role: "assistant", text: final });
    }
    if (job?.kind === "captions" || job?.kind === "silence") {
      const token = typeof result?.plan_token === "string" ? result.plan_token : null;
      if (state === "done" && token) {
        this.store.set({
          offer: {
            token,
            jobId: ev.task_id,
            label: job.kind === "captions" ? "captions" : "silence",
            summary: result ?? {},
          },
        });
      }
    }
    if (job?.kind === "reference" && state === "done") {
      const g = result?.grammar as ReferenceGrammar | undefined;
      if (g)
        this.store.set((s) => ({ references: { ...s.references, [g.provenance.asset_id]: g } }));
    }
    if (job?.kind === "demand" && state === "done") {
      const spec = result?.spec as DemandSpec | undefined;
      if (spec) this.store.set({ demand: spec });
    }
    if (job?.kind === "probe" || job?.kind === "import_models" || job?.kind === "connect")
      void this.refresh();
    if (error) {
      this.store.set({ lastError: error });
      this.notify("error", error.code, error.message);
    }
  }
}

function failedAtAll(result: Record<string, unknown> | undefined): boolean {
  return result?.status === "failed";
}
