/**
 * Réplica local do documento: aplica os patches (ops primitivas) devolvidos pelo engine. É estado
 * **derivado** — a fonte de verdade é o Command Engine; a réplica nunca é escrita pela UI.
 * Funções puras e imutáveis (novo objeto só nas partes que mudam: compartilhamento estrutural).
 */
import type {
  AssetRow,
  ChangeSet,
  Clip,
  Deliverable,
  Folder,
  Marker,
  PatchOp,
  ProjectSnapshot,
  SequenceHeader,
  SequenceModel,
  SequenceSummary,
} from "./editor";

export interface ReadModel {
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  project: { path: string; name: string | null } | null;
  sequences: Record<string, SequenceSummary>;
  folders: Record<string, Folder>;
  deliverables: Record<string, Deliverable>;
  assets: Record<string, AssetRow>;
  /** Conteúdo das sequences já carregadas (as demais são buscadas sob demanda). */
  models: Record<string, SequenceModel>;
}

export const EMPTY_MODEL: ReadModel = {
  revision: 0,
  canUndo: false,
  canRedo: false,
  project: null,
  sequences: {},
  folders: {},
  deliverables: {},
  assets: {},
  models: {},
};

const byId = <T extends { id: string }>(items: T[]): Record<string, T> =>
  Object.fromEntries(items.map((i) => [i.id, i]));

export function fromSnapshot(s: ProjectSnapshot): ReadModel {
  return {
    revision: s.revision,
    canUndo: s.can_undo,
    canRedo: s.can_redo,
    project: s.project,
    sequences: byId(s.sequences),
    folders: byId(s.folders),
    deliverables: byId(s.deliverables),
    assets: byId(s.assets),
    models: {},
  };
}

function withoutKey<T>(rec: Record<string, T>, key: string): Record<string, T> {
  if (!(key in rec)) return rec;
  const rest = { ...rec };
  Reflect.deleteProperty(rest, key);
  return rest;
}

function put<T>(rec: Record<string, T>, key: string, value: T | null): Record<string, T> {
  return value === null ? withoutKey(rec, key) : { ...rec, [key]: value };
}

function patchModel(m: SequenceModel, op: PatchOp): SequenceModel {
  switch (op.op) {
    case "clip":
      return { ...m, clips: put<Clip>(m.clips, op.id, op.new) };
    case "marker":
      return { ...m, markers: put<Marker>(m.markers, op.id, op.new) };
    case "track": {
      const rest = m.tracks.filter((t) => t.id !== op.id);
      if (op.new === null) return { ...m, tracks: rest };
      rest.splice(op.new.index, 0, op.new.track);
      return { ...m, tracks: rest };
    }
    default:
      return m;
  }
}

function patchHeader(m: SequenceModel, header: SequenceHeader): SequenceModel {
  return { ...m, header };
}

/**
 * Aplica um conjunto de mudanças. Sequences cujo conteúdo ainda não foi carregado ignoram os
 * patches de clip/track (serão buscadas inteiras ao abrir); resumos e flags sempre atualizam.
 */
export function applyChange(model: ReadModel, change: ChangeSet): ReadModel {
  let next: ReadModel = {
    ...model,
    revision: change.revision,
    canUndo: change.can_undo,
    canRedo: change.can_redo,
  };
  for (const op of change.patches ?? []) {
    switch (op.op) {
      case "clip":
      case "track":
      case "marker": {
        const m = next.models[op.sequence];
        if (m) next = { ...next, models: { ...next.models, [op.sequence]: patchModel(m, op) } };
        break;
      }
      case "sequence": {
        if (op.new === null) {
          next = {
            ...next,
            sequences: withoutKey(next.sequences, op.id),
            models: withoutKey(next.models, op.id),
          };
        } else {
          const m = next.models[op.id];
          if (m) next = { ...next, models: { ...next.models, [op.id]: patchHeader(m, op.new) } };
        }
        break;
      }
      case "folder":
        next = { ...next, folders: put(next.folders, op.id, op.new) };
        break;
      case "deliverable":
        next = { ...next, deliverables: put(next.deliverables, op.id, op.new) };
        break;
      case "asset":
        // as linhas da biblioteca (disponibilidade, caminho) vêm de `assets_changed`/`assets.list`
        break;
    }
  }
  for (const s of change.sequence_summaries ?? []) {
    if (s) next = { ...next, sequences: { ...next.sequences, [s.id]: s } };
  }
  return next;
}

export function withAssets(model: ReadModel, assets: AssetRow[]): ReadModel {
  return { ...model, assets: byId(assets) };
}

export function withSequenceModel(model: ReadModel, id: string, m: SequenceModel): ReadModel {
  return { ...model, models: { ...model.models, [id]: m } };
}
