import { useEffect, useState } from "react";
import type { EngineClient, EngineInfo } from "@capia/engine-bindings";

type EngineState =
  | { status: "loading" }
  | { status: "ready"; info: EngineInfo }
  | { status: "unavailable"; reason: string };

/** Janela mínima do scaffold: identifica o CapIA e mostra o engine por trás do cliente injetado. */
export function App({ client }: { client: EngineClient }) {
  const [state, setState] = useState<EngineState>({ status: "loading" });

  useEffect(() => {
    let active = true;
    client.getEngineInfo().then(
      (info) => {
        if (active) setState({ status: "ready", info });
      },
      (error: unknown) => {
        if (active) {
          setState({
            status: "unavailable",
            reason: error instanceof Error ? error.message : "unknown error",
          });
        }
      },
    );
    return () => {
      active = false;
    };
  }, [client]);

  return (
    <main className="capia-shell">
      <h1>CapIA</h1>
      <p>Editor de vídeo AI-first — scaffold da Fase 1 (sem editor visual).</p>
      <p role="status" data-testid="engine-status">
        {state.status === "loading" && "Conectando ao engine…"}
        {state.status === "ready" &&
          `Engine ${state.info.name} v${state.info.version} · API ${String(state.info.engine_api_version)} · schema documento ${String(state.info.document_schema_version)} / comandos ${String(state.info.command_schema_version)}`}
        {state.status === "unavailable" && `Engine indisponível: ${state.reason}`}
      </p>
    </main>
  );
}
