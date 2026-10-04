import { useEffect, useMemo, useState } from "react";
import {
  Badge,
  Button,
  Dialog,
  IconButton,
  NumberField,
  Select,
  Spinner,
  TextInput,
} from "@capia/ui-kit";
import type { Deliverable, EncoderCapability, ExportItem } from "@capia/engine-bindings";
import { useController, useUi } from "../context";
import { useT } from "../i18n";

type Preset = "h264-mp4" | "intermediate";

function dirOf(path: string): { dir: string; sep: string } {
  const sep = path.includes("\\") ? "\\" : "/";
  const i = path.lastIndexOf(sep);
  return { dir: i >= 0 ? path.slice(0, i) : ".", sep };
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");

function safeName(n: string): string {
  return n.replace(/[^\w.-]+/g, "_").replace(/^_+|_+$/g, "") || "export";
}

export function ExportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const c = useController();
  const t = useT();
  const { sequences, active, deliverables, projectPath, run } = useUi((s) => ({
    sequences: s.model.sequences,
    active: s.active,
    deliverables: s.model.deliverables,
    projectPath: s.model.project?.path ?? "",
    run: s.exportRun,
  }));
  const [encoders, setEncoders] = useState<EncoderCapability[] | null>(null);
  const [sequence, setSequence] = useState<string>("");
  const [preset, setPreset] = useState<Preset>("h264-mp4");
  const [encoder, setEncoder] = useState<string>("");
  const [width, setWidth] = useState(0);
  const [height, setHeight] = useState(0);
  const [path, setPath] = useState("");
  const [overwrite, setOverwrite] = useState(false);

  const seq = sequences[sequence];
  const { dir, sep } = dirOf(projectPath);
  const ext = preset === "h264-mp4" ? ".mp4" : "";
  const suggested = seq ? `${dir}${sep}${safeName(seq.name)}${ext}` : "";

  // ao abrir: escolhe a sequence ativa e consulta as capacidades de encoder aprovadas
  useEffect(() => {
    if (!open) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- inicialização ao abrir o diálogo
    setSequence(active ?? Object.keys(sequences)[0] ?? "");
    setEncoders(null);
    let live = true;
    c.client.encoders().then(
      (e) => {
        if (live) setEncoders(e);
      },
      (e: unknown) => {
        if (live) setEncoders([]);
        c.reportError(e);
      },
    );
    return () => {
      live = false;
    };
    // só ao abrir
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- sugestão de destino
    setPath(suggested);
    setWidth(seq?.width ?? 0);
    setHeight(seq?.height ?? 0);
  }, [suggested, seq?.width, seq?.height]);

  const usable = (encoders ?? []).filter(
    (e) => e.available && e.codec.toLowerCase().includes("264"),
  );
  const noEncoder = encoders !== null && usable.length === 0;
  const running = run !== null && !run.finished;
  const labelFor = (id: string, name: string) => ({ [id]: name });

  const itemFor = (): ExportItem => ({
    id: "single",
    sequence,
    preset,
    path,
    width: width || null,
    height: height || null,
    overwrite,
    encoder: preset === "h264-mp4" && encoder ? encoder : null,
  });

  const start = () => {
    if (!seq) return;
    void c.startExport([itemFor()], labelFor("single", seq.name));
  };

  const batch = () => {
    const ds = Object.values(deliverables);
    if (ds.length === 0) return;
    const items: ExportItem[] = ds.map((d) => ({
      id: d.id,
      sequence: d.sequence,
      preset: d.preset === "intermediate" ? "intermediate" : "h264-mp4",
      path: d.path,
      width: d.width ?? null,
      height: d.height ?? null,
      overwrite,
      encoder: null,
    }));
    void c.startExport(items, Object.fromEntries(ds.map((d) => [d.id, d.name])));
  };

  const addDeliverable = () => {
    if (!seq || !path) return;
    void c.createDeliverable({
      name: `${seq.name} ${String(width || seq.width)}×${String(height || seq.height)}`,
      sequence,
      preset,
      path,
      ...(width ? { width } : {}),
      ...(height ? { height } : {}),
    });
  };

  const browse = async () => {
    const p = await c.platform.pickSavePath({
      title: t("export.destination"),
      defaultName: `${safeName(seq?.name ?? "export")}${ext}`,
      filters: preset === "h264-mp4" ? [{ name: "MP4", extensions: ["mp4"] }] : [],
    });
    if (p) setPath(p);
  };

  const dl: Deliverable[] = useMemo(() => Object.values(deliverables), [deliverables]);
  const stateLabel = {
    pending: t("export.itemPending"),
    running: t("export.itemRunning"),
    done: t("export.itemOk"),
    failed: t("export.itemFailed"),
    cancelled: t("export.cancelled"),
  } as const;

  return (
    <Dialog
      title={t("export.title")}
      open={open}
      onClose={onClose}
      footer={
        <>
          {running && (
            <Button
              data-testid="export-cancel"
              onClick={() => {
                void c.cancelExport();
              }}
            >
              {t("export.cancel")}
            </Button>
          )}
          <Button onClick={onClose}>{t("common.close")}</Button>
          <Button
            variant="primary"
            data-testid="export-start"
            disabled={running || !seq || path.trim() === "" || (preset === "h264-mp4" && noEncoder)}
            onClick={start}
          >
            {t("export.start")}
          </Button>
        </>
      }
    >
      <div className="ed-export" data-testid="export-dialog">
        <Select
          label={t("export.sequence")}
          value={sequence}
          data-testid="export-sequence"
          onChange={(e) => {
            setSequence(e.currentTarget.value);
          }}
        >
          {Object.values(sequences).map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </Select>
        <Select
          label={t("export.preset")}
          value={preset}
          data-testid="export-preset"
          onChange={(e) => {
            setPreset(e.currentTarget.value as Preset);
          }}
        >
          <option value="h264-mp4">{t("export.preset.h264")}</option>
          <option value="intermediate">{t("export.preset.intermediate")}</option>
        </Select>
        {preset === "h264-mp4" && (
          <>
            <Select
              label={t("export.encoder")}
              value={encoder}
              data-testid="export-encoder"
              onChange={(e) => {
                setEncoder(e.currentTarget.value);
              }}
            >
              <option value="">{t("export.encoderAuto")}</option>
              {(encoders ?? [])
                .filter((e) => e.codec.toLowerCase().includes("264"))
                .map((e) => (
                  <option key={e.ffmpeg_name} value={e.ffmpeg_name} disabled={!e.available}>
                    {e.ffmpeg_name} ·{" "}
                    {e.available
                      ? e.hardware
                        ? t("export.encoderHardware")
                        : t("export.encoderSoftware")
                      : t("export.encoderUnavailable")}
                  </option>
                ))}
            </Select>
            {encoders === null && <Spinner label={t("app.loading")} />}
            {noEncoder && (
              <p role="alert" className="ed-warn" data-testid="export-no-encoder">
                {t("export.noEncoder")}
              </p>
            )}
            <p className="ed-hint" data-testid="export-legal">
              {t("export.legalNote")}
            </p>
          </>
        )}
        <div className="ed-field-row">
          <span className="ed-field-label">{t("export.width")}</span>
          <NumberField
            label={t("export.width")}
            value={width}
            min={2}
            max={7680}
            onCommit={setWidth}
          />
        </div>
        <div className="ed-field-row">
          <span className="ed-field-label">{t("export.height")}</span>
          <NumberField
            label={t("export.height")}
            value={height}
            min={2}
            max={7680}
            onCommit={setHeight}
          />
        </div>
        <div className="ed-field-row">
          <span className="ed-field-label">{t("export.frameRate")}</span>
          <span>{t("export.frameRatePolicy")}</span>
        </div>
        <TextInput
          label={t("export.destination")}
          value={path}
          data-testid="export-path"
          onChange={(e) => {
            setPath(e.currentTarget.value);
          }}
        />
        {c.platform.native && (
          <Button
            onClick={() => {
              void browse();
            }}
          >
            {t("common.details")}…
          </Button>
        )}
        <label className="ed-check">
          <input
            type="checkbox"
            checked={overwrite}
            data-testid="export-overwrite"
            onChange={(e) => {
              setOverwrite(e.currentTarget.checked);
            }}
          />
          {t("export.overwrite")}
        </label>

        <h3 className="ed-subhead">{t("export.deliverables")}</h3>
        {dl.length === 0 ? (
          <p className="ed-hint">{t("export.noDeliverables")}</p>
        ) : (
          <ul className="ed-deliv" data-testid="deliverables">
            {dl.map((d) => (
              <li key={d.id}>
                <span title={d.path}>
                  {d.name} <small>{d.path}</small>
                </span>
                <IconButton
                  icon="trash"
                  label={t("export.removeDeliverable")}
                  onClick={() => {
                    void c.deleteDeliverable(d.id);
                  }}
                />
              </li>
            ))}
          </ul>
        )}
        <div className="ed-row">
          <Button data-testid="deliverable-add" onClick={addDeliverable} disabled={!seq || !path}>
            {t("export.addDeliverable")}
          </Button>
          <Button data-testid="export-batch" disabled={dl.length === 0 || running} onClick={batch}>
            {t("export.batch")}
          </Button>
        </div>

        {run && (
          <ul className="ed-export-run" data-testid="export-run" aria-live="polite">
            {run.items.map((it) => (
              <li key={it.id} data-state={it.state} data-testid={`export-item-${it.id}`}>
                <strong>{it.label}</strong>{" "}
                <Badge
                  {...(it.state === "done"
                    ? { tone: "success" as const }
                    : it.state === "failed"
                      ? { tone: "danger" as const }
                      : {})}
                >
                  {stateLabel[it.state]}
                </Badge>
                {it.state === "running" && (
                  <>
                    {" "}
                    <progress value={it.done} max={Math.max(1, it.total)} />{" "}
                    {t("export.progress", { done: it.done, total: it.total })}
                  </>
                )}
                {it.state === "done" && it.report && (
                  <div className="ed-hint" data-testid="export-report">
                    {t("export.validated")}: {str(it.report.codec)} {str(it.report.encoder)} ·{" "}
                    {String(it.report.width)}×{String(it.report.height)} ·{" "}
                    {String(it.report.frames)} frames
                    <br />
                    <code>{String(it.report.path)}</code>
                  </div>
                )}
                {it.error && (
                  <div role="alert" className="ed-warn">
                    {it.error.code}: {it.error.message}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </Dialog>
  );
}
