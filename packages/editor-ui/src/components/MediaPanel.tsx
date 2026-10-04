import { useEffect, useMemo, useReducer, useState } from "react";
import {
  Badge,
  Button,
  Dialog,
  EmptyState,
  Icon,
  IconButton,
  Select,
  TextInput,
  type MenuEntry,
} from "@capia/ui-kit";
import type { AssetRow } from "@capia/engine-bindings";
import { useController, useUi } from "../context";
import { useT } from "../i18n";
import { formatDuration } from "../lib/timecode";
import { useContextMenu } from "./Menus";

type Filter = "all" | "video" | "audio" | "image";
type Sort = "name" | "duration" | "type";

export const ASSET_MIME = "application/x-capia-asset";

function Thumb({ asset }: { asset: AssetRow }) {
  const c = useController();
  const [, bump] = useReducer((n: number) => n + 1, 0);
  useEffect(() => c.visuals.subscribe(bump), [c]);
  const hasPicture = asset.kind === "video" || asset.kind === "image";
  const url =
    hasPicture && asset.status === "online" && asset.has_file
      ? c.visuals.thumbnailUrl(asset.id)
      : null;
  if (url) return <img src={url} alt="" draggable={false} />;
  return (
    <Icon
      name={asset.kind === "audio" ? "music" : asset.kind === "image" ? "image" : "film"}
      size={24}
    />
  );
}

export function MediaPanel({ initialFilter = "all" }: { initialFilter?: Filter }) {
  const c = useController();
  const t = useT();
  const { assets, pending, mediaOk, last } = useUi((s) => ({
    assets: s.model.assets,
    pending: s.pendingImports,
    mediaOk: s.engine?.mediaAvailable !== false,
    last: s.lastImported,
  }));
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>(initialFilter);
  const [sort, setSort] = useState<Sort>("name");
  const [importOpen, setImportOpen] = useState(false);
  const [importText, setImportText] = useState("");
  const [relink, setRelink] = useState<{ asset: AssetRow; force: boolean } | null>(null);
  const [relinkPath, setRelinkPath] = useState("");
  const [folderOpen, setFolderOpen] = useState(false);
  const [folderPath, setFolderPath] = useState("");
  const menu = useContextMenu();

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = Object.values(assets).filter(
      (a) =>
        a.has_file &&
        (filter === "all" || a.kind === filter) &&
        (q === "" || a.name.toLowerCase().includes(q)),
    );
    const key = (a: AssetRow): string | number =>
      sort === "duration"
        ? (a.duration ?? 0)
        : sort === "type"
          ? (a.kind ?? "")
          : a.name.toLowerCase();
    return list.sort((a, b) => {
      const ka = key(a);
      const kb = key(b);
      return ka < kb ? -1 : ka > kb ? 1 : 0;
    });
  }, [assets, query, filter, sort]);

  const doImport = async () => {
    const paths = importText.split("\n");
    setImportOpen(false);
    setImportText("");
    await c.importPaths(paths);
  };

  const pickFiles = async () => {
    const picked = await c.platform.pickFiles({
      title: t("media.import"),
      multiple: true,
      filters: [
        {
          name: "Media",
          extensions: [
            "mp4",
            "mov",
            "mkv",
            "webm",
            "avi",
            "mp3",
            "wav",
            "m4a",
            "aac",
            "flac",
            "ogg",
            "png",
            "jpg",
            "jpeg",
            "webp",
          ],
        },
      ],
    });
    if (picked && picked.length > 0) await c.importPaths(picked);
  };

  const openMenu = (a: AssetRow, x: number, y: number) => {
    const entries: MenuEntry[] = [
      { type: "item", id: "add", label: t("media.addToTimeline"), disabled: a.status !== "online" },
      { type: "separator" },
      { type: "item", id: "relink", label: t("media.relink") },
      { type: "item", id: "relinkFolder", label: t("media.relinkFolder") },
      { type: "item", id: "force", label: t("media.forceRelink"), danger: true },
    ];
    menu.open({
      x,
      y,
      label: a.name,
      entries,
      onSelect: (id) => {
        if (id === "add") void c.addAssetAtPlayhead(a.id);
        else if (id === "relink") {
          setRelinkPath("");
          setRelink({ asset: a, force: false });
        } else if (id === "force") {
          setRelinkPath("");
          setRelink({ asset: a, force: true });
        } else if (id === "relinkFolder") {
          setFolderPath("");
          setFolderOpen(true);
        }
      },
    });
  };

  const offlineCount = Object.values(assets).filter(
    (a) => a.has_file && a.status !== "online",
  ).length;

  return (
    <section className="ed-panel" aria-label={t("media.title")} data-testid="media-panel">
      <div className="ed-panel-head">
        <span>{t("media.title")}</span>
        <div className="ed-spacer" />
        {offlineCount > 0 && (
          <Badge tone="danger">{`${String(offlineCount)} ${t("media.offline")}`}</Badge>
        )}
        <Button
          variant="primary"
          disabled={!mediaOk}
          data-testid="import-media"
          onClick={() => {
            if (c.platform.native) void pickFiles();
            else setImportOpen(true);
          }}
        >
          <Icon name="import" /> {t("media.import")}
        </Button>
      </div>
      <div style={{ display: "flex", gap: 6, padding: 8, flexWrap: "wrap" }}>
        <TextInput
          aria-label={t("media.search")}
          placeholder={t("media.search")}
          value={query}
          data-testid="media-search"
          style={{ flex: 1, minWidth: 80 }}
          onChange={(e) => {
            setQuery(e.currentTarget.value);
          }}
        />
        <Select
          aria-label={t("common.search")}
          value={filter}
          data-testid="media-filter"
          onChange={(e) => {
            setFilter(e.currentTarget.value as Filter);
          }}
        >
          {(["all", "video", "audio", "image"] as const).map((f) => (
            <option key={f} value={f}>
              {t(`media.filter.${f}`)}
            </option>
          ))}
        </Select>
        <Select
          aria-label={t("media.sort")}
          value={sort}
          data-testid="media-sort"
          onChange={(e) => {
            setSort(e.currentTarget.value as Sort);
          }}
        >
          {(["name", "duration", "type"] as const).map((f) => (
            <option key={f} value={f}>
              {t(`media.sort.${f}`)}
            </option>
          ))}
        </Select>
      </div>
      <div className="ed-panel-body">
        {rows.length === 0 && pending === 0 ? (
          <EmptyState icon="film" title={t("media.empty")} hint={t("media.emptyHint")} />
        ) : (
          <div className="ed-media-grid" data-testid="media-grid">
            {Array.from({ length: pending }, (_, i) => (
              <div key={`p${String(i)}`} className="ed-media-card" aria-busy="true">
                <div className="ed-media-thumb">
                  <span className="cp-spinner" role="status" aria-label={t("media.importing")} />
                </div>
                <div className="ed-media-name">{t("media.importing")}</div>
              </div>
            ))}
            {rows.map((a) => (
              <div
                key={a.id}
                className="ed-media-card"
                role="button"
                tabIndex={0}
                draggable={a.status === "online"}
                data-testid={`asset-${a.id}`}
                data-asset-name={a.name}
                data-status={a.status}
                data-recent={last === a.id}
                title={a.path ?? a.name}
                onDragStart={(e) => {
                  e.dataTransfer.setData(ASSET_MIME, a.id);
                  e.dataTransfer.effectAllowed = "copy";
                  c.setDragging({
                    span: a.kind === "image" ? 5 * 705_600_000 : (a.duration ?? 3 * 705_600_000),
                  });
                }}
                onDragEnd={() => {
                  c.setDragging(null);
                }}
                onDoubleClick={() => {
                  if (a.status === "online") void c.addAssetAtPlayhead(a.id);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && a.status === "online") void c.addAssetAtPlayhead(a.id);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  openMenu(a, e.clientX, e.clientY);
                }}
              >
                <div className="ed-media-thumb">
                  <Thumb asset={a} />
                  {a.duration !== null && a.kind !== "image" && (
                    <span
                      style={{ position: "absolute", right: 4, bottom: 4 }}
                      className="cp-badge"
                    >
                      {formatDuration(a.duration)}
                    </span>
                  )}
                </div>
                <div className="ed-media-name">{a.name}</div>
                <div className="ed-media-meta">
                  <span>
                    {a.width && a.height
                      ? `${String(a.width)}×${String(a.height)}`
                      : (a.kind ?? "")}
                  </span>
                  {a.status === "offline" && <Badge tone="danger">{t("media.offline")}</Badge>}
                  {a.status === "modified" && <Badge tone="warning">{t("media.modified")}</Badge>}
                  {!a.in_document && <Badge>{t("media.unusedBadge")}</Badge>}
                </div>
                {a.status !== "online" && (
                  <Button
                    variant="danger"
                    data-testid={`relink-${a.id}`}
                    onClick={() => {
                      setRelinkPath("");
                      setRelink({ asset: a, force: false });
                    }}
                  >
                    {t("media.relink")}
                  </Button>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      <Dialog
        title={t("media.import")}
        open={importOpen}
        onClose={() => {
          setImportOpen(false);
        }}
        initialFocus="textarea"
        footer={
          <>
            <Button
              onClick={() => {
                setImportOpen(false);
              }}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              data-testid="import-confirm"
              disabled={importText.trim() === ""}
              onClick={() => void doImport()}
            >
              {t("media.import")}
            </Button>
          </>
        }
      >
        <p style={{ margin: 0, color: "var(--text-muted)" }}>{t("media.importHint")}</p>
        <label className="cp-field">
          <span>{t("media.importPrompt")}</span>
          <textarea
            className="cp-input"
            rows={5}
            style={{ height: "auto", padding: 8, fontFamily: "var(--font-mono)" }}
            data-testid="import-paths"
            value={importText}
            onChange={(e) => {
              setImportText(e.currentTarget.value);
            }}
          />
        </label>
      </Dialog>

      <Dialog
        title={relink?.force ? t("media.forceRelink") : t("media.relink")}
        open={relink !== null}
        onClose={() => {
          setRelink(null);
        }}
        initialFocus="input"
        footer={
          <>
            <Button
              onClick={() => {
                setRelink(null);
              }}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant={relink?.force ? "danger" : "primary"}
              data-testid="relink-confirm"
              disabled={relinkPath.trim() === ""}
              onClick={() => {
                const r = relink;
                setRelink(null);
                if (r)
                  void c.relink(r.asset.id, relinkPath.trim(), { force: r.force }).then(() => {
                    c.visuals.invalidate(r.asset.id);
                  });
              }}
            >
              {relink?.force ? t("media.forceRelink") : t("media.relink")}
            </Button>
          </>
        }
      >
        {relink?.force && (
          <div role="alert" style={{ color: "var(--warning)" }}>
            {t("media.forceRelinkWarning")}
          </div>
        )}
        <TextInput
          label={t("media.relinkPath")}
          value={relinkPath}
          data-testid="relink-path"
          onChange={(e) => {
            setRelinkPath(e.currentTarget.value);
          }}
        />
        {c.platform.native && (
          <IconButton
            icon="folder"
            label={t("media.relinkPath")}
            onClick={() => {
              void c.platform.pickFiles({ title: t("media.relinkPath") }).then((p) => {
                if (p?.[0]) setRelinkPath(p[0]);
              });
            }}
          />
        )}
      </Dialog>

      <Dialog
        title={t("media.relinkFolder")}
        open={folderOpen}
        onClose={() => {
          setFolderOpen(false);
        }}
        initialFocus="input"
        footer={
          <>
            <Button
              onClick={() => {
                setFolderOpen(false);
              }}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              data-testid="relink-folder-confirm"
              disabled={folderPath.trim() === ""}
              onClick={() => {
                setFolderOpen(false);
                void c.relinkFolder(folderPath.trim());
              }}
            >
              {t("media.relinkFolder")}
            </Button>
          </>
        }
      >
        <TextInput
          label={t("media.relinkFolderPath")}
          value={folderPath}
          data-testid="relink-folder-path"
          onChange={(e) => {
            setFolderPath(e.currentTarget.value);
          }}
        />
      </Dialog>
      {menu.node}
    </section>
  );
}
