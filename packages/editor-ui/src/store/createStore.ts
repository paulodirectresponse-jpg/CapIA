import { useRef, useSyncExternalStore } from "react";

/** Store mínimo (estado imutável + assinaturas). Sem dependências externas. */
export interface Store<T> {
  get(): T;
  set(patch: Partial<T> | ((s: T) => Partial<T>)): void;
  subscribe(fn: () => void): () => void;
}

export function createStore<T extends object>(initial: T): Store<T> {
  let state = initial;
  const subs = new Set<() => void>();
  return {
    get: () => state,
    set(patch) {
      const p = typeof patch === "function" ? patch(state) : patch;
      const next = { ...state, ...p };
      // nada mudou (referências iguais)? não notifica
      if ((Object.keys(p) as (keyof T)[]).every((k) => Object.is(state[k], next[k]))) return;
      state = next;
      for (const fn of [...subs]) fn();
    },
    subscribe(fn) {
      subs.add(fn);
      return () => {
        subs.delete(fn);
      };
    },
  };
}

/** Igualdade rasa de objetos/arrays para seletores que devolvem estruturas novas. */
export function shallowEqual(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  const ka = Object.keys(a);
  const kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  return ka.every((k) =>
    Object.is((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]),
  );
}

/** Assina uma fatia do store; re-renderiza só quando a fatia muda (comparação rasa). */
export function useStoreSelector<T extends object, S>(store: Store<T>, selector: (s: T) => S): S {
  const last = useRef<{ s: S } | null>(null);
  return useSyncExternalStore(
    (cb) => store.subscribe(cb),
    () => {
      const next = selector(store.get());
      if (last.current && shallowEqual(last.current.s, next)) return last.current.s;
      last.current = { s: next };
      return next;
    },
  );
}
