/**
 * Contrato tipado do serviço `ai.*` (crate `capia-intelligence`, Fase 4). A UI **nunca** recebe
 * segredos: a chave do provider só viaja UMA vez, no `saveProvider` (campo write-only), e o engine
 * responde apenas `credential_configured`. Toda edição proposta pela IA volta como um `plan_token`
 * que o usuário aplica (preview → apply_plan); a UI não monta comandos de IA.
 */
import type { EditorTransport } from "./editor";

export type ProviderKind =
  | "open_ai_compatible"
  | "anthropic"
  | "google"
  | "local_open_ai_compatible"
  | "whisper_local"
  | "replay";

export interface AiProviderView {
  id: string;
  kind: ProviderKind;
  display_name: string;
  base_url: string | null;
  enabled: boolean;
  local: boolean;
  needs_credential: boolean;
  /** Existe uma credencial no cofre? (a chave nunca volta para a UI) */
  credential_configured: boolean;
  bound_host: string | null;
  extra_headers: Record<string, string>;
  timeout_s: number;
  max_concurrency: number;
  allow_loopback: boolean;
}

/** Entrada de `saveProvider` (sem credencial: ela vai em `apiKey`). */
export interface AiProviderInput {
  id: string;
  kind: ProviderKind;
  display_name: string;
  base_url?: string | null;
  enabled?: boolean;
  allow_loopback?: boolean;
  extra_headers?: Record<string, string>;
  timeout_s?: number;
  max_concurrency?: number;
}

export type CapabilityName =
  | "text_generation"
  | "streaming"
  | "tool_calling"
  | "structured_output"
  | "vision_input"
  | "audio_input"
  | "speech_to_text";

export type CapabilityOrigin = "declared" | "probed" | "preset";

export interface CapabilityState {
  supported: boolean;
  origin: CapabilityOrigin;
}

export interface AiModelEndpoint {
  id: string;
  provider_id: string;
  model_id: string;
  display_name: string;
  /** `Capabilities` do engine: `{ entries: { <capability>: { supported, origin } } }`. */
  capabilities: { entries?: Record<string, CapabilityState> };
  context_window: number;
  max_output_tokens: number;
  pricing?: unknown;
  enabled: boolean;
  last_probe?: {
    success: boolean;
    /** `ready` tudo ok · `partial` gera texto mas algo falhou · `failed` sem conexão/texto. */
    status?: "ready" | "partial" | "failed";
    connected?: boolean;
    latency_ms: number;
    verified: string[];
    failures: Record<string, string>;
    timestamp_ms: number;
    connection_error?: string | null;
  } | null;
  health: "unknown" | "healthy" | "degraded" | "unavailable";
}

export interface BrainProfileView {
  id: string;
  name: string;
  brain: string;
  role_overrides?: Record<string, string>;
  fallbacks?: Record<string, string[]>;
  budgets?: {
    currency?: string | null;
    max_cost_per_task_micros?: number | null;
    warn_at_micros?: number | null;
    auto_approve_below_micros?: number | null;
    max_tokens_per_task?: number | null;
    max_calls_per_task?: number | null;
  };
  privacy?: {
    never_upload_video: boolean;
    vision_frames_only: boolean;
    transcription_local_only: boolean;
    no_cloud_audio: boolean;
    no_external_document_upload: boolean;
  };
}

export interface AiPreset {
  key: string;
  display_name: string;
  kind: ProviderKind;
  base_url: string | null;
}

export interface AiStatus {
  enabled: boolean;
  any_usable_model: boolean;
  secret_backend: string;
  providers: AiProviderView[];
  models: AiModelEndpoint[];
  profiles: BrainProfileView[];
  active_profile: string | null;
  presets: AiPreset[];
}

export type AiTaskPhase =
  "started" | "progress" | "text" | "reset" | "tool" | "approval" | "done" | "error" | "cancelled";

export interface PendingApproval {
  plan_token: string;
  label: string;
  operations: number;
}

export interface AiTaskEvent {
  kind: "ai_task";
  task_id: string;
  phase: AiTaskPhase;
  data: Record<string, unknown>;
}

export type AiTaskState = "running" | "done" | "failed" | "cancelled";

export interface AiTaskInfo {
  task_id: string;
  kind: string;
  state: AiTaskState;
  result: Record<string, unknown> | null;
  error: [string, string] | null;
}

export interface SourceRef {
  doc_id: string;
  doc_name: string;
  unit_id: string;
  page?: number;
  t_us?: number;
  quote: string;
}

export type Basis = "explicit" | "inferred" | "unverified";

export interface SpecField {
  value: string | null;
  basis: Basis;
  sources: SourceRef[];
}

export interface SpecItem {
  text: string;
  basis: Basis;
  sources: SourceRef[];
}

export interface DemandSpec {
  schema_version: number;
  id: string;
  version: number;
  title: string | null;
  product: SpecField;
  audience: SpecField;
  offer: SpecField;
  objective: SpecField;
  tone: SpecField;
  platform: SpecField;
  duration_and_format: SpecField;
  cta: SpecField;
  key_claims: SpecItem[];
  must_include: SpecItem[];
  must_avoid: SpecItem[];
  constraints: SpecItem[];
  assets_mentioned: SpecItem[];
  open_questions: { question: string; reason: string }[];
  documents: { id: string; name: string; kind: string; units: number; truncated: boolean }[];
  verification: {
    sources_proposed: number;
    sources_verified: number;
    sources_dropped: number;
    fields_unverified: number;
  };
  provenance: {
    endpoint_id: string;
    model_id: string;
    cost_micros: number | null;
    created_ms: number;
    task_id: string;
    truncated_docs: string[];
  };
}

export interface ReferenceGrammar {
  schema_version: number;
  duration_us: number;
  scan_fps: number;
  shots: { start_us: number; end_us: number; entry: string }[];
  cut_rhythm: {
    shot_count: number;
    cuts_per_minute: number;
    mean_shot_us: number;
    median_shot_us: number;
    p10_shot_us: number;
    p90_shot_us: number;
    shortest_us: number;
    longest_us: number;
    histogram: number[];
  };
  transitions: { cuts: number; dissolves: number; fades: number };
  audio: {
    speech_ratio_permille: number;
    silence_ratio_permille: number;
    mean_db: number;
    peak_db: number;
    dynamic_range_db: number;
    db_per_10s: number[];
  } | null;
  speech: {
    language: string | null;
    word_count: number;
    words_per_minute: number;
    first_word_us: number | null;
    hook_text: string;
  } | null;
  structure: { name: string; start_us: number; end_us: number; shots: number }[];
  provenance: { producer: string; asset_id: string; fully_local: boolean; notes: string[] };
}

export type ApprovalMode = "ask" | "auto";

export interface UsageSummary {
  calls: number;
  failed_calls: number;
  cache_hits: number;
  input_tokens: number;
  output_tokens: number;
  known_cost_micros: number;
  currency: string | null;
  unknown_cost_calls: number;
}

// ---- Fase 5: AI Runs, memória, gateway -------------------------------------------------------

export type RunStatusName =
  "pending" | "running" | "waiting_user" | "paused" | "completed" | "failed" | "cancelled";

export type RunStageName =
  "understand" | "plan" | "validate_plan" | "acquire" | "edit" | "review" | "correct" | "done";

export interface RunDecisionOption {
  id: string;
  label: string;
}

export interface RunPendingDecision {
  id: string;
  kind: string;
  question: string;
  options: RunDecisionOption[];
  context?: Record<string, unknown>;
  consequences: string;
  default_option?: string | null;
  resume_stage: RunStageName;
}

export interface RunUsage {
  cost_micros: number;
  unknown_cost_calls: number;
  tokens: number;
  provider_calls: number;
  generations: number;
  review_loops: number;
  replans: number;
  wall_time_ms: number;
}

export interface RunBudgetView {
  max_cost_micros: number | null;
  max_tokens: number | null;
  max_provider_calls: number | null;
  max_generations: number | null;
  max_review_loops: number;
  max_replans: number;
  max_wall_time_ms: number | null;
}

export interface ProducedSequenceView {
  deliverable: string;
  sequence_id: string;
  role: string;
}

export interface RunSummary {
  id: string;
  status: RunStatusName;
  stage: RunStageName;
  revision: number;
  created_ms: number;
  updated_ms: number;
  completed_ms: number | null;
  usage: RunUsage;
  budget: RunBudgetView;
  pending: RunPendingDecision | null;
  error: { code: string; message: string } | null;
  parent_run_id: string | null;
  variant_group_id: string | null;
  deliverables: string[] | null;
  sequences: ProducedSequenceView[];
  resume_class: string;
}

export interface RunFinding {
  id: string;
  key: string;
  severity: "blocker" | "major" | "minor" | "info";
  category: string;
  expected: string;
  observed: string;
  needs_replan: boolean;
}

export interface RunStageRow {
  seq: number;
  stage: RunStageName;
  attempt: number;
  status: string;
}

export interface RunSnapshot {
  run: RunSummary & {
    inputs?: Record<string, unknown>;
    policy?: Record<string, unknown>;
    checkpoint?: Record<string, unknown>;
    production_plan?: Record<string, unknown> | null;
  };
  usage: RunUsage;
  stages: RunStageRow[];
  provenance: Record<string, unknown>[];
  last_event_seq: number;
  resume_class: string;
}

export interface RunEvent {
  seq: number;
  kind: string;
  ts_ms: number;
  data: Record<string, unknown>;
}

export interface RunCreateInput {
  briefText?: string;
  assets?: string[];
  references?: string[];
  documents?: string[];
  deliverables?: { key: string; maxDurationS?: number; width?: number; height?: number }[];
  variants?: { count: number; axis: string[] };
  clientId?: string;
  /** Política: `always` pede aprovação do plano; `auto` aplica quando validado. */
  planApproval?: "always" | "auto";
  allowGateway?: boolean;
  allowGeneration?: boolean;
  maxCostMicros?: number;
}

export type MemoryScopeName = "system" | "user" | "client" | "project";
export type MemoryStatusName = "proposed" | "active" | "rejected" | "archived";

export interface MemoryItemView {
  id: string;
  scope: MemoryScopeName;
  client_id: string | null;
  kind: string;
  content: string;
  source: string;
  status: MemoryStatusName;
  confidence: number;
  evidence: { kind: string; detail: string }[];
  origin_run: string | null;
}

export interface GatewayAdapterView {
  id: string;
  kind: string;
  enabled: boolean;
  hosts: string[];
  paid: boolean;
}

export interface GatewayStatusView {
  adapters: GatewayAdapterView[];
  generation: { enabled: boolean; available: boolean };
}

/** Cliente do `ai.*`: só traduz chamadas; nenhuma regra de IA vive na UI. */
export class AiClient {
  constructor(private readonly transport: EditorTransport) {}

  private json<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    return this.transport.call(method, params) as Promise<T>;
  }

  status() {
    return this.json<AiStatus>("ai.status");
  }
  setEnabled(enabled: boolean) {
    return this.json<AiStatus>("ai.enabled.set", { enabled });
  }
  /** `apiKey` é write-only: vai uma vez e nunca volta (o engine responde só `credential_configured`). */
  saveProvider(provider: AiProviderInput, apiKey?: string) {
    return this.json<{ provider: AiProviderView }>("ai.provider.save", {
      provider,
      ...(apiKey ? { api_key: apiKey } : {}),
    });
  }
  /** "Conectar IA": provedor + chave → modelo padrão, probe por capability e Brain Profile `auto`. */
  connect(preset: string, apiKey: string, model?: string) {
    return this.json<{ task_id: string }>("ai.connect", {
      preset,
      api_key: apiKey,
      ...(model ? { model } : {}),
    });
  }
  deleteProvider(id: string) {
    return this.json<AiStatus>("ai.provider.delete", { id });
  }
  deleteCredential(providerId: string) {
    return this.json<AiStatus>("ai.credential.delete", { provider_id: providerId });
  }
  saveModel(endpoint: Record<string, unknown>) {
    return this.json<AiStatus>("ai.model.save", { endpoint });
  }
  deleteModel(id: string) {
    return this.json<AiStatus>("ai.model.delete", { id });
  }
  importModels(providerId: string) {
    return this.json<{ task_id: string }>("ai.models.import", { provider_id: providerId });
  }
  probe(endpointId: string) {
    return this.json<{ task_id: string }>("ai.model.probe", { endpoint_id: endpointId });
  }
  setBrain(profile: BrainProfileView) {
    return this.json<AiStatus>("ai.brain.set", { profile });
  }
  /** Texto de diagnóstico **já redigido** pelo engine. */
  diagnostics() {
    return this.json<{ text: string }>("ai.diagnostics");
  }

  transcribe(assetId: string, opts: { language?: string; force?: boolean } = {}) {
    return this.json<{ task_id: string }>("ai.transcribe", { asset_id: assetId, ...opts });
  }
  planCaptions(sequence: string, assetId: string, language?: string) {
    return this.json<{ task_id: string }>("ai.captions.plan", {
      sequence,
      asset_id: assetId,
      ...(language ? { language } : {}),
    });
  }
  planSilence(
    sequence: string,
    assetId: string,
    opts: {
      clip?: string;
      params?: Record<string, unknown>;
      rippleScope?: "track" | "sequence";
    } = {},
  ) {
    return this.json<{ task_id: string }>("ai.silence.plan", {
      sequence,
      asset_id: assetId,
      ...(opts.clip ? { clip: opts.clip } : {}),
      ...(opts.params ? { params: opts.params } : {}),
      ...(opts.rippleScope ? { ripple_scope: opts.rippleScope } : {}),
    });
  }
  /** Aplica um plano produzido por uma tarefa de IA desta sessão (mesmo gate preview→apply). */
  applyPlan(planToken: string) {
    return this.json<Record<string, unknown>>("ai.plan.apply", { plan_token: planToken });
  }
  detectScenes(assetId: string) {
    return this.json<{ task_id: string }>("ai.scenes.detect", { asset_id: assetId });
  }
  analyzeReference(assetId: string, force = false) {
    return this.json<{ task_id: string }>("ai.reference.analyze", { asset_id: assetId, force });
  }
  getReference(assetId: string) {
    return this.json<{ available: boolean; grammar?: ReferenceGrammar }>("ai.reference.get", {
      asset_id: assetId,
    });
  }
  interpretDemand(input: {
    documents: string[];
    assets: string[];
    note?: string;
    force?: boolean;
  }) {
    return this.json<{ task_id: string }>("ai.demand.interpret", input);
  }
  getDemand() {
    return this.json<{ specs: DemandSpec[] }>("ai.demand.get");
  }
  saveDemand(spec: DemandSpec) {
    return this.json<{ spec: DemandSpec }>("ai.demand.save", { spec });
  }
  assistantSend(
    text: string,
    mode: ApprovalMode,
    conversationId?: string,
    context?: { selected_clips: string[]; playhead_ticks: number; sequence: string | null },
  ) {
    return this.json<{ task_id: string; conversation_id: string }>("ai.assistant.send", {
      text,
      mode,
      ...(context ?? {}),
      ...(conversationId ? { conversation_id: conversationId } : {}),
    });
  }
  assistantApprove(taskId: string, planToken: string) {
    return this.json<{ task: Record<string, unknown> }>("ai.assistant.approve", {
      task_id: taskId,
      plan_token: planToken,
    });
  }
  assistantReject(taskId: string) {
    return this.json<{ task: Record<string, unknown> }>("ai.assistant.reject", { task_id: taskId });
  }
  cancelTask(taskId: string) {
    return this.json<{ cancelled: boolean }>("ai.task.cancel", { task_id: taskId });
  }
  task(taskId: string) {
    return this.json<AiTaskInfo>("ai.task.get", { task_id: taskId });
  }
  usage(taskId?: string) {
    return this.json<UsageSummary>("ai.usage.summary", taskId ? { task_id: taskId } : {});
  }

  // ---- Fase 5: Runs -------------------------------------------------------------------------
  runCreate(input: RunCreateInput, start = true) {
    const policy: Record<string, unknown> = {};
    if (input.planApproval) policy.plan = input.planApproval;
    if (input.allowGateway !== undefined) policy.allow_gateway = input.allowGateway;
    if (input.allowGeneration !== undefined) policy.allow_generation = input.allowGeneration;
    return this.json<{ run: RunSummary }>("ai.run.create", {
      inputs: {
        ...(input.briefText ? { brief_text: input.briefText } : {}),
        assets: input.assets ?? [],
        references: input.references ?? [],
        documents: input.documents ?? [],
        deliverables: (input.deliverables ?? []).map((d) => ({
          key: d.key,
          ...(d.maxDurationS ? { max_duration_s: d.maxDurationS } : {}),
          ...(d.width ? { width: d.width } : {}),
          ...(d.height ? { height: d.height } : {}),
        })),
        ...(input.variants ? { variants: input.variants } : {}),
        ...(input.clientId ? { client_id: input.clientId } : {}),
      },
      policy,
      ...(input.maxCostMicros ? { budget: { max_cost_micros: input.maxCostMicros } } : {}),
      start,
    });
  }
  runList(limit = 100) {
    return this.json<{ runs: RunSummary[] }>("ai.run.list", { limit });
  }
  runGet(runId: string) {
    return this.json<RunSnapshot>("ai.run.get", { run_id: runId });
  }
  runEvents(runId: string, after = 0) {
    return this.json<{ events: RunEvent[] }>("ai.run.events", { run_id: runId, after });
  }
  runPause(runId: string) {
    return this.json<{ run: RunSummary }>("ai.run.pause", { run_id: runId });
  }
  runResume(runId: string) {
    return this.json<{ run: RunSummary }>("ai.run.resume", { run_id: runId });
  }
  runCancel(runId: string) {
    return this.json<{ run: RunSummary }>("ai.run.cancel", { run_id: runId });
  }
  runDecide(runId: string, decisionId: string, option: string, payload?: unknown) {
    return this.json<{ run: RunSummary }>("ai.run.decide", {
      run_id: runId,
      decision_id: decisionId,
      option,
      ...(payload !== undefined ? { payload } : {}),
    });
  }
  runRerun(runId: string, brief?: string) {
    return this.json<{ run: RunSummary }>("ai.run.rerun", {
      run_id: runId,
      ...(brief ? { brief } : {}),
    });
  }
  runVariants(runId: string, count: number, axis: string[]) {
    return this.json<{ run: RunSummary; variant_group: string }>("ai.run.variants", {
      run_id: runId,
      count,
      axis,
    });
  }
  runGroup(variantGroup: string) {
    return this.json<{ runs: RunSummary[] }>("ai.run.group", { variant_group: variantGroup });
  }
  runProvenance(runId: string) {
    return this.json<{ provenance: Record<string, unknown>[] }>("ai.run.provenance", {
      run_id: runId,
    });
  }
  // ---- Fase 5: memória e fontes -------------------------------------------------------------
  memoryList(scope?: MemoryScopeName, status?: MemoryStatusName) {
    return this.json<{ items: MemoryItemView[] }>("ai.memory.list", {
      ...(scope ? { scope } : {}),
      ...(status ? { status } : {}),
    });
  }
  memoryAdd(scope: MemoryScopeName, content: string, clientId?: string) {
    return this.json<{ item: MemoryItemView }>("ai.memory.add", {
      scope,
      content,
      ...(clientId ? { client_id: clientId } : {}),
    });
  }
  memoryApprove(id: string, scope?: MemoryScopeName, clientId?: string) {
    return this.json<{ item: MemoryItemView }>("ai.memory.approve", {
      id,
      ...(scope ? { scope } : {}),
      ...(clientId ? { client_id: clientId } : {}),
    });
  }
  memoryReject(id: string) {
    return this.json<{ item: MemoryItemView }>("ai.memory.reject", { id });
  }
  memoryArchive(id: string) {
    return this.json<{ item: MemoryItemView }>("ai.memory.archive", { id });
  }
  memoryDelete(id: string) {
    return this.json<{ deleted: boolean }>("ai.memory.delete", { id });
  }
  gatewayStatus() {
    return this.json<GatewayStatusView>("ai.gateway.status");
  }
  gatewaySetEnabled(id: string, enabled: boolean) {
    return this.json<GatewayStatusView>("ai.gateway.set_enabled", { id, enabled });
  }
  generationSetEnabled(enabled: boolean) {
    return this.json<GatewayStatusView>("ai.generation.set_enabled", { enabled });
  }
}
