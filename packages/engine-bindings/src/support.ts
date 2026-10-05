/**
 * Contrato tipado de `support.*` / `update.*` (Fase 6, Track C). Preferências de privacidade, diagnóstico e
 * verificação de atualização. Nada aqui toca o documento; o engine decide o que existe (método desconhecido
 * devolve `UNKNOWN_METHOD`, e a UI simplesmente não mostra a seção).
 */
import type { EditorTransport } from "./editor";

export interface SupportStatus {
  version: string;
  build: {
    display: string;
    os: string;
    arch: string;
    dev_build: boolean;
    build_id: string | null;
  };
  crash: {
    /** Opt-in explícito; padrão `false`. */
    opt_in: boolean;
    decided: boolean;
    status: {
      opt_in: boolean;
      sink_configured: boolean;
      local_records: number;
      pending_upload: number;
    };
  };
  update_channel: "stable" | "beta";
  onboarding_dismissed: boolean;
}

export interface DiagnosticEntry {
  path: string;
  bytes: number;
  description: string;
}

export interface DiagnosticPreview {
  entries: DiagnosticEntry[];
  total_bytes: number;
  never_included: string[];
}

export interface DiagnosticResult {
  path: string;
  bytes: number;
  entries: DiagnosticEntry[];
}

export type UpdateCheck =
  | {
      state: "not_configured";
      reason: "no_trusted_keys" | "no_endpoint";
      channel: string;
      current: string;
    }
  | { state: "up_to_date"; channel: string; current: string }
  | {
      state: "available";
      version: string;
      notes: string;
      signature: "verified";
      channel: string;
      current: string;
    }
  | {
      state: "requires_intermediate";
      version: string;
      min_version: string;
      channel: string;
      current: string;
    }
  | { state: "rejected" | "invalid"; reason: string; channel: string; current: string };

export class SupportClient {
  constructor(private readonly transport: EditorTransport) {}

  private json<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    return this.transport.call(method, params) as Promise<T>;
  }

  status() {
    return this.json<SupportStatus>("support.status");
  }
  setCrashReporting(enabled: boolean) {
    return this.json<SupportStatus>("support.crash.set", { enabled });
  }
  setChannel(channel: "stable" | "beta") {
    return this.json<SupportStatus>("support.channel.set", { channel });
  }
  dismissOnboarding() {
    return this.json<SupportStatus>("support.onboarding.dismiss");
  }
  diagnosticPreview() {
    return this.json<DiagnosticPreview>("support.diagnostic.preview");
  }
  createDiagnostic() {
    return this.json<DiagnosticResult>("support.diagnostic.create");
  }
  checkUpdates() {
    return this.json<UpdateCheck>("update.check");
  }
}
