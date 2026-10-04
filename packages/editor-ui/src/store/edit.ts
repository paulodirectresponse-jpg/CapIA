/**
 * Construtores **puros** de comandos do editor: transformam a intenção do usuário (dividir, apagar,
 * soltar mídia, colar…) em comandos do Command Engine. Nada aqui altera o documento — o controlador
 * os executa e aplica o patch devolvido. Os limites finais (sobreposição, handles, travas) são
 * sempre decididos pelo engine; aqui só se escolhe a intenção mais provável.
 */
import {
  DEFAULT_TEXT_STYLE,
  TICKS_PER_SECOND,
  type AssetRow,
  type Clip,
  type CommandBody,
  type SequenceModel,
  type TextStyle,
  type Ticks,
  type Track,
} from "@capia/engine-bindings";

export const DEFAULT_IMAGE_SECONDS = 5;
export const DEFAULT_TEXT_SECONDS = 3;

export interface EditContext {
  seqId: string;
  seq: SequenceModel;
  playhead: Ticks;
  selection: string[];
  assets: Record<string, AssetRow>;
}

export const frameOf = (seq: SequenceModel): Ticks => seq.frame_ticks;

export function alignFloor(t: Ticks, frame: Ticks): Ticks {
  return Math.floor(t / frame) * frame;
}

export function alignRound(t: Ticks, frame: Ticks): Ticks {
  return Math.floor((t + frame / 2) / frame) * frame;
}

const trackById = (seq: SequenceModel, id: string): Track | undefined =>
  seq.tracks.find((t) => t.id === id);

const clipsOn = (seq: SequenceModel, track: string): Clip[] =>
  Object.values(seq.clips)
    .filter((c) => c.track === track)
    .sort((a, b) => a.start - b.start);

const end = (c: Clip): Ticks => c.start + c.duration;

/** Alvos de uma ação sobre clips: a seleção, ou o clip sob o playhead nas tracks destravadas. */
export function targetClips(ctx: EditContext): Clip[] {
  const sel = ctx.selection
    .map((id) => ctx.seq.clips[id])
    .filter((c): c is Clip => c !== undefined);
  if (sel.length > 0) return sel;
  return Object.values(ctx.seq.clips).filter(
    (c) =>
      c.start <= ctx.playhead &&
      ctx.playhead < end(c) &&
      trackById(ctx.seq, c.track)?.locked !== true,
  );
}

/** Dividir no playhead (só clips que o playhead atravessa de fato). */
export function splitCommands(ctx: EditContext): CommandBody[] {
  const frame = frameOf(ctx.seq);
  const at = alignRound(ctx.playhead, frame);
  return targetClips(ctx)
    .filter((c) => c.start < at && at < end(c))
    .map((c) => ({ type: "split_clip", clip: c.id, at }));
}

export function deleteCommands(ctx: EditContext, ripple: boolean): CommandBody[] {
  return targetClips(ctx).map((c) => ({
    type: "delete_clip",
    clip: c.id,
    ripple: ripple ? true : null,
  }));
}

/** Corta o início/fim do clip até o playhead. */
export function trimToPlayheadCommands(ctx: EditContext, edge: "in" | "out"): CommandBody[] {
  const frame = frameOf(ctx.seq);
  const at = alignRound(ctx.playhead, frame);
  return targetClips(ctx)
    .filter((c) => c.start < at && at < end(c))
    .map((c) => ({ type: "trim_clip", clip: c.id, edge, to: at }));
}

// ----------------------------------------------------------------------------- colocação

export interface NewClipSpec {
  kind: "visual" | "audio";
  duration: Ticks;
  content: CommandBody;
  name?: string;
  source_in?: Ticks;
  properties?: Record<string, unknown>;
  speed?: string;
  reversed?: boolean;
}

export interface Placed {
  commands: CommandBody[];
  /** Referências `$ref` dos clips criados (na ordem das specs). */
  refs: string[];
}

function hasRoom(
  seq: SequenceModel,
  track: Track,
  start: Ticks,
  duration: Ticks,
  ignore?: Set<string>,
): boolean {
  return clipsOn(seq, track.id).every(
    (c) => ignore?.has(c.id) === true || end(c) <= start || c.start >= start + duration,
  );
}

/** Fronteira (início de clip ou fim da track) mais próxima de `t` numa track magnética. */
export function nearestBoundary(seq: SequenceModel, track: string, t: Ticks): Ticks {
  const list = clipsOn(seq, track);
  let best = 0;
  let bestD = Math.abs(t);
  for (const c of list) {
    for (const edge of [c.start, end(c)]) {
      const d = Math.abs(t - edge);
      if (d < bestD) {
        best = edge;
        bestD = d;
      }
    }
  }
  return best;
}

let tmpCounter = 0;
const newRef = (p: string): string => `$${p}${String(++tmpCounter)}`;

export function addTrackCommand(
  seqId: string,
  kind: "visual" | "audio",
  role: string,
  opts: { name?: string; magnetic?: boolean; ref?: string } = {},
): CommandBody {
  return {
    type: "add_track",
    sequence: seqId,
    kind,
    role,
    magnetic: opts.magnetic ?? false,
    ...(opts.name ? { name: opts.name } : {}),
    ...(opts.ref ? { ref: opts.ref } : {}),
  };
}

/**
 * Resolve onde um clip novo entra e devolve os comandos (inclusive a criação de track, se preciso).
 * `preferred` = track sob o ponteiro (pode ser de outro tipo/ocupada → cai para nova track).
 */
export function placeClips(
  ctx: EditContext,
  specs: NewClipSpec[],
  start: Ticks,
  preferred: { track?: string; side?: "above" | "below" } = {},
): Placed {
  const { seq, seqId } = ctx;
  const frame = frameOf(seq);
  const commands: CommandBody[] = [];
  const refs: string[] = [];
  const newTracks = new Map<string, string>(); // chave tipo+ocupação → ref
  let cursor = alignRound(Math.max(0, start), frame);
  for (const spec of specs) {
    let trackRef: string | null = null;
    const pref = preferred.track ? trackById(seq, preferred.track) : undefined;
    let at = cursor;
    if (pref && pref.kind === spec.kind && !pref.locked && !pref.hidden) {
      if (pref.magnetic) {
        at = nearestBoundary(seq, pref.id, at);
        trackRef = pref.id;
      } else if (hasRoom(seq, pref, at, spec.duration)) {
        trackRef = pref.id;
      }
    }
    if (!trackRef) {
      // outra track livre do mesmo tipo (nunca a magnética nem travada)?
      const free =
        preferred.side === undefined
          ? [...seq.tracks]
              .filter((t) => t.kind === spec.kind && !t.locked && !t.magnetic && !t.hidden)
              .reverse()
              .find((t) => hasRoom(seq, t, at, spec.duration))
          : undefined;
      if (free) trackRef = free.id;
    }
    if (!trackRef) {
      const key = `${spec.kind}:${String(at)}`;
      let ref = newTracks.get(key);
      if (!ref) {
        ref = newRef("trk");
        newTracks.set(key, ref);
        commands.push(
          addTrackCommand(seqId, spec.kind, spec.kind === "visual" ? "overlay" : "sfx", { ref }),
        );
      }
      trackRef = ref;
    }
    const clipRef = newRef("clip");
    refs.push(clipRef);
    commands.push({
      type: "insert_clip",
      ref: clipRef,
      track: trackRef,
      start: at,
      clip: {
        name: spec.name ?? "",
        duration: spec.duration,
        content: spec.content,
        source_in: spec.source_in ?? 0,
        speed: spec.speed ?? "1",
        reversed: spec.reversed ?? false,
        properties: spec.properties ?? {},
      },
    });
    cursor = at;
  }
  return { commands, refs };
}

export type DropSpot =
  | { kind: "track"; track: string; time: Ticks }
  | { kind: "new-track"; side: "above" | "below"; time: Ticks };

/** Comandos para soltar um asset da biblioteca na timeline. */
export function dropAssetCommands(
  ctx: EditContext,
  asset: AssetRow,
  spot: DropSpot,
): CommandBody[] | { error: "no-file" | "no-duration" } {
  if (!asset.has_file) return { error: "no-file" };
  const frame = frameOf(ctx.seq);
  const isAudioOnly =
    asset.kind === "audio" || (asset.has_video === false && asset.has_audio === true);
  const isImage = asset.kind === "image";
  const kind: "visual" | "audio" = isAudioOnly ? "audio" : "visual";
  let duration: Ticks;
  if (isImage) duration = alignRound(DEFAULT_IMAGE_SECONDS * TICKS_PER_SECOND, frame);
  else if (asset.duration && asset.duration > 0)
    duration = kind === "visual" ? alignFloor(asset.duration, frame) : asset.duration;
  else return { error: "no-duration" };
  if (duration <= 0) return { error: "no-duration" };
  const content: CommandBody = isImage
    ? { type: "image", asset: asset.id }
    : {
        type: "media",
        asset: asset.id,
        has_video: !isAudioOnly,
        has_audio: asset.has_audio ?? false,
      };
  const spec: NewClipSpec = { kind, duration, content, name: asset.name };
  const preferred = spot.kind === "track" ? { track: spot.track } : { side: spot.side };
  // nova track pedida explicitamente (fora das linhas): não reaproveita livres
  if (spot.kind === "new-track") {
    const ref = newRef("trk");
    const clipRef = newRef("clip");
    return [
      addTrackCommand(ctx.seqId, kind, kind === "visual" ? "overlay" : "sfx", { ref }),
      {
        type: "insert_clip",
        ref: clipRef,
        track: ref,
        start: alignRound(Math.max(0, spot.time), frame),
        clip: {
          name: spec.name ?? "",
          duration,
          content,
          source_in: 0,
          speed: "1",
          reversed: false,
          properties: {},
        },
      },
    ];
  }
  return placeClips(ctx, [spec], spot.time, preferred).commands;
}

/** Solta uma sequence como clip nested (track visual livre/magnética, ou nova track). */
export function dropSequenceCommands(
  ctx: EditContext,
  sequence: string,
  duration: Ticks,
  spot: DropSpot,
): CommandBody[] {
  const { seq, seqId } = ctx;
  const frame = frameOf(seq);
  const len = Math.max(frame, alignFloor(duration, frame));
  const cmds: CommandBody[] = [];
  let track: string | null = null;
  let start = alignRound(Math.max(0, spot.time), frame);
  if (spot.kind === "track") {
    const t = trackById(seq, spot.track);
    if (t && t.kind === "visual" && !t.locked) {
      if (t.magnetic) {
        start = nearestBoundary(seq, t.id, start);
        track = t.id;
      } else if (hasRoom(seq, t, start, len)) track = t.id;
    }
  }
  if (!track) {
    track = newRef("trk");
    cmds.push(addTrackCommand(seqId, "visual", "overlay", { ref: track }));
  }
  cmds.push({ type: "insert_nested", ref: newRef("clip"), track, start, sequence });
  return cmds;
}

// ------------------------------------------------------------------------ texto e legendas

export type TextPreset = "title" | "caption" | "lowerThird";

export const TEXT_PRESETS: Record<TextPreset, { style: TextStyle; role: string; propY: number }> = {
  title: {
    style: { ...DEFAULT_TEXT_STYLE, size_permille: 90, weight: 700 },
    role: "text",
    propY: 0,
  },
  caption: {
    style: {
      ...DEFAULT_TEXT_STYLE,
      size_permille: 55,
      weight: 700,
      color: "#FFFFFF",
      background: "#000000B3",
    },
    role: "captions",
    propY: 340,
  },
  lowerThird: {
    style: {
      ...DEFAULT_TEXT_STYLE,
      size_permille: 55,
      align: "left",
      weight: 700,
      stroke: "#000000",
      stroke_permille: 3,
    },
    role: "text",
    propY: 280,
  },
};

/** Cria um clip de texto/legenda no playhead, na track da função (cria a track se faltar). */
export function addTextCommands(
  ctx: EditContext,
  preset: TextPreset,
  text: string,
  frameHeight: number,
): CommandBody[] {
  const p = TEXT_PRESETS[preset];
  const frame = frameOf(ctx.seq);
  const duration = alignRound(DEFAULT_TEXT_SECONDS * TICKS_PER_SECOND, frame);
  const start = alignRound(ctx.playhead, frame);
  const existing = ctx.seq.tracks.find(
    (t) =>
      t.kind === "visual" &&
      (typeof t.role === "string" ? t.role : "") === p.role &&
      !t.locked &&
      hasRoom(ctx.seq, t, start, duration),
  );
  const cmds: CommandBody[] = [];
  let track = existing?.id ?? "";
  if (!existing) {
    track = newRef("trk");
    cmds.push(
      addTrackCommand(ctx.seqId, "visual", p.role, {
        ref: track,
        name: p.role === "captions" ? "Captions" : "Text",
      }),
    );
  }
  const clipRef = newRef("clip");
  cmds.push({
    type: "insert_clip",
    ref: clipRef,
    track,
    start,
    clip: {
      name: "",
      duration,
      content: { type: "text", text, style: p.style },
      source_in: 0,
      speed: "1",
      reversed: false,
      properties: {},
    },
  });
  // legenda/terço inferior ficam na parte de baixo do quadro (posição em px do quadro de saída)
  if (p.propY !== 0) {
    cmds.push({
      type: "set_property",
      clip: clipRef,
      prop: "position_y",
      value: (p.propY / 1080) * frameHeight,
    });
  }
  return cmds;
}

// ----------------------------------------------------------------- copiar / colar / duplicar

export interface ClipboardItem {
  content: Clip["content"];
  duration: Ticks;
  source_in: Ticks;
  speed: string;
  reversed: boolean;
  name: string;
  properties: Clip["properties"];
  trackId: string;
  trackKind: "visual" | "audio";
  offset: Ticks;
}

export interface ClipboardPayload {
  version: 1;
  sourceSequence: string;
  items: ClipboardItem[];
  /** Caminhos dos assets usados (para colar em outro projeto, importando o que faltar). */
  assetPaths: Record<string, string | null>;
}

export function copyPayload(ctx: EditContext): ClipboardPayload | null {
  const clips = ctx.selection
    .map((id) => ctx.seq.clips[id])
    .filter((c): c is Clip => c !== undefined);
  if (clips.length === 0) return null;
  const min = Math.min(...clips.map((c) => c.start));
  const assetPaths: Record<string, string | null> = {};
  const items = clips.map((c) => {
    const tr = trackById(ctx.seq, c.track);
    if (c.content.type === "media" || c.content.type === "image") {
      assetPaths[c.content.asset] = ctx.assets[c.content.asset]?.path ?? null;
    }
    return {
      content: c.content,
      duration: c.duration,
      source_in: c.source_in,
      speed: c.speed,
      reversed: c.reversed,
      name: c.name,
      properties: c.properties,
      trackId: c.track,
      trackKind: tr?.kind ?? "visual",
      offset: c.start - min,
    } satisfies ClipboardItem;
  });
  return { version: 1, sourceSequence: ctx.seqId, items, assetPaths };
}

/** Cola no playhead; cada item volta à sua track de origem se existir e couber, senão nova track. */
export function pasteCommands(
  ctx: EditContext,
  payload: ClipboardPayload,
  at: Ticks,
  selectedTrack?: string,
): CommandBody[] {
  const frame = frameOf(ctx.seq);
  const base = alignRound(at, frame);
  const cmds: CommandBody[] = [];
  const failed = new Set<string>();
  for (const it of payload.items) {
    const spec: NewClipSpec = {
      kind: it.trackKind,
      duration: it.duration,
      content: it.content,
      name: it.name,
      source_in: it.source_in,
      speed: it.speed,
      reversed: it.reversed,
      properties: it.properties,
    };
    const preferred =
      it.trackId && trackById(ctx.seq, it.trackId)?.kind === it.trackKind
        ? it.trackId
        : selectedTrack;
    const placed = placeClips(ctx, [spec], base + it.offset, preferred ? { track: preferred } : {});
    cmds.push(...placed.commands);
    failed.add(it.trackId);
  }
  return cmds;
}

/** Duplica a seleção logo após o fim do conjunto (mesmas tracks). */
export function duplicateCommands(ctx: EditContext): CommandBody[] {
  const payload = copyPayload(ctx);
  if (!payload) return [];
  const clips = ctx.selection
    .map((id) => ctx.seq.clips[id])
    .filter((c): c is Clip => c !== undefined);
  const maxEnd = Math.max(...clips.map(end));
  return pasteCommands(ctx, payload, maxEnd);
}

/** Duplicar arrastando (Alt+drag): mesmas tracks de destino do movimento. */
export function duplicateAtCommands(
  ctx: EditContext,
  moves: { clip: string; track: string; start: Ticks }[],
): CommandBody[] {
  const cmds: CommandBody[] = [];
  for (const m of moves) {
    const c = ctx.seq.clips[m.clip];
    const tr = trackById(ctx.seq, m.track);
    if (!c || !tr) continue;
    const spec: NewClipSpec = {
      kind: tr.kind,
      duration: c.duration,
      content: c.content,
      name: c.name,
      source_in: c.source_in,
      speed: c.speed,
      reversed: c.reversed,
      properties: c.properties,
    };
    cmds.push(...placeClips(ctx, [spec], m.start, { track: m.track }).commands);
  }
  return cmds;
}

// ------------------------------------------------------------------ grupos, propriedades, tracks

export function groupCommands(ctx: EditContext): CommandBody[] {
  return ctx.selection.length >= 2 ? [{ type: "group_clips", clips: ctx.selection }] : [];
}

export function ungroupCommands(ctx: EditContext): CommandBody[] {
  return ctx.selection.some((id) => ctx.seq.clips[id]?.group)
    ? [{ type: "ungroup", clips: ctx.selection }]
    : [];
}

/** Valor de propriedade: com keyframes ativos cria/atualiza um keyframe no playhead. */
export function setPropertyCommand(
  clip: Clip,
  prop: string,
  value: number,
  playhead: Ticks,
  frame: Ticks,
): CommandBody {
  const a = clip.properties[prop];
  if (a && "animated" in a) {
    const at = Math.min(
      clip.start + clip.duration,
      Math.max(clip.start, alignRound(playhead, frame)),
    );
    return { type: "add_keyframe", clip: clip.id, prop, at, value, interp: "linear" };
  }
  return { type: "set_property", clip: clip.id, prop, value };
}

export function trackFlagsCommand(
  track: string,
  flags: Partial<Record<"locked" | "hidden" | "muted" | "solo" | "magnetic", boolean>>,
): CommandBody {
  return { type: "set_track_flags", track, ...flags };
}

/** Faixas padrão de uma sequence nova (ordem do documento: a primeira é a de baixo da pilha). */
export function defaultTrackCommands(seqId: string): CommandBody[] {
  return [
    addTrackCommand(seqId, "visual", "main", { name: "Main", magnetic: true }),
    addTrackCommand(seqId, "visual", "overlay", { name: "Overlay" }),
    addTrackCommand(seqId, "visual", "text", { name: "Text" }),
    addTrackCommand(seqId, "audio", "voice", { name: "Voice" }),
    addTrackCommand(seqId, "audio", "music", { name: "Music" }),
    addTrackCommand(seqId, "audio", "sfx", { name: "SFX" }),
  ];
}

export const FORMAT_PRESETS = {
  vertical: { width: 1080, height: 1920 },
  square: { width: 1080, height: 1080 },
  portrait: { width: 1080, height: 1350 },
  landscape: { width: 1920, height: 1080 },
} as const;

export type FormatPreset = keyof typeof FORMAT_PRESETS;
