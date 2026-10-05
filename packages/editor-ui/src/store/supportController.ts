/**
 * Controlador de privacidade, diagnóstico e atualização (Fase 6). Única peça da UI que fala com
 * `support.*`/`update.*`. Regras:
 *  - crash report é opt-in e começa DESLIGADO (o estado só reflete o que o engine devolve);
 *  - o diagnóstico é gerado pelo engine, sempre com preview antes; a UI nunca monta o conteúdo;
 *  - sem o serviço (navegador/devserver) `available` fica falso e a seção não aparece: o editor funciona igual.
 */
import {
  ApiError,
  type DiagnosticPreview,
  type DiagnosticResult,
  type SupportClient,
  type SupportStatus,
  type UpdateCheck,
} from "@capia/engine-bindings";
import { createStore, type Store } from "./createStore";

export interface SupportState {
  /** `false` = o engine não tem o serviço (ou ainda não carregou). */
  available: boolean;
  status: SupportStatus | null;
  preview: DiagnosticPreview | null;
  diagnostic: DiagnosticResult | null;
  update: UpdateCheck | null;
  busy: boolean;
  error: string | null;
}

export class SupportController {
  readonly store: Store<SupportState>;

  constructor(private readonly client: SupportClient) {
    this.store = createStore<SupportState>({
      available: false,
      status: null,
      preview: null,
      diagnostic: null,
      update: null,
      busy: false,
      error: null,
    });
  }

  get state(): SupportState {
    return this.store.get();
  }

  private fail(e: unknown): void {
    const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
    this.store.set({ busy: false, error: msg });
  }

  /** Carrega o estado; um engine sem o serviço deixa `available = false` (sem erro visível). */
  async refresh(): Promise<void> {
    try {
      const status = await this.client.status();
      this.store.set({ available: true, status, error: null });
    } catch {
      this.store.set({ available: false, status: null });
    }
  }

  async setCrashReporting(enabled: boolean): Promise<void> {
    this.store.set({ busy: true, error: null });
    try {
      const status = await this.client.setCrashReporting(enabled);
      this.store.set({ status, busy: false });
    } catch (e) {
      this.fail(e);
    }
  }

  async setChannel(channel: "stable" | "beta"): Promise<void> {
    try {
      const status = await this.client.setChannel(channel);
      this.store.set({ status, update: null });
    } catch (e) {
      this.fail(e);
    }
  }

  async dismissOnboarding(): Promise<void> {
    // some imediatamente; a persistência é melhor esforço
    const cur = this.store.get().status;
    if (cur) this.store.set({ status: { ...cur, onboarding_dismissed: true } });
    try {
      this.store.set({ status: await this.client.dismissOnboarding() });
    } catch {
      /* sem persistência: o aviso só volta na próxima abertura */
    }
  }

  async loadPreview(): Promise<void> {
    this.store.set({ busy: true, error: null, diagnostic: null });
    try {
      const preview = await this.client.diagnosticPreview();
      this.store.set({ preview, busy: false });
    } catch (e) {
      this.fail(e);
    }
  }

  /** Só depois do preview carregado (a UI exige que o usuário o tenha visto). */
  async createDiagnostic(): Promise<void> {
    if (!this.store.get().preview) return;
    this.store.set({ busy: true, error: null });
    try {
      const diagnostic = await this.client.createDiagnostic();
      this.store.set({ diagnostic, busy: false });
    } catch (e) {
      this.fail(e);
    }
  }

  async checkUpdates(): Promise<void> {
    this.store.set({ busy: true, error: null });
    try {
      const update = await this.client.checkUpdates();
      this.store.set({ update, busy: false });
    } catch (e) {
      this.fail(e);
    }
  }
}
