/**
 * Atalhos configuráveis. Binding = texto normalizado `Ctrl+Shift+Z` / `Space` / `ArrowLeft`
 * (⌘ conta como Ctrl). Há um preset padrão (`DEFAULT_BINDINGS`), edição, detecção de conflito e
 * reset. Os atalhos disparam **ações** (ids); o que cada ação faz mora no controlador do editor.
 */

export type ActionId =
  | "playPause"
  | "shuttleBack"
  | "shuttlePause"
  | "shuttleForward"
  | "stepBack"
  | "stepForward"
  | "step10Back"
  | "step10Forward"
  | "goStart"
  | "goEnd"
  | "split"
  | "delete"
  | "rippleDelete"
  | "trimLeft"
  | "trimRight"
  | "undo"
  | "redo"
  | "copy"
  | "paste"
  | "duplicate"
  | "group"
  | "ungroup"
  | "toggleSnapping"
  | "zoomIn"
  | "zoomOut"
  | "zoomFit"
  | "selectAll"
  | "deselect"
  | "newSequence"
  | "addMarker"
  | "export"
  | "toggleLeftPanel"
  | "toggleRightPanel";

export const DEFAULT_BINDINGS: Record<ActionId, string[]> = {
  playPause: ["Space"],
  shuttleBack: ["J"],
  shuttlePause: ["K"],
  shuttleForward: ["L"],
  stepBack: ["ArrowLeft"],
  stepForward: ["ArrowRight"],
  step10Back: ["Shift+ArrowLeft"],
  step10Forward: ["Shift+ArrowRight"],
  goStart: ["Home"],
  goEnd: ["End"],
  split: ["Ctrl+B", "S"],
  delete: ["Delete", "Backspace"],
  rippleDelete: ["Shift+Delete", "Shift+Backspace"],
  trimLeft: ["Q"],
  trimRight: ["W"],
  undo: ["Ctrl+Z"],
  redo: ["Ctrl+Shift+Z", "Ctrl+Y"],
  copy: ["Ctrl+C"],
  paste: ["Ctrl+V"],
  duplicate: ["Ctrl+D"],
  group: ["Ctrl+G"],
  ungroup: ["Ctrl+Shift+G"],
  toggleSnapping: ["N"],
  zoomIn: ["=", "+"],
  zoomOut: ["-"],
  zoomFit: ["Shift+Z"],
  selectAll: ["Ctrl+A"],
  deselect: ["Escape"],
  newSequence: ["Ctrl+N"],
  addMarker: ["M"],
  export: ["Ctrl+E"],
  toggleLeftPanel: ["Ctrl+1"],
  toggleRightPanel: ["Ctrl+2"],
};

export const ACTION_IDS = Object.keys(DEFAULT_BINDINGS) as ActionId[];

export type Bindings = Record<ActionId, string[]>;

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph"]);

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** Normaliza o nome da tecla (letras em maiúsculas; espaço = `Space`). */
function keyName(key: string): string {
  if (key === " ") return "Space";
  if (key.length === 1) return key.toUpperCase();
  return key;
}

/** Evento de teclado → binding normalizado, ou `null` se for só um modificador. */
export function eventToBinding(e: KeyLike): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  const name = keyName(e.key);
  // `+` e `=` exigem Shift em muitos layouts: Shift só entra se não for o próprio caractere
  const shiftIsPartOfChar = name.length === 1 && !/[A-Z0-9]/.test(name);
  if (e.shiftKey && !shiftIsPartOfChar) parts.push("Shift");
  parts.push(name);
  return parts.join("+");
}

/** Mescla o preset com as customizações (`custom` substitui as teclas da ação). */
export function resolveBindings(custom: Record<string, string[]>): Bindings {
  const out = {} as Bindings;
  for (const id of ACTION_IDS) out[id] = custom[id] ?? DEFAULT_BINDINGS[id];
  return out;
}

/** Binding → ação (primeira ação que declara a tecla vence; conflitos são detectáveis à parte). */
export function buildLookup(b: Bindings): Map<string, ActionId> {
  const m = new Map<string, ActionId>();
  for (const id of ACTION_IDS) {
    for (const k of b[id]) if (!m.has(k)) m.set(k, id);
  }
  return m;
}

export interface Conflict {
  binding: string;
  actions: ActionId[];
}

/** Teclas atribuídas a mais de uma ação. */
export function findConflicts(b: Bindings): Conflict[] {
  const by = new Map<string, ActionId[]>();
  for (const id of ACTION_IDS) {
    for (const k of new Set(b[id])) by.set(k, [...(by.get(k) ?? []), id]);
  }
  return [...by.entries()]
    .filter(([, a]) => a.length > 1)
    .map(([binding, actions]) => ({ binding, actions }));
}

/** Define as teclas de `action`; devolve conflitos introduzidos (a UI decide se aceita). */
export function rebind(
  custom: Record<string, string[]>,
  action: ActionId,
  keys: string[],
): { custom: Record<string, string[]>; conflicts: Conflict[] } {
  const next = { ...custom, [action]: keys };
  const conflicts = findConflicts(resolveBindings(next)).filter((c) => c.actions.includes(action));
  return { custom: next, conflicts };
}

/** Remove as customizações (uma ação ou tudo). */
export function resetBindings(
  custom: Record<string, string[]>,
  action?: ActionId,
): Record<string, string[]> {
  if (!action) return {};
  const rest = { ...custom };
  Reflect.deleteProperty(rest, action);
  return rest;
}

/** Texto amigável do binding (`Ctrl+B` → `Ctrl+B`; `ArrowLeft` → `←`). */
export function formatBinding(b: string): string {
  return b
    .replace("ArrowLeft", "←")
    .replace("ArrowRight", "→")
    .replace("ArrowUp", "↑")
    .replace("ArrowDown", "↓");
}

/** Atalhos que não devem disparar enquanto se digita num campo. */
export function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  if (t.isContentEditable) return true;
  const tag = t.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = (t as HTMLInputElement).type;
    return !["button", "checkbox", "radio", "range"].includes(type);
  }
  return false;
}
