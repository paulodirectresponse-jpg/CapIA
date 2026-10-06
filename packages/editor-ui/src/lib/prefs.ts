/**
 * Preferências **da interface** (largura de painéis, idioma, atalhos, qualidade do preview…).
 * Ficam separadas dos dados do projeto (docs/PHASE3 §43). Carregamento à prova de corrupção: JSON
 * inválido ou campos fora do esquema voltam ao padrão **campo a campo** — nunca derrubam o app.
 */

export type PreviewQuality = "auto" | "540" | "720";
export type Language = "en" | "pt-BR";

export interface Prefs {
  version: 1;
  language: Language;
  panels: {
    leftWidth: number;
    rightWidth: number;
    timelineHeight: number;
    leftCollapsed: boolean;
    rightCollapsed: boolean;
  };
  keymap: Record<string, string[]>;
  preview: { quality: PreviewQuality; proxy: boolean; safeAreas: boolean; audio: boolean };
  timeline: { pxPerSecond: number; trackHeights: Record<string, number>; snapping: boolean };
  /** Projetos abertos recentemente (mais novo primeiro). Preferência da UI: não vai no projeto. */
  recent: RecentProject[];
}

export interface RecentProject {
  path: string;
  openedAt: number;
}

export const MAX_RECENT = 8;

/** Adiciona/promove `path` no topo, sem duplicar, respeitando o limite. */
export function pushRecent(
  list: readonly RecentProject[],
  path: string,
  now: number,
): RecentProject[] {
  const p = path.trim();
  if (p === "") return [...list];
  return [{ path: p, openedAt: now }, ...list.filter((r) => r.path !== p)].slice(0, MAX_RECENT);
}

/** Primeira execução em português quando o sistema está em português. */
export function systemLanguage(): Language {
  try {
    return typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("pt")
      ? "pt-BR"
      : "en";
  } catch {
    return "en";
  }
}

export const DEFAULT_PREFS: Prefs = {
  version: 1,
  language: systemLanguage(),
  panels: {
    leftWidth: 300,
    rightWidth: 300,
    timelineHeight: 400,
    leftCollapsed: false,
    rightCollapsed: false,
  },
  keymap: {},
  preview: { quality: "auto", proxy: false, safeAreas: false, audio: true },
  timeline: { pxPerSecond: 80, trackHeights: {}, snapping: true },
  recent: [],
};

export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const KEY = "capia.prefs.v1";

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const num = (v: unknown, d: number, min: number, max: number): number =>
  typeof v === "number" && Number.isFinite(v) ? Math.min(max, Math.max(min, v)) : d;

const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : d);

/** Valida/normaliza um valor arbitrário (JSON de disco) para `Prefs`. */
export function sanitizePrefs(raw: unknown): Prefs {
  const d = DEFAULT_PREFS;
  if (!isRecord(raw)) return structuredClone(d);
  const panels = isRecord(raw.panels) ? raw.panels : {};
  const preview = isRecord(raw.preview) ? raw.preview : {};
  const timeline = isRecord(raw.timeline) ? raw.timeline : {};
  const keymap: Record<string, string[]> = {};
  if (isRecord(raw.keymap)) {
    for (const [action, v] of Object.entries(raw.keymap)) {
      if (
        Array.isArray(v) &&
        v.every((b) => typeof b === "string" && b.length > 0 && b.length < 40)
      ) {
        keymap[action] = v as string[];
      }
    }
  }
  const trackHeights: Record<string, number> = {};
  if (isRecord(timeline.trackHeights)) {
    for (const [id, h] of Object.entries(timeline.trackHeights)) {
      if (typeof h === "number" && Number.isFinite(h))
        trackHeights[id] = Math.min(240, Math.max(24, h));
    }
  }
  const recent: RecentProject[] = Array.isArray(raw.recent)
    ? raw.recent
        .filter(
          (r): r is RecentProject =>
            isRecord(r) &&
            typeof r.path === "string" &&
            r.path.length > 0 &&
            r.path.length < 1024 &&
            typeof r.openedAt === "number" &&
            Number.isFinite(r.openedAt),
        )
        .slice(0, MAX_RECENT)
    : [];
  const quality = preview.quality;
  return {
    version: 1,
    language: raw.language === "pt-BR" || raw.language === "en" ? raw.language : d.language,
    panels: {
      leftWidth: num(panels.leftWidth, d.panels.leftWidth, 180, 640),
      rightWidth: num(panels.rightWidth, d.panels.rightWidth, 220, 640),
      timelineHeight: num(panels.timelineHeight, d.panels.timelineHeight, 160, 900),
      leftCollapsed: bool(panels.leftCollapsed, d.panels.leftCollapsed),
      rightCollapsed: bool(panels.rightCollapsed, d.panels.rightCollapsed),
    },
    keymap,
    preview: {
      quality:
        quality === "540" || quality === "720" || quality === "auto" ? quality : d.preview.quality,
      proxy: bool(preview.proxy, d.preview.proxy),
      safeAreas: bool(preview.safeAreas, d.preview.safeAreas),
      audio: bool(preview.audio, d.preview.audio),
    },
    timeline: {
      pxPerSecond: num(timeline.pxPerSecond, d.timeline.pxPerSecond, 2, 4000),
      trackHeights,
      snapping: bool(timeline.snapping, d.timeline.snapping),
    },
    recent,
  };
}

/** Lê as preferências; qualquer falha de leitura/parse devolve o padrão (e sinaliza `recovered`). */
export function loadPrefs(storage: KeyValueStorage | null): { prefs: Prefs; recovered: boolean } {
  if (!storage) return { prefs: structuredClone(DEFAULT_PREFS), recovered: false };
  let text: string | null;
  try {
    text = storage.getItem(KEY);
  } catch {
    return { prefs: structuredClone(DEFAULT_PREFS), recovered: true };
  }
  if (text === null) return { prefs: structuredClone(DEFAULT_PREFS), recovered: false };
  try {
    return { prefs: sanitizePrefs(JSON.parse(text) as unknown), recovered: false };
  } catch {
    return { prefs: structuredClone(DEFAULT_PREFS), recovered: true };
  }
}

export function savePrefs(storage: KeyValueStorage | null, prefs: Prefs): boolean {
  if (!storage) return false;
  try {
    storage.setItem(KEY, JSON.stringify(prefs));
    return true;
  } catch {
    return false;
  }
}

export function browserStorage(): KeyValueStorage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}
