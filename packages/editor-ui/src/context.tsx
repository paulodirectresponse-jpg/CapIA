import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { EditorController, UiState } from "./store/controller";
import { useStoreSelector } from "./store/createStore";
import type { AiState } from "./store/aiController";

const Ctx = createContext<EditorController | null>(null);

export function ControllerProvider({
  controller,
  children,
}: {
  controller: EditorController;
  children: ReactNode;
}) {
  return <Ctx.Provider value={controller}>{children}</Ctx.Provider>;
}

export function useController(): EditorController {
  const c = useContext(Ctx);
  if (!c) throw new Error("ControllerProvider is missing");
  return c;
}

/** Fatia do estado da UI (re-renderiza só quando a fatia muda, comparação rasa). */
export function useUi<S>(selector: (s: UiState) => S): S {
  const c = useController();
  return useStoreSelector(c.store, selector);
}

/** Fatia do estado da IA. */
export function useAi<S>(selector: (s: AiState) => S): S {
  const c = useController();
  return useStoreSelector(c.ai.store, selector);
}

/** Sequence ativa (conteúdo) e seu resumo. */
export function useActiveSequence() {
  return useUi((s) => ({
    id: s.active,
    seq: s.active ? (s.model.models[s.active] ?? null) : null,
    summary: s.active ? (s.model.sequences[s.active] ?? null) : null,
  }));
}

export function useMemoValue<T>(factory: () => T, deps: unknown[]): T {
  // eslint-disable-next-line react-hooks/exhaustive-deps, react-hooks/use-memo -- deps passadas pelo chamador
  return useMemo(factory, deps);
}
