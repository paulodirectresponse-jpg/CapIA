/**
 * Contrato tipado da API do editor (crate `capia-editor-api`). A UI **nunca** altera o documento:
 * envia comandos (`execute`) e aplica os patches devolvidos à sua réplica (ARCHITECTURE §7).
 * Tempo é sempre inteiro em Ticks (705.600.000/s); cabe num `number` (teto de 24 h < 2^53).
 */

import { AiClient } from "./ai";
import { SupportClient } from "./support";

export type Ticks = number;
export const TICKS_PER_SECOND = 705_600_000;

/** Racional serializado pelo engine: "30" ou "30000/1001". */
export type RationalText = string;

export type TrackKind = "visual" | "audio";
export type TrackRole =
  "main" | "overlay" | "text" | "captions" | "voice" | "music" | "sfx" | { custom: string };

export interface Track {
  id: string;
  name: string;
  kind: TrackKind;
  role: TrackRole;
  magnetic: boolean;
  locked: boolean;
  hidden: boolean;
  muted: boolean;
  solo: boolean;
  sync_lock: boolean;
  group: string | null;
}

export interface TextStyle {
  font_family: string;
  size_permille: number;
  weight: number;
  align: "left" | "center" | "right";
  color: string;
  background?: string | null;
  stroke?: string | null;
  stroke_permille?: number;
}

export const DEFAULT_TEXT_STYLE: TextStyle = {
  font_family: "sans",
  size_permille: 60,
  weight: 400,
  align: "center",
  color: "#FFFFFF",
};

export type ClipContent =
  | { type: "media"; asset: string; has_video: boolean; has_audio: boolean }
  | { type: "image"; asset: string }
  | { type: "text"; text: string; style?: TextStyle }
  | { type: "solid"; color: string }
  | { type: "nested"; sequence: string; follow_length?: boolean };

export type Interp =
  "hold" | "linear" | { bezier: { x1: number; y1: number; x2: number; y2: number } };

export interface Keyframe {
  time: Ticks;
  value: number;
  interp: Interp;
}

export type Animatable = { static: number } | { animated: Keyframe[] };

export type TransitionKind = "dissolve" | "fade" | "slide_in";

export interface Transition {
  kind: TransitionKind;
  duration: Ticks;
}

export interface Clip {
  id: string;
  track: string;
  start: Ticks;
  duration: Ticks;
  name: string;
  enabled: boolean;
  content: ClipContent;
  source_in: Ticks;
  speed: RationalText;
  reversed: boolean;
  properties: Record<string, Animatable>;
  group?: string | null;
  transition_in?: Transition | null;
}

export interface Marker {
  id: string;
  time: Ticks;
  label: string;
}

export interface SequenceHeader {
  name: string;
  frame_rate: RationalText;
  sample_rate: number;
  width?: number;
  height?: number;
  folder?: string | null;
}

/** Conteúdo de uma sequence (replica local; mantida por patches). */
export interface SequenceModel {
  header: SequenceHeader;
  tracks: Track[];
  clips: Record<string, Clip>;
  markers: Record<string, Marker>;
  frame_ticks: Ticks;
}

export interface SequenceSummary {
  id: string;
  name: string;
  frame_rate: RationalText;
  frame_ticks: Ticks;
  sample_rate: number;
  width: number;
  height: number;
  folder: string | null;
  duration: Ticks;
  clip_count: number;
  nested_usage: number;
}

export interface Folder {
  id: string;
  name: string;
  parent: string | null;
}

export interface Deliverable {
  id: string;
  name: string;
  sequence: string;
  preset: string;
  path: string;
  width?: number | null;
  height?: number | null;
}

export type AssetStatus = "online" | "offline" | "modified";
export type AssetKind = "video" | "audio" | "image";

export interface AssetRow {
  id: string;
  in_document: boolean;
  name: string;
  kind: AssetKind | null;
  duration: Ticks | null;
  has_video: boolean | null;
  has_audio: boolean | null;
  width: number | null;
  height: number | null;
  size_bytes: number | null;
  status: AssetStatus;
  path: string | null;
  has_file: boolean;
}

export interface ProjectSnapshot {
  revision: number;
  can_undo: boolean;
  can_redo: boolean;
  project: { path: string; name: string | null };
  sequences: SequenceSummary[];
  folders: Folder[];
  deliverables: Deliverable[];
  assets: AssetRow[];
}

/** Op primitiva do engine (patch): `new = null` remove a entidade. */
export type PatchOp =
  | { op: "clip"; sequence: string; id: string; old: Clip | null; new: Clip | null }
  | {
      op: "track";
      sequence: string;
      id: string;
      old: { index: number; track: Track } | null;
      new: { index: number; track: Track } | null;
    }
  | { op: "marker"; sequence: string; id: string; old: Marker | null; new: Marker | null }
  | { op: "sequence"; id: string; old: SequenceHeader | null; new: SequenceHeader | null }
  | { op: "asset"; id: string; old: unknown; new: unknown }
  | { op: "folder"; id: string; old: Folder | null; new: Folder | null }
  | { op: "deliverable"; id: string; old: Deliverable | null; new: Deliverable | null };

export interface ChangeSet {
  revision: number;
  can_undo: boolean;
  can_redo: boolean;
  patches?: PatchOp[];
  sequence_summaries?: (SequenceSummary | null)[];
  touched_sequences?: string[];
  refs?: Record<string, string>;
  results?: { id?: string; created?: { kind: string; id: string }[]; warnings?: string[] }[];
  replayed?: boolean;
}

export interface HistoryEntry {
  id: number;
  label: string;
  actor: { kind: string; id: string };
  timestamp_ms: number;
  revision: number;
  commands: { operation_id: string; command_type: string; label: string }[];
  applied: boolean;
}

export interface HistoryList {
  entries: HistoryEntry[];
  cursor: number;
}

export interface EncoderCapability {
  ffmpeg_name: string;
  codec: string;
  hardware: boolean;
  available: boolean;
  policy: string;
  reason_unavailable?: string | null;
}

export interface ExportItem {
  id: string;
  sequence: string;
  preset?: "h264-mp4" | "intermediate";
  path: string;
  width?: number | null;
  height?: number | null;
  overwrite?: boolean;
  encoder?: string | null;
}

/** Comando do Command Engine (JSON do `capia-commands`); o `operation_id` é preenchido pelo cliente. */
export type CommandBody = { type: string } & Record<string, unknown>;

export type EditorEvent =
  | { kind: "document_changed"; change: ChangeSet }
  /** Evento de uma tarefa de IA (progresso, texto em streaming, aprovação, fim). */
  | {
      kind: "ai_task";
      task_id: string;
      phase: string;
      data: Record<string, unknown>;
    }
  /** O documento mudou por um comando (de qualquer cliente): quem estiver defasado ressincroniza. */
  | { kind: "revision_changed"; revision: number }
  | { kind: "assets_changed"; assets: AssetRow[] }
  | { kind: "import_finalized"; ticket_id: string; result: { asset_id: string; outcome: string } }
  | { kind: "export_item_started"; batch: string; id: string }
  | {
      kind: "export_progress";
      batch: string;
      id: string;
      done_frames: number;
      total_frames: number;
    }
  | {
      kind: "export_item_finished";
      batch: string;
      id: string;
      ok: boolean;
      cancelled?: boolean;
      report?: Record<string, unknown>;
      error?: ApiErrorBody;
    }
  | { kind: "export_batch_finished"; batch: string; cancelled: boolean }
  | { kind: "import_failed" | "import_cancelled" | "import_interrupted"; ticket_id: string };

export interface ApiErrorBody {
  code: string;
  message: string;
  details?: {
    entities?: { kind: string; id: string }[];
    hint?: Record<string, unknown>;
    command_index?: number;
  } & Record<string, unknown>;
}

export class ApiError extends Error {
  readonly code: string;
  readonly details: ApiErrorBody["details"];
  constructor(body: ApiErrorBody) {
    super(body.message);
    this.name = "ApiError";
    this.code = body.code;
    this.details = body.details;
  }
}

/** Resposta binária (quadros, miniaturas). */
export interface BinaryReply {
  mime: string;
  bytes: ArrayBuffer;
  meta: Record<string, unknown>;
}

/** Transporte do editor: JSON e bytes. Tauri IPC (produto) ou HTTP local (E2E/dev). */
export interface EditorTransport {
  call(method: string, params?: Record<string, unknown>): Promise<unknown>;
  callBinary(method: string, params?: Record<string, unknown>): Promise<BinaryReply>;
}

let opCounter = 0;
function nextOperationId(): string {
  opCounter += 1;
  return `ui-${Date.now().toString(36)}-${opCounter.toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Cliente tipado: cada método devolve o que o engine devolveu (sem lógica de edição aqui). */
export class EditorClient {
  /** Serviço de IA (`ai.*`): o editor funciona igual se ele nunca for usado. */
  readonly ai: AiClient;
  /** Privacidade, diagnóstico e atualização (`support.*`/`update.*`, Fase 6). Só o app desktop os expõe. */
  readonly support: SupportClient;

  constructor(private readonly transport: EditorTransport) {
    this.ai = new AiClient(transport);
    this.support = new SupportClient(transport);
  }

  private json<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    return this.transport.call(method, params) as Promise<T>;
  }

  engineInfo() {
    return this.json<Record<string, unknown> & { media_available: boolean }>("engine.info");
  }
  createProject(path: string) {
    return this.json<ProjectSnapshot>("project.create", { path });
  }
  openProject(path: string) {
    return this.json<ProjectSnapshot>("project.open", { path });
  }
  closeProject() {
    return this.json<{ closed: boolean }>("project.close");
  }
  snapshot() {
    return this.json<ProjectSnapshot>("project.snapshot");
  }
  sequence(id: string) {
    return this.json<SequenceModel>("sequence.get", { sequence: id });
  }

  /** Executa uma transação (um item de histórico). `operation_id`s são gerados aqui. */
  execute(label: string, commands: CommandBody[]) {
    const withIds = commands.map((c) => ({ operation_id: nextOperationId(), ...c }));
    return this.json<ChangeSet>("command.execute", { label, commands: withIds });
  }
  undo() {
    return this.json<ChangeSet>("command.undo");
  }
  redo() {
    return this.json<ChangeSet>("command.redo");
  }
  history() {
    return this.json<HistoryList>("history.list");
  }
  /** Relatório (sem aplicar) do undo seletivo de tudo que um ator fez (ex.: `run:<id>`). */
  undoReport(actorId: string) {
    return this.json<{
      entries: number[];
      conflicts: { entry_id: number; blocked_by: number; blocked_by_actor: { id: string } }[];
    }>("history.undo_report", { actor_id: actorId });
  }
  /** Desfaz só o que o ator fez, como NOVA entrada de histórico (modo `safe` nunca apaga edição manual). */
  undoSelective(actorId: string, mode: "safe" | "partial" = "safe", label?: string) {
    return this.json<ChangeSet>("history.undo_selective", {
      actor_id: actorId,
      mode,
      ...(label ? { label } : {}),
    });
  }

  importAssets(paths: string[]) {
    return this.json<{
      tickets: { ticket_id: string; path: string }[];
      errors: { path: string; error: ApiErrorBody }[];
    }>("assets.import", { paths });
  }
  assets() {
    return this.json<AssetRow[]>("assets.list");
  }
  verifyAsset(asset: string) {
    return this.json<Record<string, unknown>>("assets.verify", { asset });
  }
  relink(asset: string, path: string, opts: { force?: boolean; dryRun?: boolean } = {}) {
    return this.json<ChangeSet & Record<string, unknown>>("assets.relink", {
      asset,
      path,
      force: opts.force ?? false,
      dry_run: opts.dryRun ?? false,
    });
  }
  relinkFolder(folder: string) {
    return this.json<Record<string, unknown>>("assets.relink_folder", { folder });
  }
  peaks(asset: string, buckets: number, from = 0, to?: number) {
    return this.json<{ asset: string; buckets: number; peaks: number[] }>("media.peaks", {
      asset,
      buckets,
      from,
      ...(to === undefined ? {} : { to }),
    });
  }
  thumbnail(asset: string, at = 0, maxDim = 240) {
    return this.transport.callBinary("media.thumbnail", { asset, at, max_dim: maxDim });
  }
  renderFrame(sequence: string, at: Ticks, width: number, height: number) {
    return this.transport.callBinary("render.frame", { sequence, at, width, height });
  }

  /** PCM f32le estéreo do mix de `[from, from+duration)` (ticks) — monitoração do preview. */
  renderAudio(sequence: string, from: Ticks, duration: Ticks, sampleRate = 48_000) {
    return this.transport.callBinary("render.audio", {
      sequence,
      from,
      duration,
      sample_rate: sampleRate,
    });
  }

  encoders() {
    return this.json<EncoderCapability[]>("export.encoders");
  }
  startExport(items: ExportItem[]) {
    return this.json<{ batch: string; items: string[] }>("export.start", { items });
  }
  cancelExport(batch: string) {
    return this.json<{ cancelled: boolean }>("export.cancel", { id: batch });
  }
  pollEvents() {
    return this.json<{ events: EditorEvent[] }>("events.poll");
  }
}

/** Transporte HTTP (dev/E2E): `POST {base}/api/<método>`; bytes com metadados em `X-Capia-Meta`. */
export function createHttpTransport(base = "", token?: string): EditorTransport {
  const headers = (): Record<string, string> => ({
    "content-type": "application/json",
    ...(token ? { "x-capia-token": token } : {}),
  });
  const post = async (method: string, params: Record<string, unknown> | undefined) => {
    const res = await fetch(`${base}/api/${method}`, {
      method: "POST",
      headers: headers(),
      body: JSON.stringify(params ?? {}),
    });
    if (!res.ok) {
      let body: ApiErrorBody;
      try {
        body = (await res.json()) as ApiErrorBody;
      } catch {
        body = { code: "HTTP_ERROR", message: `HTTP ${String(res.status)}` };
      }
      throw new ApiError(body);
    }
    return res;
  };
  return {
    async call(method, params) {
      return (await post(method, params)).json() as Promise<unknown>;
    },
    async callBinary(method, params) {
      const res = await post(method, params);
      const meta = JSON.parse(res.headers.get("x-capia-meta") ?? "{}") as Record<string, unknown>;
      return {
        mime: res.headers.get("content-type") ?? "application/octet-stream",
        bytes: await res.arrayBuffer(),
        meta,
      };
    },
  };
}
