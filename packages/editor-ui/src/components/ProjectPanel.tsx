import { useMemo, useState, type DragEvent, type KeyboardEvent } from "react";
import { Badge, Icon, IconButton, EmptyState, type MenuEntry } from "@capia/ui-kit";
import type { Folder, SequenceSummary } from "@capia/engine-bindings";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { formatFps } from "../lib/timecode";
import { FORMAT_PRESETS, type FormatPreset } from "../store/edit";
import { useContextMenu } from "./Menus";

const SEQ_MIME = "application/x-capia-sequence";
const FOLDER_MIME = "application/x-capia-folder";

function InlineRename({
  initial,
  onCommit,
  onCancel,
}: {
  initial: string;
  onCommit: (v: string) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState(initial);
  const key = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") onCommit(v);
    else if (e.key === "Escape") onCancel();
    e.stopPropagation();
  };
  return (
    <input
      className="cp-input"
      aria-label="rename"
      data-testid="inline-rename"
      autoFocus
      value={v}
      style={{ flex: 1, height: 22 }}
      onFocus={(e) => {
        e.currentTarget.select();
      }}
      onChange={(e) => {
        setV(e.currentTarget.value);
      }}
      onKeyDown={key}
      onBlur={() => {
        onCommit(v);
      }}
    />
  );
}

export function ProjectPanel() {
  const c = useController();
  const t = useT();
  const { sequences, folders, active, renaming, deliverables } = useUi((s) => ({
    sequences: s.model.sequences,
    folders: s.model.folders,
    active: s.active,
    renaming: s.renaming,
    deliverables: s.model.deliverables,
  }));
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null);
  const [dropOver, setDropOver] = useState<string | null>(null);
  const menu = useContextMenu();
  const presetMenu = useContextMenu();

  const childrenOf = useMemo(() => {
    const byParent = new Map<string | null, Folder[]>();
    for (const f of Object.values(folders)) {
      const k = f.parent;
      byParent.set(k, [...(byParent.get(k) ?? []), f]);
    }
    for (const l of byParent.values()) l.sort((a, b) => a.name.localeCompare(b.name));
    const seqsBy = new Map<string | null, SequenceSummary[]>();
    for (const sq of Object.values(sequences)) {
      seqsBy.set(sq.folder, [...(seqsBy.get(sq.folder) ?? []), sq]);
    }
    for (const l of seqsBy.values()) l.sort((a, b) => a.name.localeCompare(b.name));
    return { byParent, seqsBy };
  }, [folders, sequences]);

  const onDropOn = (e: DragEvent, folder: string | null) => {
    e.preventDefault();
    setDropOver(null);
    const seq = e.dataTransfer.getData(SEQ_MIME);
    const fld = e.dataTransfer.getData(FOLDER_MIME);
    if (seq) void c.moveSequenceToFolder(seq, folder);
    else if (fld && fld !== folder) void c.moveFolder(fld, folder);
  };
  const allowDrop = (e: DragEvent, id: string | null) => {
    if (e.dataTransfer.types.includes(SEQ_MIME) || e.dataTransfer.types.includes(FOLDER_MIME)) {
      e.preventDefault();
      setDropOver(id ?? "root");
    }
  };

  const seqMenu = (sq: SequenceSummary, x: number, y: number) => {
    const entries: MenuEntry[] = [
      { type: "item", id: "open", label: t("project.open") },
      { type: "item", id: "rename", label: t("common.rename") },
      { type: "item", id: "duplicate", label: t("common.duplicate") },
      { type: "item", id: "deliverable", label: t("project.addToDeliverables") },
      ...(sq.folder
        ? ([{ type: "item", id: "root", label: t("project.moveToRoot") }] as MenuEntry[])
        : []),
      { type: "separator" },
      { type: "item", id: "delete", label: t("project.deleteSequence"), danger: true },
    ];
    menu.open({
      x,
      y,
      label: sq.name,
      entries,
      onSelect: (id) => {
        if (id === "open") void c.openSequence(sq.id);
        else if (id === "rename") {
          void c.openSequence(sq.id).then(() => {
            c.store.set({ renaming: sq.id });
          });
        } else if (id === "duplicate") void c.duplicateSequence(sq.id);
        else if (id === "root") void c.moveSequenceToFolder(sq.id, null);
        else if (id === "delete") void c.deleteSequence(sq.id);
        else if (id === "deliverable") {
          void c.createDeliverable({
            name: sq.name,
            sequence: sq.id,
            preset: "h264-mp4",
            path: `${sq.name.replace(/[^\w.-]+/g, "_")}.mp4`,
          });
        }
      },
    });
  };

  const renderFolder = (f: Folder, depth: number): React.ReactNode => {
    const open = !collapsed.has(f.id);
    return (
      <div key={f.id} role="treeitem" aria-expanded={open} aria-selected={false}>
        <div
          className="ed-tree-item"
          style={{ paddingLeft: 8 + depth * 14 }}
          data-drop={dropOver === f.id}
          data-testid={`folder-${f.id}`}
          draggable
          onDragStart={(e) => {
            e.dataTransfer.setData(FOLDER_MIME, f.id);
          }}
          onDragOver={(e) => {
            allowDrop(e, f.id);
          }}
          onDragLeave={() => {
            setDropOver(null);
          }}
          onDrop={(e) => {
            onDropOn(e, f.id);
          }}
          onClick={() => {
            setCollapsed((s) => {
              const n = new Set(s);
              if (n.has(f.id)) n.delete(f.id);
              else n.add(f.id);
              return n;
            });
          }}
          onDoubleClick={() => {
            setRenamingFolder(f.id);
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            menu.open({
              x: e.clientX,
              y: e.clientY,
              label: f.name,
              entries: [
                { type: "item", id: "rename", label: t("common.rename") },
                { type: "item", id: "new-seq", label: t("project.newSequence") },
                { type: "item", id: "new-folder", label: t("project.newFolder") },
                { type: "separator" },
                { type: "item", id: "delete", label: t("project.deleteFolder"), danger: true },
              ],
              onSelect: (id) => {
                if (id === "rename") setRenamingFolder(f.id);
                else if (id === "new-seq") void c.createSequence({ folder: f.id });
                else if (id === "new-folder")
                  void c.createFolder(
                    t("project.folderName", { n: Object.keys(folders).length + 1 }),
                    f.id,
                  );
                else if (id === "delete") void c.deleteFolder(f.id);
              },
            });
          }}
        >
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="folder" size={14} />
          {renamingFolder === f.id ? (
            <InlineRename
              initial={f.name}
              onCancel={() => {
                setRenamingFolder(null);
              }}
              onCommit={(v) => {
                setRenamingFolder(null);
                if (v.trim() && v.trim() !== f.name) void c.renameFolder(f.id, v.trim());
              }}
            />
          ) : (
            <span className="ed-tree-label">{f.name}</span>
          )}
        </div>
        {open && renderChildren(f.id, depth + 1)}
      </div>
    );
  };

  const renderSeq = (sq: SequenceSummary, depth: number): React.ReactNode => (
    <div
      key={sq.id}
      role="treeitem"
      aria-selected={active === sq.id}
      className="ed-tree-item"
      style={{ paddingLeft: 8 + depth * 14 + 14 }}
      data-active={active === sq.id}
      data-testid={`seq-${sq.id}`}
      data-seq-name={sq.name}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(SEQ_MIME, sq.id);
        e.dataTransfer.setData("application/x-capia-sequence-nested", sq.id);
        c.setDragging({ span: sq.duration });
      }}
      onDragEnd={() => {
        c.setDragging(null);
      }}
      onDoubleClick={() => {
        void c.openSequence(sq.id);
      }}
      onClick={() => {
        if (c.state.tabs.includes(sq.id)) c.setActive(sq.id);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        seqMenu(sq, e.clientX, e.clientY);
      }}
    >
      <Icon name="film" size={14} />
      {renaming === sq.id ? (
        <InlineRename
          initial={sq.name}
          onCancel={() => {
            c.stopRenaming();
          }}
          onCommit={(v) => {
            void c.renameSequence(sq.id, v);
          }}
        />
      ) : (
        <span className="ed-tree-label" title={sq.name}>
          {sq.name}
        </span>
      )}
      <Badge>{`${String(sq.width)}×${String(sq.height)}`}</Badge>
      <Badge>{formatFps(sq.frame_rate)}</Badge>
      {sq.nested_usage > 0 && (
        <Badge tone="success">{t("project.usedBy", { count: sq.nested_usage })}</Badge>
      )}
      {Object.values(deliverables).some((d) => d.sequence === sq.id) && (
        <Badge tone="warning">↗</Badge>
      )}
    </div>
  );

  const renderChildren = (parent: string | null, depth: number): React.ReactNode => (
    <>
      {(childrenOf.byParent.get(parent) ?? []).map((f) => renderFolder(f, depth))}
      {(childrenOf.seqsBy.get(parent) ?? []).map((sq) => renderSeq(sq, depth))}
    </>
  );

  const total = Object.keys(sequences).length;
  const presetEntries: MenuEntry[] = [
    { type: "item", id: "active", label: t("preset.fromActive") },
    { type: "separator" },
    ...(Object.keys(FORMAT_PRESETS) as FormatPreset[]).map((p): MenuEntry => ({
      type: "item",
      id: p,
      label: t(`preset.${p}`),
    })),
  ];

  return (
    <section className="ed-panel" aria-label={t("project.title")} data-testid="project-panel">
      <div className="ed-panel-head">
        <span>{t("project.title")}</span>
        <div className="ed-spacer" />
        <IconButton
          icon="plus"
          label={t("project.newSequence")}
          data-testid="new-sequence"
          onClick={() => {
            void c.createSequence();
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            presetMenu.open({
              x: e.clientX,
              y: e.clientY,
              label: t("preset.newFrom"),
              entries: presetEntries,
              onSelect: (id) => void c.createSequence({ preset: id as FormatPreset | "active" }),
            });
          }}
        />
        <IconButton
          icon="chevronDown"
          label={t("preset.newFrom")}
          data-testid="new-sequence-preset"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            presetMenu.open({
              x: r.left,
              y: r.bottom + 4,
              label: t("preset.newFrom"),
              entries: presetEntries,
              onSelect: (id) => void c.createSequence({ preset: id as FormatPreset | "active" }),
            });
          }}
        />
        <IconButton
          icon="folder"
          label={t("project.newFolder")}
          data-testid="new-folder"
          onClick={() => {
            void c.createFolder(
              t("project.folderName", { n: Object.keys(folders).length + 1 }),
              null,
            );
          }}
        />
      </div>
      <div
        className="ed-panel-body"
        role="tree"
        aria-label={t("project.sequences")}
        data-drop={dropOver === "root"}
        onDragOver={(e) => {
          allowDrop(e, null);
        }}
        onDragLeave={() => {
          setDropOver(null);
        }}
        onDrop={(e) => {
          onDropOn(e, null);
        }}
      >
        {total === 0 && Object.keys(folders).length === 0 ? (
          <EmptyState icon="film" title={t("project.empty")} hint={t("project.emptyHint")} />
        ) : (
          renderChildren(null, 0)
        )}
      </div>
      {menu.node}
      {presetMenu.node}
    </section>
  );
}
