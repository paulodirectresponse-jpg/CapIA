import { useEffect, useState } from "react";
import type { EditorClient } from "@capia/engine-bindings";
import type { TimelineCore } from "@capia/ui-timeline";
import { EditorShell } from "./components/EditorShell";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Welcome } from "./components/Welcome";
import { ControllerProvider, useUi } from "./context";
import { I18nProvider } from "./i18n";
import type { KeyValueStorage } from "./lib/prefs";
import type { PlatformServices } from "./platform";
import type { FrameSource } from "./preview/frames";
import { EditorController } from "./store/controller";
import "@capia/ui-kit/styles.css";
import "./app.css";

export interface AppProps {
  client: EditorClient;
  platform?: PlatformServices;
  frames?: FrameSource;
  /** Carrega o núcleo WASM da timeline (ghost/snap). Sem ele o editor ainda funciona. */
  loadCore?: () => Promise<TimelineCore | null>;
  storage?: KeyValueStorage | null;
  pollMs?: number;
}

function Root() {
  const phase = useUi((s) => s.phase);
  const lang = useUi((s) => s.prefs.language);
  return (
    <I18nProvider lang={lang}>
      <ErrorBoundary scope="app">
        {phase === "ready" ? (
          <EditorShell />
        ) : phase === "boot" ? (
          <div className="ed-welcome" role="status" aria-busy="true" />
        ) : (
          <Welcome />
        )}
      </ErrorBoundary>
    </I18nProvider>
  );
}

/**
 * Raiz do editor. O controlador é criado uma vez por montagem; o `dispose` é adiado para o fim
 * do tick para sobreviver ao ciclo mount→unmount→mount do StrictMode sem perder o estado.
 */
export function App({ client, platform, frames, loadCore, storage, pollMs }: AppProps) {
  const [controller] = useState(
    () =>
      new EditorController(client, {
        ...(platform ? { platform } : {}),
        ...(frames ? { frames } : {}),
        ...(loadCore ? { loadCore } : {}),
        ...(storage !== undefined ? { storage } : {}),
        ...(pollMs !== undefined ? { pollMs } : {}),
      }),
  );
  useEffect(() => {
    const pending = (controller as unknown as { __pendingDispose?: ReturnType<typeof setTimeout> })
      .__pendingDispose;
    if (pending) clearTimeout(pending);
    if (controller.state.phase === "boot") void controller.boot();
    return () => {
      (
        controller as unknown as { __pendingDispose?: ReturnType<typeof setTimeout> }
      ).__pendingDispose = setTimeout(() => {
        controller.dispose();
      }, 0);
    };
  }, [controller]);
  // preferências pendentes (debounce) são gravadas ao sair/recarregar/esconder a janela
  useEffect(() => {
    const flush = () => {
      controller.flushPrefs();
    };
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", flush);
    };
  }, [controller]);
  return (
    <ControllerProvider controller={controller}>
      <Root />
    </ControllerProvider>
  );
}
