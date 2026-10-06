/**
 * Nome exibido de cada track. A timeline é livre: o usuário vê "Vídeo 1", "Vídeo 2", "Áudio 1"…
 * (numeradas de baixo para cima, como na pilha de composição), a menos que tenha dado um nome
 * próprio. Os nomes de fábrica das versões anteriores (Main, Overlay, Voice…) contam como "sem
 * nome" — projetos antigos abrem iguais, só com rótulos mais simples.
 */
export interface TrackLike {
  id: string;
  kind: "visual" | "audio";
  name?: string | null | undefined;
  role?: unknown;
}

const LEGACY_NAMES = new Set(["main", "overlay", "text", "voice", "music", "sfx", "captions", ""]);

export function trackLabels(
  /** Na ordem do documento (a primeira é a de baixo da pilha). */
  tracks: readonly TrackLike[],
  words: { video: string; audio: string; captions: string },
): Map<string, string> {
  const out = new Map<string, string>();
  const n = { visual: 0, audio: 0 };
  for (const tr of tracks) {
    const custom = (tr.name ?? "").trim();
    if (custom !== "" && !LEGACY_NAMES.has(custom.toLowerCase())) {
      out.set(tr.id, custom);
      n[tr.kind] += 1;
      continue;
    }
    if (tr.role === "captions") {
      out.set(tr.id, words.captions);
      continue;
    }
    n[tr.kind] += 1;
    out.set(tr.id, `${tr.kind === "audio" ? words.audio : words.video} ${String(n[tr.kind])}`);
  }
  return out;
}
