/** Cores do canvas, lidas dos tokens CSS do design system (um só lugar para o visual). */
export interface Palette {
  bg: string;
  rowA: string;
  rowB: string;
  rowLine: string;
  ruler: string;
  rulerText: string;
  rulerTick: string;
  text: string;
  textMuted: string;
  accent: string;
  accentSoft: string;
  danger: string;
  playhead: string;
  snap: string;
  role: Record<string, string>;
  font: string;
}

const FALLBACK: Palette = {
  bg: "#101114",
  rowA: "#17181c",
  rowB: "#1b1d22",
  rowLine: "#2d3037",
  ruler: "#1e2025",
  rulerText: "#a3a9b6",
  rulerTick: "#424651",
  text: "#e8eaef",
  textMuted: "#a3a9b6",
  accent: "#5b8cff",
  accentSoft: "rgba(91,140,255,0.18)",
  danger: "#ff6b6b",
  playhead: "#ff5d5d",
  snap: "#ffd166",
  role: {
    main: "#4f7cff",
    overlay: "#9a6bff",
    text: "#f5b74a",
    captions: "#ffd166",
    voice: "#4fd18b",
    music: "#3fb7c9",
    sfx: "#ef7ab0",
    nested: "#c8895a",
    custom: "#7b8394",
  },
  font: '12px "Segoe UI", system-ui, sans-serif',
};

/** Lê `--var` do elemento; cai no padrão se ausente (ex.: testes sem CSS). */
export function readPalette(el: Element | null): Palette {
  if (!el || typeof getComputedStyle === "undefined") return FALLBACK;
  const cs = getComputedStyle(el);
  const v = (name: string, d: string) => cs.getPropertyValue(name).trim() || d;
  const f = FALLBACK;
  return {
    bg: v("--surface-0", f.bg),
    rowA: v("--surface-1", f.rowA),
    rowB: v("--surface-2", f.rowB),
    rowLine: v("--border", f.rowLine),
    ruler: v("--surface-2", f.ruler),
    rulerText: v("--text-muted", f.rulerText),
    rulerTick: v("--border-strong", f.rulerTick),
    text: v("--text", f.text),
    textMuted: v("--text-muted", f.textMuted),
    accent: v("--accent", f.accent),
    accentSoft: v("--accent-soft", f.accentSoft),
    danger: v("--danger", f.danger),
    playhead: f.playhead,
    snap: v("--warning", f.snap),
    role: {
      main: v("--role-main", f.role.main ?? "#4f7cff"),
      overlay: v("--role-overlay", f.role.overlay ?? "#9a6bff"),
      text: v("--role-text", f.role.text ?? "#f5b74a"),
      captions: v("--role-captions", f.role.captions ?? "#ffd166"),
      voice: v("--role-voice", f.role.voice ?? "#4fd18b"),
      music: v("--role-music", f.role.music ?? "#3fb7c9"),
      sfx: v("--role-sfx", f.role.sfx ?? "#ef7ab0"),
      nested: v("--role-nested", f.role.nested ?? "#c8895a"),
      custom: f.role.custom ?? "#7b8394",
    },
    font: `12px ${cs.fontFamily || "system-ui, sans-serif"}`,
  };
}

export function roleColor(p: Palette, role: unknown): string {
  if (typeof role === "string") return p.role[role] ?? p.role.custom ?? "#7b8394";
  return p.role.custom ?? "#7b8394";
}
