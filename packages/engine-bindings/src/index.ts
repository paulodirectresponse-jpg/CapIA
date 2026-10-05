/**
 * Contrato entre a UI e o engine Rust (docs/ARCHITECTURE.md §5, §7).
 *
 * Regras: este pacote não conhece Tauri, React nem UI. O transporte (Tauri IPC hoje; WASM/REST/MCP
 * depois) é injetado. Os tipos são escritos à mão no scaffold; a fixture compartilhada
 * (`fixtures/engine_info.json`) é testada dos dois lados e vira geração de tipos na Fase 2.
 */

/** Espelha `capia_project::EngineInfo` (JSON em snake_case). */
export interface EngineInfo {
  name: string;
  version: string;
  engine_api_version: number;
  document_schema_version: number;
  command_schema_version: number;
  /** Cabe num `Number` do JS: o teto de timeline (24 h) é inteiro seguro (ADR-007, S4). */
  ticks_per_second: number;
}

/** Nomes dos comandos IPC expostos pelos adaptadores. */
export const ENGINE_COMMANDS = { getEngineInfo: "get_engine_info" } as const;

/** Transporte injetado pelo adaptador (shell Tauri, testes, futuro WASM). */
export interface EngineTransport {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
}

/** Cliente do engine consumido pela UI. */
export interface EngineClient {
  getEngineInfo(): Promise<EngineInfo>;
}

export class EngineContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EngineContractError";
  }
}

const INFO_NUMBER_FIELDS = [
  "engine_api_version",
  "document_schema_version",
  "command_schema_version",
  "ticks_per_second",
] as const;

/** Valida em runtime o payload vindo do engine (não confiamos no tipo estático na fronteira). */
export function parseEngineInfo(value: unknown): EngineInfo {
  if (typeof value !== "object" || value === null) {
    throw new EngineContractError("EngineInfo: expected an object");
  }
  const record = value as Record<string, unknown>;
  const { name, version } = record;
  if (typeof name !== "string" || typeof version !== "string") {
    throw new EngineContractError("EngineInfo: name/version must be strings");
  }
  const numbers: Record<string, number> = {};
  for (const field of INFO_NUMBER_FIELDS) {
    const n = record[field];
    if (typeof n !== "number" || !Number.isSafeInteger(n)) {
      throw new EngineContractError(`EngineInfo: ${field} must be a safe integer`);
    }
    numbers[field] = n;
  }
  return {
    name,
    version,
    engine_api_version: numbers.engine_api_version ?? 0,
    document_schema_version: numbers.document_schema_version ?? 0,
    command_schema_version: numbers.command_schema_version ?? 0,
    ticks_per_second: numbers.ticks_per_second ?? 0,
  };
}

export function createEngineClient(transport: EngineTransport): EngineClient {
  return {
    async getEngineInfo() {
      return parseEngineInfo(await transport.invoke(ENGINE_COMMANDS.getEngineInfo));
    },
  };
}
export * from "./ai";
export * from "./editor";
export * from "./readmodel";
