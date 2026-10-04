/**
 * Controlador do editor: único ponto que fala com o engine (via `EditorClient`). Mantém a réplica
 * do documento (aplicando patches), o estado de UI (abas, seleção, playhead, zoom…) e executa as
 * ações do usuário como **comandos do Command Engine** — nunca altera o documento diretamente
 * (regra principal da Fase 3). Chamadas ao engine são serializadas para preservar a ordem dos
 * patches; o ghost de gestos vem do WASM do core (`@capia/ui-timeline`).
 */
import {
  ApiError,
  EMPTY_MODEL,
  TICKS_PER_SECOND,
  applyChange,
  fromSnapshot,
  withAssets,
  withSequenceModel,
  type ApiErrorBody,
  type ChangeSet,
  type Clip,
  type CommandBody,
  type EditorClient,
  type EditorEvent,
  type ExportItem,
  type HistoryList,
  type ReadModel,
  type SequenceModel,
  type Ticks,
  type Transition,
  type TransitionKind,
} from "@capia/engine-bindings";
import type { MovePlan, TimelineCore } from "@capia/ui-timeline";
import { describeError, type MessageKey, type Translate, createTranslator } from "../i18n";
import {
  buildLookup,
  eventToBinding,
  isTypingTarget,
  resolveBindings,
  type ActionId,
} from "../lib/keymap";
import {
  DEFAULT_PREFS,
  browserStorage,
  loadPrefs,
  savePrefs,
  type KeyValueStorage,
  type Prefs,
} from "../lib/prefs";
import { frameTicks, snapToFrame } from "../lib/timecode";
import {
  FORMAT_PRESETS,
  addTextCommands,
  copyPayload,
  defaultTrackCommands,
  deleteCommands,
  dropAssetCommands,
  dropSequenceCommands,
  duplicateAtCommands,
  duplicateCommands,
  groupCommands,
  pasteCommands,
  setPropertyCommand,
  splitCommands,
  trackFlagsCommand,
  trimToPlayheadCommands,
  ungroupCommands,
  type ClipboardPayload,
  type DropSpot,
  type EditContext,
  type FormatPreset,
  type TextPreset,
  addTrackCommand,
} from "./edit";
import { createStore, type Store } from "./createStore";
import { MediaVisuals } from "./visuals";
import { PerfLog } from "./perf";
import { noPlatform, type PlatformServices } from "../platform";
import { ipcFrameSource, type FrameSource } from "../preview/frames";
import { AudioMonitor, pcmFromReply, webAudioContext, type AudioCtxLike } from "../preview/audio";

export interface ToastItem {
  id: number;
  tone: "info" | "success" | "error";
  title: string;
  detail?: string | undefined;
}

export type ExportItemState = "pending" | "running" | "done" | "failed" | "cancelled";

export interface ExportRunItem {
  id: string;
  label: string;
  state: ExportItemState;
  done: number;
  total: number;
  error?: ApiErrorBody;
  report?: Record<string, unknown>;
}

export interface ExportRun {
  batch: string | null;
  items: ExportRunItem[];
  finished: boolean;
}

export type Phase = "boot" | "welcome" | "opening" | "ready";

export interface UiState {
  phase: Phase;
  engine: { mediaAvailable: boolean } | null;
  model: ReadModel;
  tabs: string[];
  active: string | null;
  /** Sequence pai de cada sequence aberta por navegação em nested (breadcrumb). */
  parentOf: Record<string, string>;
  /** Sequence em renomeação inline (recém-criada). */
  renaming: string | null;
  selection: string[];
  selectedTrack: string | null;
  playhead: Ticks;
  playing: boolean;
  shuttle: number;
  tool: "select" | "blade";
  clipboard: ClipboardPayload | null;
  toasts: ToastItem[];
  save: "saved" | "saving" | "error";
  pendingImports: number;
  exportRun: ExportRun | null;
  history: HistoryList | null;
  prefs: Prefs;
  prefsRecovered: boolean;
  /** Linha da biblioteca em destaque após importar (rolagem/foco). */
  lastImported: string | null;
  /** Quantas operações ao engine estão em andamento. */
  busy: number;
  /** Pedidos de UI sem estado próprio (ajustar zoom / abrir exportação): a UI reage ao contador. */
  zoomFitNonce: number;
  exportNonce: number;
  /** Diagnóstico do último erro (expansível). */
  lastError: { message: string; body: ApiErrorBody } | null;
}

const MAX_TOASTS = 4;

export interface ControllerOptions {
  platform?: PlatformServices;
  /** Fonte dos quadros do preview (padrão: bytes pelo transporte; o desktop pode dar o SharedBuffer). */
  frames?: FrameSource;
  /** Contexto de áudio (testes injetam um falso; `null` desliga a monitoração). */
  audioContext?: () => AudioCtxLike | null;
  storage?: KeyValueStorage | null;
  loadCore?: () => Promise<TimelineCore | null>;
  pollMs?: number;
}

export class EditorController {
  readonly store: Store<UiState>;
  private t: Translate;
  private core: TimelineCore | null = null;
  private coreLoading: Promise<void> | null = null;
  private coreSeq: string | null = null;
  private queue: Promise<unknown> = Promise.resolve();
  private poller: ReturnType<typeof setInterval> | null = null;
  private raf: number | null = null;
  private lastTick = 0;
  private pollTicks = 0;
  private toastId = 0;
  private toastTimers = new Map<number, ReturnType<typeof setTimeout>>();
  private prefsTimer: ReturnType<typeof setTimeout> | null = null;
  private loading = new Map<string, Promise<void>>();
  private readonly storage: KeyValueStorage | null;
  private disposed = false;
  private frameHeight = 1080;
  readonly visuals: MediaVisuals;
  readonly perf = new PerfLog();
  readonly frames: FrameSource;
  private readonly audio: AudioMonitor;

  constructor(
    readonly client: EditorClient,
    private readonly opts: ControllerOptions = {},
  ) {
    this.frames = opts.frames ?? ipcFrameSource(client);
    this.audio = new AudioMonitor(async (seq, from, duration) => {
      const r = await client.renderAudio(seq, from, duration);
      return pcmFromReply(r.bytes, r.meta);
    }, opts.audioContext ?? webAudioContext);
    this.visuals = new MediaVisuals(client, (ms) => {
      this.perf.record("thumb", ms);
    });
    this.storage = opts.storage === undefined ? browserStorage() : opts.storage;
    const { prefs, recovered } = loadPrefs(this.storage);
    this.t = createTranslator(prefs.language);
    this.store = createStore<UiState>({
      phase: "boot",
      engine: null,
      model: EMPTY_MODEL,
      tabs: [],
      active: null,
      parentOf: {},
      renaming: null,
      selection: [],
      selectedTrack: null,
      playhead: 0,
      playing: false,
      shuttle: 1,
      tool: "select",
      clipboard: null,
      toasts: [],
      save: "saved",
      pendingImports: 0,
      exportRun: null,
      history: null,
      prefs,
      prefsRecovered: recovered,
      lastImported: null,
      busy: 0,
      zoomFitNonce: 0,
      exportNonce: 0,
      lastError: null,
    });
  }

  get state(): UiState {
    return this.store.get();
  }

  get platform(): PlatformServices {
    return this.opts.platform ?? noPlatform;
  }

  setTranslator(t: Translate): void {
    this.t = t;
  }

  getCore(): TimelineCore | null {
    return this.core;
  }

  // -------------------------------------------------------------------------- ciclo de vida

  async boot(): Promise<void> {
    try {
      const info = await this.client.engineInfo();
      this.store.set({ engine: { mediaAvailable: info.media_available }, phase: "welcome" });
    } catch (e) {
      this.reportError(e);
      this.store.set({ phase: "welcome" });
    }
    void this.ensureCore();
  }

  private ensureCore(): Promise<void> {
    if (this.core || !this.opts.loadCore) return Promise.resolve();
    this.coreLoading ??= this.opts.loadCore().then(
      (c) => {
        this.core = c;
        this.syncCore(true);
      },
      () => {
        this.core = null;
      },
    );
    return this.coreLoading;
  }

  /** Mantém o WASM com a réplica da sequence ativa. */
  private syncCore(full: boolean, change?: ChangeSet): void {
    const core = this.core;
    const s = this.state;
    const id = s.active;
    if (!core || !id) return;
    const model = s.model.models[id];
    if (!model) return;
    try {
      if (full || this.coreSeq !== id || !change) {
        core.loadSequence(id, model);
        this.coreSeq = id;
      } else if (change.patches) {
        core.applyPatches(change.patches);
      }
    } catch {
      try {
        core.loadSequence(id, model);
        this.coreSeq = id;
      } catch {
        this.coreSeq = null;
      }
    }
  }

  dispose(): void {
    this.disposed = true;
    this.stopPolling();
    this.pause();
    for (const h of this.toastTimers.values()) clearTimeout(h);
    this.toastTimers.clear();
    if (this.prefsTimer) clearTimeout(this.prefsTimer);
    this.flushPrefs();
    this.visuals.dispose();
    this.frames.dispose?.();
    this.audio.dispose();
  }

  private startPolling(): void {
    if (this.poller || this.disposed) return;
    this.poller = setInterval(() => {
      void this.pollOnce();
    }, this.opts.pollMs ?? 250);
  }

  private stopPolling(): void {
    if (this.poller) clearInterval(this.poller);
    this.poller = null;
  }

  private async pollOnce(): Promise<void> {
    if (this.state.phase !== "ready") return;
    try {
      const { events } = await this.client.pollEvents();
      for (const ev of events) this.handleEvent(ev);
      // disponibilidade (online/offline/modificado) é recalculada pelo engine em `assets.list`
      // (só `metadata`, sem hash): releitura leve a cada ~4 s para o aviso de offline aparecer
      this.pollTicks += 1;
      if (this.pollTicks % 16 === 0 && this.state.pendingImports === 0) await this.refreshAssets();
    } catch {
      /* falha transitória do transporte: tenta no próximo ciclo */
    }
  }

  /** Processa um evento do engine (exposto para testes). */
  handleEvent(ev: EditorEvent): void {
    switch (ev.kind) {
      case "document_changed":
        this.applyChangeSet(ev.change);
        break;
      case "revision_changed":
        // outro cliente (CLI, IA, outra janela) alterou o documento: ressincroniza se defasado
        void this.resyncIfBehind(ev.revision);
        break;
      case "assets_changed":
        this.store.set((s) => ({ model: withAssets(s.model, ev.assets) }));
        break;
      case "import_finalized":
        this.store.set((s) => ({
          pendingImports: Math.max(0, s.pendingImports - 1),
          lastImported: ev.result.asset_id,
        }));
        break;
      case "import_failed":
      case "import_cancelled":
      case "import_interrupted":
        this.store.set((s) => ({ pendingImports: Math.max(0, s.pendingImports - 1) }));
        this.toast("error", this.t("media.importing"), ev.kind);
        break;
      case "export_item_started":
        this.updateExport(ev.id, { state: "running" });
        break;
      case "export_progress":
        this.updateExport(ev.id, {
          state: "running",
          done: ev.done_frames,
          total: ev.total_frames,
        });
        break;
      case "export_item_finished":
        if (ev.ok) {
          this.updateExport(ev.id, { state: "done", ...(ev.report ? { report: ev.report } : {}) });
        } else if (ev.cancelled === true) this.updateExport(ev.id, { state: "cancelled" });
        else {
          this.updateExport(ev.id, { state: "failed", ...(ev.error ? { error: ev.error } : {}) });
          if (ev.error) this.reportError(new ApiError(ev.error));
        }
        break;
      case "export_batch_finished":
        this.store.set((s) =>
          s.exportRun ? { exportRun: { ...s.exportRun, finished: true } } : {},
        );
        break;
    }
  }

  /**
   * Relê o projeto quando o engine avisa de uma revisão que a réplica não tem. Roda na fila de
   * comandos (depois dos comandos desta UI já em voo); o evento do próprio comando chega com a
   * revisão que a réplica já aplicou e não faz nada.
   */
  private resyncIfBehind(revision: number): Promise<void> {
    const run = async () => {
      if (this.disposed || revision <= this.state.model.revision) return;
      try {
        const snap = await this.client.snapshot();
        const alive = new Set(snap.sequences.map((q) => q.id));
        const loaded = Object.keys(this.state.model.models).filter((id) => alive.has(id));
        const bodies = await Promise.all(
          loaded.map(async (id) => [id, await this.client.sequence(id)] as const),
        );
        this.store.set((s) => {
          let model = fromSnapshot(snap);
          for (const [id, m] of bodies) model = withSequenceModel(model, id, m);
          const tabs = s.tabs.filter((id) => alive.has(id));
          const active = s.active && alive.has(s.active) ? s.active : (tabs[0] ?? null);
          const seq = active ? model.models[active] : undefined;
          return {
            model,
            tabs,
            active,
            selection: seq ? s.selection.filter((id) => id in seq.clips) : [],
            history: null,
          };
        });
        this.syncCore(true);
      } catch (e) {
        this.reportError(e);
      }
    };
    const p = this.queue.then(run, run);
    this.queue = p.catch(() => null);
    return p;
  }

  private updateExport(id: string, patch: Partial<ExportRunItem>): void {
    this.store.set((s) => {
      const run = s.exportRun;
      if (!run) return {};
      return {
        exportRun: { ...run, items: run.items.map((i) => (i.id === id ? { ...i, ...patch } : i)) },
      };
    });
  }

  // ------------------------------------------------------------------------------ projeto

  async createProject(path: string): Promise<boolean> {
    const ok = await this.openWith(() => this.client.createProject(path));
    // projeto novo já abre com uma sequence pronta para editar (comando comum, desfazível)
    if (ok && Object.keys(this.state.model.sequences).length === 0) {
      await this.createSequence();
      this.store.set({ renaming: null });
    }
    return ok;
  }

  async openProject(path: string): Promise<boolean> {
    return this.openWith(() => this.client.openProject(path));
  }

  private async openWith(fn: () => ReturnType<EditorClient["openProject"]>): Promise<boolean> {
    this.store.set({ phase: "opening" });
    try {
      const snap = await fn();
      this.coreSeq = null;
      this.store.set({
        model: fromSnapshot(snap),
        phase: "ready",
        tabs: [],
        active: null,
        parentOf: {},
        selection: [],
        selectedTrack: null,
        playhead: 0,
        history: null,
        exportRun: null,
        save: "saved",
      });
      const first = snap.sequences[0];
      if (first) await this.openSequence(first.id);
      this.startPolling();
      return true;
    } catch (e) {
      this.store.set({ phase: "welcome" });
      this.reportError(e);
      return false;
    }
  }

  async closeProject(): Promise<void> {
    this.stopPolling();
    this.pause();
    try {
      await this.client.closeProject();
    } catch {
      /* fechar é best-effort */
    }
    this.coreSeq = null;
    this.store.set({
      phase: "welcome",
      model: EMPTY_MODEL,
      tabs: [],
      active: null,
      parentOf: {},
      selection: [],
      history: null,
      exportRun: null,
    });
  }

  // --------------------------------------------------------------------------- sequences/abas

  /** Carrega o conteúdo de uma sequence (uma vez; depois é mantido por patches). */
  loadSequence(id: string): Promise<void> {
    if (this.state.model.models[id]) return Promise.resolve();
    const inflight = this.loading.get(id);
    if (inflight) return inflight;
    const p = (async () => {
      try {
        const seq = await this.client.sequence(id);
        this.store.set((s) => ({ model: withSequenceModel(s.model, id, seq) }));
      } catch (e) {
        this.reportError(e);
      } finally {
        this.loading.delete(id);
      }
    })();
    this.loading.set(id, p);
    return p;
  }

  async openSequence(id: string, from?: string): Promise<void> {
    await this.loadSequence(id);
    this.store.set((s) => ({
      tabs: s.tabs.includes(id) ? s.tabs : [...s.tabs, id],
      active: id,
      parentOf:
        from && from !== id && !(id in s.parentOf) ? { ...s.parentOf, [id]: from } : s.parentOf,
      selection: [],
      selectedTrack: null,
    }));
    this.syncCore(true);
  }

  setActive(id: string): void {
    if (!this.state.tabs.includes(id) || this.state.active === id) return;
    this.store.set({ active: id, selection: [], selectedTrack: null });
    void this.loadSequence(id).then(() => {
      this.syncCore(true);
    });
  }

  closeTab(id: string): void {
    this.store.set((s) => {
      const tabs = s.tabs.filter((t) => t !== id);
      let active = s.active;
      if (active === id) active = tabs[Math.max(0, s.tabs.indexOf(id) - 1)] ?? null;
      return { tabs, active, selection: s.active === id ? [] : s.selection };
    });
    this.syncCore(true);
  }

  closeOtherTabs(id: string): void {
    this.store.set({ tabs: [id], active: id });
    this.syncCore(true);
  }

  /** Cadeia de sequences até a atual (breadcrumb de nested). */
  breadcrumb(): string[] {
    const s = this.state;
    const out: string[] = [];
    let cur = s.active;
    let guard = 0;
    while (cur && guard++ < 32) {
      out.unshift(cur);
      cur = s.parentOf[cur] ?? null;
    }
    return out;
  }

  async openNested(clipId: string): Promise<void> {
    const s = this.state;
    const seq = s.active ? s.model.models[s.active] : undefined;
    const clip = seq?.clips[clipId];
    if (clip?.content.type !== "nested" || !s.active) return;
    await this.openSequence(clip.content.sequence, s.active);
  }

  async createSequence(
    opts: { preset?: FormatPreset | "active"; name?: string; folder?: string | null } = {},
  ): Promise<string | null> {
    const s = this.state;
    const active = s.active ? s.model.sequences[s.active] : undefined;
    let size: { width: number; height: number };
    if (opts.preset && opts.preset !== "active") size = FORMAT_PRESETS[opts.preset];
    else if (active) size = { width: active.width, height: active.height };
    else size = FORMAT_PRESETS.vertical;
    const n = Object.keys(s.model.sequences).length + 1;
    const name = opts.name ?? this.t("project.sequenceName", { n });
    const folder = opts.folder === undefined ? (active?.folder ?? null) : opts.folder;
    const id = `seq-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
    const cmds: CommandBody[] = [
      {
        type: "create_sequence",
        id,
        name,
        frame_rate: active?.frame_rate ?? "30",
        width: size.width,
        height: size.height,
        folder,
      },
      ...defaultTrackCommands(id),
    ];
    const ch = await this.exec("create sequence", cmds);
    if (!ch) return null;
    await this.openSequence(id);
    this.store.set({ renaming: id });
    return id;
  }

  renameSequence(id: string, name: string): Promise<ChangeSet | null> {
    this.store.set({ renaming: null });
    const trimmed = name.trim();
    if (!trimmed || this.state.model.sequences[id]?.name === trimmed) return Promise.resolve(null);
    return this.exec("rename sequence", [{ type: "rename_sequence", sequence: id, name: trimmed }]);
  }

  stopRenaming(): void {
    this.store.set({ renaming: null });
  }

  async duplicateSequence(id: string): Promise<void> {
    const src = this.state.model.sequences[id];
    if (!src) return;
    const newId = `seq-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
    const ch = await this.exec("duplicate sequence", [
      { type: "duplicate_sequence", source: id, new_sequence: newId, name: `${src.name} copy` },
    ]);
    if (ch) {
      await this.openSequence(newId);
      this.store.set({ renaming: newId });
    }
  }

  async deleteSequence(id: string): Promise<void> {
    const ch = await this.exec("delete sequence", [{ type: "delete_sequence", sequence: id }]);
    if (ch) this.closeTab(id);
  }

  moveSequenceToFolder(id: string, folder: string | null): Promise<ChangeSet | null> {
    return this.exec("move sequence", [{ type: "set_sequence_folder", sequence: id, folder }]);
  }

  createFolder(name: string, parent: string | null): Promise<ChangeSet | null> {
    return this.exec("create folder", [{ type: "create_folder", name, parent }]);
  }

  renameFolder(folder: string, name: string): Promise<ChangeSet | null> {
    return this.exec("rename folder", [{ type: "rename_folder", folder, name }]);
  }

  moveFolder(folder: string, parent: string | null): Promise<ChangeSet | null> {
    return this.exec("move folder", [{ type: "move_folder", folder, parent }]);
  }

  deleteFolder(folder: string): Promise<ChangeSet | null> {
    return this.exec("delete folder", [{ type: "delete_folder", folder }]);
  }

  setSequenceFormat(id: string, width: number, height: number): Promise<ChangeSet | null> {
    return this.exec("sequence format", [
      { type: "set_sequence_format", sequence: id, width, height },
    ]);
  }

  makeUnique(clipId: string): Promise<ChangeSet | null> {
    return this.exec("make unique", [{ type: "make_unique", clip: clipId }]);
  }

  // ---------------------------------------------------------------------------- execução

  /**
   * Executa uma transação no engine (serializada). Em erro mostra o toast e devolve `null`; em
   * sucesso aplica o patch à réplica e ao WASM.
   */
  exec(
    label: string,
    commands: CommandBody[],
    opts: { quiet?: boolean } = {},
  ): Promise<ChangeSet | null> {
    if (commands.length === 0) return Promise.resolve(null);
    const run = async (): Promise<ChangeSet | null> => {
      this.store.set((s) => ({ busy: s.busy + 1, save: "saving" }));
      try {
        const t0 = performance.now();
        const change = await this.client.execute(label, commands);
        const t1 = performance.now();
        this.applyChangeSet(change);
        const t2 = performance.now();
        this.perf.record("rpc", t1 - t0);
        this.perf.record("apply", t2 - t1);
        this.perf.record("commit", t2 - t0);
        this.store.set({ save: "saved", lastError: null });
        return change;
      } catch (e) {
        this.store.set({ save: "saved" });
        if (!opts.quiet) this.reportError(e);
        return null;
      } finally {
        this.store.set((s) => ({ busy: Math.max(0, s.busy - 1) }));
      }
    };
    const p = this.queue.then(run, run);
    this.queue = p.catch(() => null);
    return p;
  }

  applyChangeSet(change: ChangeSet): void {
    // o áudio já agendado reflete o documento antigo: reinicia a partir da posição atual
    if (this.state.playing && this.audio.active)
      queueMicrotask(() => {
        this.syncAudio();
      });
    this.store.set((s) => {
      const model = applyChange(s.model, change);
      // a seleção não pode apontar para clips que deixaram de existir
      const seq = s.active ? model.models[s.active] : undefined;
      const selection = seq ? s.selection.filter((id) => id in seq.clips) : s.selection;
      return {
        model,
        selection: selection.length === s.selection.length ? s.selection : selection,
        history: null,
      };
    });
    this.syncCore(false, change);
  }

  undo(): Promise<void> {
    return this.stepHistory(() => this.client.undo());
  }

  redo(): Promise<void> {
    return this.stepHistory(() => this.client.redo());
  }

  private stepHistory(fn: () => Promise<ChangeSet>): Promise<void> {
    const run = async () => {
      try {
        const t0 = performance.now();
        const change = await fn();
        const t1 = performance.now();
        this.applyChangeSet(change);
        const t2 = performance.now();
        this.perf.record("rpc", t1 - t0);
        this.perf.record("apply", t2 - t1);
        this.perf.record("history", t2 - t0);
      } catch (e) {
        if (!(
          e instanceof ApiError &&
          (e.code === "NOTHING_TO_UNDO" || e.code === "NOTHING_TO_REDO")
        ))
          this.reportError(e);
      }
    };
    const p = this.queue.then(run, run);
    this.queue = p.catch(() => null);
    return p;
  }

  async loadHistory(): Promise<void> {
    try {
      this.store.set({ history: await this.client.history() });
    } catch (e) {
      this.reportError(e);
    }
  }

  // -------------------------------------------------------------------------------- contexto

  private editContext(): EditContext | null {
    const s = this.state;
    const seq = s.active ? s.model.models[s.active] : undefined;
    if (!s.active || !seq) return null;
    return {
      seqId: s.active,
      seq,
      playhead: s.playhead,
      selection: s.selection,
      assets: s.model.assets,
    };
  }

  activeSequence(): SequenceModel | null {
    const s = this.state;
    return s.active ? (s.model.models[s.active] ?? null) : null;
  }

  // ------------------------------------------------------------------------------- seleção

  select(ids: string[], mode: "replace" | "add" | "toggle" = "replace"): void {
    this.store.set((s) => {
      let next: string[];
      if (mode === "replace") next = ids;
      else if (mode === "add") next = [...new Set([...s.selection, ...ids])];
      else {
        const cur = new Set(s.selection);
        for (const id of ids) {
          if (cur.has(id)) cur.delete(id);
          else cur.add(id);
        }
        next = [...cur];
      }
      return { selection: next };
    });
  }

  selectAll(): void {
    const seq = this.activeSequence();
    if (seq) this.store.set({ selection: Object.keys(seq.clips) });
  }

  clearSelection(): void {
    this.store.set({ selection: [], selectedTrack: null });
  }

  selectTrack(id: string | null): void {
    this.store.set({ selectedTrack: id });
  }

  /** Seleciona o clip sob o playhead na track escolhida (ou na primeira com clip). */
  selectUnderPlayhead(): void {
    const ctx = this.editContext();
    if (!ctx) return;
    const hit = Object.values(ctx.seq.clips).find(
      (c) => c.start <= ctx.playhead && ctx.playhead < c.start + c.duration,
    );
    if (hit) this.select([hit.id]);
  }

  // ---------------------------------------------------------------------- ações de edição

  split(): Promise<ChangeSet | null> {
    const c = this.editContext();
    return c ? this.exec("split", splitCommands(c)) : Promise.resolve(null);
  }

  /** Corte com a lâmina num ponto específico do clip. */
  bladeCut(clip: string, at: Ticks): Promise<ChangeSet | null> {
    return this.exec("split", [{ type: "split_clip", clip, at }]);
  }

  deleteSelection(ripple: boolean): Promise<ChangeSet | null> {
    const c = this.editContext();
    if (!c) return Promise.resolve(null);
    const cmds = deleteCommands(c, ripple);
    // o conjunto inteiro entra numa transação (um item de histórico)
    return this.exec(ripple ? "ripple delete" : "delete", cmds);
  }

  trimToPlayhead(edge: "in" | "out"): Promise<ChangeSet | null> {
    const c = this.editContext();
    return c ? this.exec("trim", trimToPlayheadCommands(c, edge)) : Promise.resolve(null);
  }

  trimClip(clip: string, edge: "in" | "out", to: Ticks): Promise<ChangeSet | null> {
    return this.exec("trim", [{ type: "trim_clip", clip, edge, to }]);
  }

  group(): Promise<ChangeSet | null> {
    const c = this.editContext();
    return c ? this.exec("group", groupCommands(c)) : Promise.resolve(null);
  }

  ungroup(): Promise<ChangeSet | null> {
    const c = this.editContext();
    return c ? this.exec("ungroup", ungroupCommands(c)) : Promise.resolve(null);
  }

  copy(): void {
    const c = this.editContext();
    const payload = c ? copyPayload(c) : null;
    if (!payload) return;
    this.store.set({ clipboard: payload });
    try {
      this.storage?.setItem("capia.clipboard.v1", JSON.stringify(payload));
    } catch {
      /* clipboard entre projetos é best-effort */
    }
  }

  paste(): Promise<ChangeSet | null> {
    const c = this.editContext();
    let payload = this.state.clipboard;
    if (!payload) {
      try {
        const raw = this.storage?.getItem("capia.clipboard.v1");
        if (raw) payload = JSON.parse(raw) as ClipboardPayload;
      } catch {
        payload = null;
      }
    }
    if (!c || !payload || (payload.version as number) !== 1) return Promise.resolve(null);
    // assets que não existem neste projeto precisam ser importados antes
    const missing = Object.entries(payload.assetPaths).filter(([id]) => !(id in c.assets));
    if (missing.length > 0) {
      const paths = missing.map(([, p]) => p).filter((p): p is string => p !== null);
      if (paths.length > 0) void this.importPaths(paths);
      this.toast("info", this.t("media.importing"), paths.join("\n"));
      return Promise.resolve(null);
    }
    return this.exec(
      "paste",
      pasteCommands(c, payload, c.playhead, this.state.selectedTrack ?? undefined),
    );
  }

  duplicate(): Promise<ChangeSet | null> {
    const c = this.editContext();
    return c ? this.exec("duplicate", duplicateCommands(c)) : Promise.resolve(null);
  }

  /** Aplica o plano de um arrasto (move/reorder/duplicar) como UMA transação. */
  moveClips(plan: MovePlan): Promise<ChangeSet | null> {
    const c = this.editContext();
    if (!c) return Promise.resolve(null);
    if (plan.duplicate && plan.moves.length > 0)
      return this.exec("duplicate", duplicateAtCommands(c, plan.moves));
    if (plan.reorder) {
      const r = plan.reorder;
      return this.exec("reorder", [
        { type: "reorder_clip", clip: r.clip, track: r.track, before: r.before, start: r.start },
      ]);
    }
    if (plan.moves.length === 0) return Promise.resolve(null);
    return this.exec("move", [{ type: "move_clips", moves: plan.moves }]);
  }

  dropAsset(assetId: string, spot: DropSpot): Promise<ChangeSet | null> {
    const c = this.editContext();
    const asset = this.state.model.assets[assetId];
    if (!c || !asset) return Promise.resolve(null);
    const cmds = dropAssetCommands(c, asset, spot);
    if (!Array.isArray(cmds)) {
      this.toast("error", this.t("timeline.invalidDrop"), cmds.error);
      return Promise.resolve(null);
    }
    return this.exec("add media", cmds);
  }

  /** Atalho: adiciona o asset no playhead (botão "adicionar à timeline" da biblioteca). */
  addAssetAtPlayhead(assetId: string): Promise<ChangeSet | null> {
    const s = this.state;
    const seq = this.activeSequence();
    if (!seq) return Promise.resolve(null);
    const main =
      seq.tracks.find((t) => t.kind === "visual" && t.magnetic) ??
      seq.tracks.find((t) => t.kind === "visual");
    const audioOnly = s.model.assets[assetId]?.kind === "audio";
    const track = audioOnly ? seq.tracks.find((t) => t.kind === "audio" && !t.locked) : main;
    return this.dropAsset(
      assetId,
      track
        ? { kind: "track", track: track.id, time: s.playhead }
        : { kind: "new-track", side: "above", time: s.playhead },
    );
  }

  /** Solta uma sequence na timeline como clip nested. */
  dropSequence(sequenceId: string, spot: DropSpot): Promise<ChangeSet | null> {
    const ctx = this.editContext();
    const child = this.state.model.sequences[sequenceId];
    if (!ctx || !child) return Promise.resolve(null);
    return this.exec("add nested", dropSequenceCommands(ctx, sequenceId, child.duration, spot));
  }

  addText(preset: TextPreset): Promise<ChangeSet | null> {
    const c = this.editContext();
    if (!c) return Promise.resolve(null);
    const label = preset === "caption" ? this.t("captions.default") : this.t("text.default");
    const h = this.state.model.sequences[c.seqId]?.height ?? this.frameHeight;
    return this.exec(
      preset === "caption" ? "add caption" : "add text",
      addTextCommands(c, preset, label, h),
    );
  }

  addTrack(kind: "visual" | "audio", role: string): Promise<ChangeSet | null> {
    const id = this.state.active;
    return id ? this.exec("add track", [addTrackCommand(id, kind, role)]) : Promise.resolve(null);
  }

  deleteTrack(track: string): Promise<ChangeSet | null> {
    return this.exec("delete track", [{ type: "delete_track", track }]);
  }

  setTrackFlags(
    track: string,
    flags: Partial<Record<"locked" | "hidden" | "muted" | "solo" | "magnetic", boolean>>,
  ): Promise<ChangeSet | null> {
    return this.exec("track", [trackFlagsCommand(track, flags)]);
  }

  setProperty(clip: Clip, prop: string, value: number): Promise<ChangeSet | null> {
    const frame = this.activeSequence()?.frame_ticks ?? frameTicks("30");
    return this.exec("property", [
      setPropertyCommand(clip, prop, value, this.state.playhead, frame),
    ]);
  }

  /** Várias propriedades de um clip numa só transação (um passo de undo). */
  setProperties(clip: Clip, props: [string, number][]): Promise<ChangeSet | null> {
    const frame = this.activeSequence()?.frame_ticks ?? frameTicks("30");
    return this.exec(
      "property",
      props.map(([p, v]) => setPropertyCommand(clip, p, v, this.state.playhead, frame)),
    );
  }

  setClipSpeed(clip: string, speed: string): Promise<ChangeSet | null> {
    return this.exec("speed", [{ type: "set_clip_speed", clip, speed }]);
  }

  renameClip(clip: string, name: string): Promise<ChangeSet | null> {
    return this.exec("rename clip", [{ type: "rename_clip", clip, name }]);
  }

  setClipEnabled(clip: string, enabled: boolean): Promise<ChangeSet | null> {
    return this.exec("enable clip", [{ type: "set_clip_enabled", clip, enabled }]);
  }

  setText(
    clip: string,
    patch: { text?: string; style?: Record<string, unknown> },
  ): Promise<ChangeSet | null> {
    return this.exec("text", [
      {
        type: "set_text",
        clip,
        ...(patch.text !== undefined ? { text: patch.text } : {}),
        ...(patch.style ? { style: patch.style } : {}),
      },
    ]);
  }

  setTransition(clip: string, transition: Transition | null): Promise<ChangeSet | null> {
    return this.exec("transition", [{ type: "set_transition", clip, transition }]);
  }

  /** Corta uma legenda específica no playhead (o texto é duplicado; o usuário edita depois). */
  splitClipAt(clip: string, at?: Ticks): Promise<ChangeSet | null> {
    const frame = this.frame();
    return this.exec("split", [
      { type: "split_clip", clip, at: snapToFrame(at ?? this.state.playhead, frame) },
    ]);
  }

  /** Une a legenda com a seguinte da mesma track: texto concatenado, fim estendido (1 transação). */
  mergeCaptionWithNext(clip: string): Promise<ChangeSet | null> {
    const seq = this.activeSequence();
    const a = seq?.clips[clip];
    if (!seq || !a || a.content.type !== "text") return Promise.resolve(null);
    const next = Object.values(seq.clips)
      .filter((x) => x.track === a.track && x.start >= a.start + a.duration && x.id !== a.id)
      .sort((x, y) => x.start - y.start)[0];
    if (!next || next.content.type !== "text") return Promise.resolve(null);
    const text = `${a.content.text} ${next.content.text}`.trim();
    return this.exec("merge captions", [
      { type: "delete_clip", clip: next.id, ripple: null },
      { type: "trim_clip", clip: a.id, edge: "out", to: next.start + next.duration },
      { type: "set_text", clip: a.id, text },
    ]);
  }

  /** Aplica um estilo a todas as legendas (tracks de função Captions) da sequence ativa. */
  applyCaptionStyle(style: Record<string, unknown>): Promise<ChangeSet | null> {
    const seq = this.activeSequence();
    if (!seq) return Promise.resolve(null);
    const capTracks = new Set(
      seq.tracks
        .filter((t) => (typeof t.role === "string" ? t.role : "") === "captions")
        .map((t) => t.id),
    );
    const cmds: CommandBody[] = Object.values(seq.clips)
      .filter((cl) => capTracks.has(cl.track) && cl.content.type === "text")
      .map((cl) => ({ type: "set_text", clip: cl.id, style }));
    return cmds.length > 0 ? this.exec("caption style", cmds) : Promise.resolve(null);
  }

  /** Transição na entrada dos clips selecionados que têm clip anterior na mesma track. */
  applyTransitionToSelection(kind: TransitionKind): Promise<ChangeSet | null> {
    const seq = this.activeSequence();
    if (!seq) return Promise.resolve(null);
    const frame = this.frame();
    const cmds: CommandBody[] = [];
    for (const id of this.state.selection) {
      const cl = seq.clips[id];
      if (!cl) continue;
      const prev = Object.values(seq.clips).find(
        (x) => x.track === cl.track && x.start + x.duration === cl.start,
      );
      if (prev) {
        // meio segundo, mas nunca mais que a metade do menor dos dois clips (o engine valida o resto)
        const cap = Math.floor(Math.min(prev.duration, cl.duration) / 2 / frame) * frame;
        const duration = Math.min(Math.round(TICKS_PER_SECOND / 2 / frame) * frame, cap);
        if (duration >= frame)
          cmds.push({ type: "set_transition", clip: id, transition: { kind, duration } });
      }
    }
    if (cmds.length === 0) {
      this.toast("info", this.t("transitions.needsCut"));
      return Promise.resolve(null);
    }
    return this.exec("transition", cmds);
  }

  detachAudio(): Promise<ChangeSet | null> {
    const c = this.editContext();
    if (!c) return Promise.resolve(null);
    const cmds = c.selection
      .map((id) => c.seq.clips[id])
      .filter(
        (cl): cl is Clip =>
          cl?.content.type === "media" && cl.content.has_video && cl.content.has_audio,
      )
      .map((cl) => ({ type: "detach_audio", clip: cl.id }));
    return this.exec("detach audio", cmds);
  }

  addKeyframe(clip: Clip, prop: string, value: number, at?: Ticks): Promise<ChangeSet | null> {
    const frame = this.activeSequence()?.frame_ticks ?? frameTicks("30");
    const t = snapToFrame(at ?? this.state.playhead, frame);
    const clamped = Math.min(clip.start + clip.duration, Math.max(clip.start, t));
    return this.exec("keyframe", [
      { type: "add_keyframe", clip: clip.id, prop, at: clamped, value, interp: "linear" },
    ]);
  }

  deleteKeyframe(clip: string, prop: string, at: Ticks): Promise<ChangeSet | null> {
    return this.exec("keyframe", [{ type: "delete_keyframe", clip, prop, at }]);
  }

  moveKeyframe(clip: string, prop: string, from: Ticks, to: Ticks): Promise<ChangeSet | null> {
    return this.exec("keyframe", [{ type: "move_keyframe", clip, prop, from, to }]);
  }

  setKeyframeInterp(
    clip: string,
    prop: string,
    at: Ticks,
    interp: unknown,
  ): Promise<ChangeSet | null> {
    return this.exec("keyframe", [{ type: "set_keyframe_interp", clip, prop, at, interp }]);
  }

  addMarker(): Promise<ChangeSet | null> {
    const id = this.state.active;
    return id
      ? this.exec("marker", [{ type: "add_marker", sequence: id, time: this.state.playhead }])
      : Promise.resolve(null);
  }

  // ------------------------------------------------------------------------------- mídia

  async importPaths(paths: string[]): Promise<void> {
    const clean = paths.map((p) => p.trim()).filter(Boolean);
    if (clean.length === 0) return;
    try {
      const r = await this.client.importAssets(clean);
      this.store.set((s) => ({ pendingImports: s.pendingImports + r.tickets.length }));
      for (const err of r.errors) this.reportError(new ApiError(err.error));
    } catch (e) {
      this.reportError(e);
    }
  }

  async relink(asset: string, path: string, opts: { force?: boolean } = {}): Promise<boolean> {
    try {
      const r = await this.client.relink(asset, path, { force: opts.force === true });
      if (r.patches) this.applyChangeSet(r);
      this.store.set({ model: withAssets(this.state.model, await this.client.assets()) });
      this.toast("success", this.t("media.relinkDone"));
      return true;
    } catch (e) {
      this.reportError(e);
      return false;
    }
  }

  async relinkFolder(folder: string): Promise<void> {
    try {
      await this.client.relinkFolder(folder);
      this.store.set({ model: withAssets(this.state.model, await this.client.assets()) });
      this.toast("success", this.t("media.relinkDone"));
    } catch (e) {
      this.reportError(e);
    }
  }

  async refreshAssets(): Promise<void> {
    try {
      this.store.set({ model: withAssets(this.state.model, await this.client.assets()) });
    } catch (e) {
      this.reportError(e);
    }
  }

  // ------------------------------------------------------------------------------ export

  async startExport(items: ExportItem[], labels: Record<string, string> = {}): Promise<boolean> {
    try {
      const r = await this.client.startExport(items);
      this.store.set({
        exportRun: {
          batch: r.batch,
          finished: false,
          items: items.map((i) => ({
            id: i.id,
            label: labels[i.id] ?? i.id,
            state: "pending",
            done: 0,
            total: 0,
          })),
        },
      });
      return true;
    } catch (e) {
      this.reportError(e);
      return false;
    }
  }

  async cancelExport(): Promise<void> {
    const b = this.state.exportRun?.batch;
    if (!b) return;
    try {
      await this.client.cancelExport(b);
    } catch (e) {
      this.reportError(e);
    }
  }

  clearExportRun(): void {
    this.store.set({ exportRun: null });
  }

  createDeliverable(d: {
    name: string;
    sequence: string;
    preset: string;
    path: string;
    width?: number;
    height?: number;
  }): Promise<ChangeSet | null> {
    return this.exec("deliverable", [{ type: "create_deliverable", ...d }]);
  }

  deleteDeliverable(id: string): Promise<ChangeSet | null> {
    return this.exec("remove deliverable", [{ type: "delete_deliverable", deliverable: id }]);
  }

  // ----------------------------------------------------------------------------- playback

  seek(t: Ticks): void {
    const seq = this.activeSequence();
    const max = seq ? Math.max(this.sequenceDuration(), 0) : 0;
    this.store.set({ playhead: Math.max(0, Math.min(t, max > 0 ? Math.max(max, t) : t)) });
    if (this.state.playing) this.syncAudio();
  }

  sequenceDuration(): Ticks {
    const s = this.state;
    return (s.active ? s.model.sequences[s.active]?.duration : 0) ?? 0;
  }

  private frame(): Ticks {
    return this.activeSequence()?.frame_ticks ?? frameTicks("30");
  }

  step(frames: number): void {
    this.pause();
    this.seek(this.state.playhead + frames * this.frame());
  }

  goStart(): void {
    this.seek(0);
  }

  goEnd(): void {
    this.seek(this.sequenceDuration());
  }

  play(rate = 1): void {
    if (this.state.playing && this.state.shuttle === rate) return;
    if (rate > 0 && this.state.playhead >= this.sequenceDuration()) this.seek(0);
    this.store.set({ playing: true, shuttle: rate });
    this.lastTick = performance.now();
    this.syncAudio();
    if (this.raf === null) this.tick();
  }

  /** (Re)inicia a monitoração de áudio a 1× na posição atual; em outra velocidade fica mudo. */
  private syncAudio(): void {
    const s = this.state;
    const id = s.active;
    if (s.playing && s.shuttle === 1 && s.prefs.preview.audio && id) {
      this.audio.start(id, s.playhead, this.sequenceDuration());
    } else {
      this.audio.stop();
    }
  }

  setPreviewAudio(on: boolean): void {
    this.setPreview({ audio: on });
    this.syncAudio();
  }

  pause(): void {
    if (this.raf !== null) {
      if (typeof cancelAnimationFrame !== "undefined") cancelAnimationFrame(this.raf);
      else clearTimeout(this.raf);
      this.raf = null;
    }
    this.audio.stop();
    if (this.state.playing) this.store.set({ playing: false, shuttle: 1 });
  }

  togglePlay(): void {
    if (this.state.playing) this.pause();
    else this.play(1);
  }

  /** J/K/L: cada L/J acelera (1×, 2×, 4×); K pausa. */
  shuttleForward(): void {
    const s = this.state;
    this.play(s.playing && s.shuttle > 0 ? Math.min(8, s.shuttle * 2) : 1);
  }

  shuttleBack(): void {
    const s = this.state;
    this.play(s.playing && s.shuttle < 0 ? Math.max(-8, s.shuttle * 2) : -1);
  }

  private tick = (): void => {
    const run = () => {
      const s = this.state;
      if (!s.playing || this.disposed) {
        this.raf = null;
        return;
      }
      const now = performance.now();
      const dt = now - this.lastTick;
      this.lastTick = now;
      const frame = this.frame();
      let next: Ticks;
      if (this.audio.active && s.shuttle === 1) {
        // relógio de áudio mestre; antes do 1º bloco agendado a imagem espera (alguns ms)
        const pos = this.audio.position();
        next = pos === null ? s.playhead : snapToFrame(Math.max(s.playhead, pos), frame);
      } else {
        const adv = Math.round((dt / 1000) * TICKS_PER_SECOND * s.shuttle);
        next = snapToFrame(s.playhead + adv, frame);
      }
      const dur = this.sequenceDuration();
      if ((s.shuttle > 0 && next >= dur) || (s.shuttle < 0 && next <= 0)) {
        this.store.set({ playhead: s.shuttle > 0 ? dur : 0 });
        this.pause();
        return;
      }
      this.store.set({ playhead: next });
      this.raf =
        typeof requestAnimationFrame !== "undefined"
          ? requestAnimationFrame(run)
          : (setTimeout(run, 16) as unknown as number);
    };
    this.raf =
      typeof requestAnimationFrame !== "undefined"
        ? requestAnimationFrame(run)
        : (setTimeout(run, 16) as unknown as number);
  };

  // ---------------------------------------------------------------------------- viewport

  setTool(tool: "select" | "blade"): void {
    this.store.set({ tool });
  }

  setSnapping(on: boolean): void {
    this.store.set((s) => ({
      prefs: { ...s.prefs, timeline: { ...s.prefs.timeline, snapping: on } },
    }));
    this.schedulePrefs();
  }

  setZoom(pps: number): void {
    this.store.set((s) => ({
      prefs: { ...s.prefs, timeline: { ...s.prefs.timeline, pxPerSecond: pps } },
    }));
    this.schedulePrefs();
  }

  // ----------------------------------------------------------------------- atalhos/ações

  /** Executa uma ação de atalho. */
  runAction(id: ActionId): void {
    const s = this.state;
    switch (id) {
      case "playPause":
        this.togglePlay();
        break;
      case "shuttleForward":
        this.shuttleForward();
        break;
      case "shuttleBack":
        this.shuttleBack();
        break;
      case "shuttlePause":
        this.pause();
        break;
      case "stepBack":
        this.step(-1);
        break;
      case "stepForward":
        this.step(1);
        break;
      case "step10Back":
        this.step(-10);
        break;
      case "step10Forward":
        this.step(10);
        break;
      case "goStart":
        this.goStart();
        break;
      case "goEnd":
        this.goEnd();
        break;
      case "split":
        void this.split();
        break;
      case "delete":
        void this.deleteSelection(false);
        break;
      case "rippleDelete":
        void this.deleteSelection(true);
        break;
      case "trimLeft":
        void this.trimToPlayhead("in");
        break;
      case "trimRight":
        void this.trimToPlayhead("out");
        break;
      case "undo":
        void this.undo();
        break;
      case "redo":
        void this.redo();
        break;
      case "copy":
        this.copy();
        break;
      case "paste":
        void this.paste();
        break;
      case "duplicate":
        void this.duplicate();
        break;
      case "group":
        void this.group();
        break;
      case "ungroup":
        void this.ungroup();
        break;
      case "toggleSnapping":
        this.setSnapping(!s.prefs.timeline.snapping);
        break;
      case "zoomIn":
        this.setZoom(s.prefs.timeline.pxPerSecond * 1.25);
        break;
      case "zoomOut":
        this.setZoom(s.prefs.timeline.pxPerSecond / 1.25);
        break;
      case "zoomFit":
        this.store.set((st) => ({ zoomFitNonce: st.zoomFitNonce + 1 }));
        break;
      case "selectAll":
        this.selectAll();
        break;
      case "deselect":
        this.clearSelection();
        break;
      case "newSequence":
        void this.createSequence();
        break;
      case "addMarker":
        void this.addMarker();
        break;
      case "export":
        this.store.set((st) => ({ exportNonce: st.exportNonce + 1 }));
        break;
      case "toggleLeftPanel":
        this.setPanels({ leftCollapsed: !s.prefs.panels.leftCollapsed });
        break;
      case "toggleRightPanel":
        this.setPanels({ rightCollapsed: !s.prefs.panels.rightCollapsed });
        break;
    }
  }

  /** Item sendo arrastado (biblioteca/projeto): o preview do drop precisa da duração. */
  private dragInfo: { span: Ticks } | null = null;

  setDragging(info: { span: Ticks } | null): void {
    this.dragInfo = info;
  }

  dragSpan(): Ticks | null {
    return this.dragInfo?.span ?? null;
  }

  /** Tecla global → ação. Devolve `true` se tratou (a UI chama `preventDefault`). */
  handleKey(e: KeyboardEvent): boolean {
    if (this.state.phase !== "ready" || isTypingTarget(e.target)) return false;
    const binding = eventToBinding(e);
    if (!binding) return false;
    const lookup = buildLookup(resolveBindings(this.state.prefs.keymap));
    const action = lookup.get(binding);
    if (!action) return false;
    this.runAction(action);
    return true;
  }

  /**
   * Diagnóstico **não sensível** para o pacote de aceitação: versões, transporte do preview,
   * contadores/latências (ms) e tamanho do projeto. Sem caminhos, nomes de arquivo nem conteúdo.
   */
  diagnostics(): Record<string, unknown> {
    const s = this.state;
    const seq = s.active ? s.model.sequences[s.active] : undefined;
    return {
      generated: new Date().toISOString(),
      app: {
        language: s.prefs.language,
        preview: s.prefs.preview,
        mediaAvailable: s.engine?.mediaAvailable,
      },
      previewTransport: this.frames.kind,
      project: {
        sequences: Object.keys(s.model.sequences).length,
        assets: Object.keys(s.model.assets).length,
        activeSequence: seq
          ? {
              clips: seq.clip_count,
              durationTicks: seq.duration,
              width: seq.width,
              height: seq.height,
            }
          : null,
        revision: s.model.revision,
      },
      perfMs: this.perf.summary(),
      environment: {
        userAgent: typeof navigator === "undefined" ? "" : navigator.userAgent,
        devicePixelRatio: typeof window === "undefined" ? 1 : window.devicePixelRatio,
        viewport:
          typeof window === "undefined" ? null : { w: window.innerWidth, h: window.innerHeight },
      },
    };
  }

  // -------------------------------------------------------------------------------- prefs

  setPrefs(patch: Partial<Prefs>): void {
    this.store.set((s) => ({ prefs: { ...s.prefs, ...patch } }));
    this.schedulePrefs();
  }

  setPanels(patch: Partial<Prefs["panels"]>): void {
    this.store.set((s) => ({ prefs: { ...s.prefs, panels: { ...s.prefs.panels, ...patch } } }));
    this.schedulePrefs();
  }

  setPreview(patch: Partial<Prefs["preview"]>): void {
    this.store.set((s) => ({ prefs: { ...s.prefs, preview: { ...s.prefs.preview, ...patch } } }));
    this.schedulePrefs();
  }

  setTrackHeight(track: string, h: number): void {
    this.store.set((s) => ({
      prefs: {
        ...s.prefs,
        timeline: {
          ...s.prefs.timeline,
          trackHeights: { ...s.prefs.timeline.trackHeights, [track]: h },
        },
      },
    }));
    this.schedulePrefs();
  }

  setLanguage(language: Prefs["language"]): void {
    this.t = createTranslator(language);
    this.setPrefs({ language });
  }

  resetPrefs(): void {
    this.store.set({ prefs: structuredClone(DEFAULT_PREFS) });
    this.schedulePrefs();
  }

  private schedulePrefs(): void {
    if (this.prefsTimer) clearTimeout(this.prefsTimer);
    this.prefsTimer = setTimeout(() => {
      this.flushPrefs();
    }, 300);
  }

  flushPrefs(): void {
    if (this.prefsTimer) {
      clearTimeout(this.prefsTimer);
      this.prefsTimer = null;
    }
    const ok = savePrefs(this.storage, this.state.prefs);
    if (!ok && this.storage) this.toast("error", this.t("settings.unsaved"));
  }

  // --------------------------------------------------------------------- toasts e erros

  toast(tone: ToastItem["tone"], title: string, detail?: string): void {
    const id = ++this.toastId;
    this.store.set((s) => ({
      toasts: [...s.toasts, { id, tone, title, detail }].slice(-MAX_TOASTS),
    }));
    const h = setTimeout(
      () => {
        this.dismissToast(id);
      },
      tone === "error" ? 10_000 : 4_500,
    );
    this.toastTimers.set(id, h);
  }

  dismissToast(id: number): void {
    const h = this.toastTimers.get(id);
    if (h) clearTimeout(h);
    this.toastTimers.delete(id);
    this.store.set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) }));
  }

  reportError(e: unknown): void {
    if (e instanceof ApiError) {
      const body: ApiErrorBody = {
        code: e.code,
        message: e.message,
        ...(e.details ? { details: e.details } : {}),
      };
      const text = describeError(this.t, e.code, e.message);
      this.store.set({ lastError: { message: text, body } });
      this.toast("error", text, e.message !== text ? e.message : undefined);
    } else {
      const msg = e instanceof Error ? e.message : String(e);
      this.store.set({
        lastError: { message: this.t("err.generic"), body: { code: "CLIENT_ERROR", message: msg } },
      });
      this.toast("error", this.t("err.generic"), msg);
    }
  }

  text(key: MessageKey, params?: Record<string, string | number>): string {
    return this.t(key, params);
  }
}
