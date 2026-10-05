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
  capabilities: Record<string, CapabilityState> | unknown;
  context_window: number;
  max_output_tokens: number;
  pricing?: unknown;
  enabled: boolean;
  last_probe?: {
    success: boolean;
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
    opts: { clip?: string; params?: Record<string, unknown>; rippleScope?: "track" | "sequence" } = {},
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
  interpretDemand(input: { documents: string[]; assets: string[]; note?: string; force?: boolean }) {
    return this.json<{ task_id: string }>("ai.demand.interpret", input);
  }
  getDemand() {
    return this.json<{ specs: DemandSpec[] }>("ai.demand.get");
  }
  saveDemand(spec: DemandSpec) {
    return this.json<{ spec: DemandSpec }>("ai.demand.save", { spec });
  }
  assistantSend(text: string, mode: ApprovalMode, conversationId?: string) {
    return this.json<{ task_id: string; conversation_id: string }>("ai.assistant.send", {
      text,
      mode,
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
}
