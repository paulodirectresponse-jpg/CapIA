import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect, MEDIA, ROOT } from "./fixtures";

/**
 * Metas de TIMELINE_UX §6 medidas no navegador contra o engine real, com um projeto de 5.000
 * clips. Escreve `target/perf/phase3-ui-perf.json` (números brutos + passa/falha por meta).
 * Em CI roda como *smoke* (limites folgados, só pega regressão grosseira); a comparação estrita
 * com as metas é `CAPIA_PERF_STRICT=1` (benchmark local, GPU/CPU reais). Nunca finge número.
 */
const TICKS = 705_600_000;
const STRICT = process.env.CAPIA_PERF_STRICT === "1";

interface Stat {
  n: number;
  p50: number;
  p95: number;
  max: number;
}
function stat(s: number[]): Stat {
  if (s.length === 0) return { n: 0, p50: 0, p95: 0, max: 0 };
  const a = [...s].sort((x, y) => x - y);
  const at = (q: number) => a[Math.min(a.length - 1, Math.floor(q * a.length))] ?? 0;
  return { n: a.length, p50: at(0.5), p95: at(0.95), max: a[a.length - 1] ?? 0 };
}

async function window_commitCount(page: import("@playwright/test").Page): Promise<number> {
  return (await page.evaluate(() => window.__capiaPerf?.samples("commit").length)) ?? 0;
}

test("timeline UX §6 targets with 5k clips", async ({ editor, page, server }) => {
  test.setTimeout(300_000);
  await editor.goto();
  await editor.createProject();
  await editor.importMedia("video_audio.mp4", "image.jpg");
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(2);

  // ---- fixture: 5 faixas visuais livres × 1.000 clips sólidos (1 s a cada 1,5 s)
  const seq = await editor.sequence();
  const cmds: Record<string, unknown>[] = [];
  const tracks: string[] = [];
  for (let t = 0; t < 5; t++) {
    const id = `perf-track-${String(t)}`;
    tracks.push(id);
    cmds.push({
      type: "add_track",
      sequence: seq.id,
      id,
      kind: "visual",
      name: `P${String(t)}`,
      role: "overlay",
      magnetic: false,
    });
  }
  const frame = 23_520_000; // 30 fps
  for (const [ti, track] of tracks.entries()) {
    for (let i = 0; i < 1000; i++) {
      cmds.push({
        type: "insert_clip",
        track,
        start: Math.round((i * 1.5 * TICKS) / frame) * frame,
        clip: {
          name: `c${String(ti)}-${String(i)}`,
          duration: 30 * frame,
          content: {
            type: "solid",
            color: ["#3366CC", "#CC6633", "#33AA66", "#AA33AA", "#999933"][ti],
          },
          source_in: 0,
          speed: "1",
          reversed: false,
          properties: {},
        },
      });
    }
  }
  const tCommit0 = Date.now();
  await editor.api("command.execute", {
    label: "perf fixture",
    commands: cmds.map((c, i) => ({ ...c, operation_id: `perf-op-${String(i)}` })),
  });
  const bulkCommitMs = Date.now() - tCommit0;
  expect(Object.keys((await editor.sequence()).clips).length).toBe(5000);

  // ---- abrir o projeto com 5k clips
  await page.getByTestId("close-project").click();
  await expect(page.getByTestId("welcome")).toBeVisible();
  await page.getByTestId("project-path").fill(join(server.dir, "project.capia"));
  const tOpen0 = Date.now();
  await page.getByTestId("project-open").click();
  await expect(page.getByTestId("editor")).toBeVisible();
  await page.waitForFunction(() => (window.__capiaTimeline?.stats().visibleClips ?? 0) > 0);
  const openMs = Date.now() - tOpen0;

  // ---- zoom + scroll com todos os clips: tempo de pintura
  await page.getByTestId("zoom-fit").click();
  await page.waitForTimeout(300);
  const fitStats = await page.evaluate(() => window.__capiaTimeline?.stats());
  const canvas = page.getByTestId("timeline-canvas");
  const box = await canvas.boundingBox();
  if (!box) throw new Error("sem canvas");
  await page.evaluate(() => window.__capiaTimeline?.resetSamples());
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 60; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, i < 30 ? -120 : 120);
    await page.keyboard.up("Control");
    await page.mouse.wheel(i % 2 === 0 ? 240 : -240, 0);
    await page.waitForTimeout(16);
  }
  const scrollStats = await page.evaluate(() => window.__capiaTimeline?.stats());
  const paint = stat(scrollStats?.paintSamples ?? []);

  // ---- arrasto (ghost local, sem IPC) e commit
  await page.getByTestId("zoom-fit").click();
  await page.waitForTimeout(200);
  await page.evaluate(() => {
    window.__capiaPerf?.reset();
    window.__capiaTimeline?.resetSamples();
  });
  const first = (await editor.clips()).find((c) => c.track === tracks[0] && c.name === "c0-3");
  if (!first) throw new Error("clip alvo ausente");
  // aproxima antes de arrastar (clips ficam pequenos no fit)
  await page.getByTestId("zoom-in").click({ clickCount: 4 });
  await page.waitForTimeout(300);
  let dragStat: Stat;
  let commitStat: Stat;
  // 8 arrastos/drops seguidos do mesmo clip (±amostras para p95 de gesto e de commit)
  for (let round = 0; round < 8; round++) {
    const from = await editor.clipPoint(first.id, 0.5, 0.5).catch(() => null);
    if (!from) break;
    const dir = round % 2 === 0 ? 1 : -1;
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= 30; i++) await page.mouse.move(from.x + dir * i * 2, from.y);
    await page.mouse.up();
    await expect.poll(async () => (await window_commitCount(page)) > round).toBe(true);
  }
  dragStat = stat(await page.evaluate(() => window.__capiaTimeline?.stats().gestureSamples ?? []));
  commitStat = stat((await page.evaluate(() => window.__capiaPerf?.samples("commit"))) ?? []);
  const commitRpc = stat((await page.evaluate(() => window.__capiaPerf?.samples("rpc"))) ?? []);
  const commitApply = stat((await page.evaluate(() => window.__capiaPerf?.samples("apply"))) ?? []);

  // ---- undo/redo
  await page.evaluate(() => window.__capiaPerf?.reset());
  for (let i = 0; i < 8; i++) {
    await page.getByTestId("undo").click();
    await page.waitForTimeout(40);
    await page.getByTestId("redo").click();
    await page.waitForTimeout(40);
  }
  const history = stat((await page.evaluate(() => window.__capiaPerf?.samples("history"))) ?? []);
  const historyRpc = stat((await page.evaluate(() => window.__capiaPerf?.samples("rpc"))) ?? []);
  const historyApply = stat(
    (await page.evaluate(() => window.__capiaPerf?.samples("apply"))) ?? [],
  );

  // ---- só o engine (sem navegador): separa o custo do Rust do custo da UI/WebGL em software
  const timed = async (fn: () => Promise<unknown>): Promise<number> => {
    const t0 = performance.now();
    await fn();
    return performance.now() - t0;
  };
  const engineUndo: number[] = [];
  const engineCommit: number[] = [];
  const someClip = (await editor.clips()).find((c) => c.track === tracks[1]);
  if (!someClip) throw new Error("sem clip para o teste do engine");
  for (let i = 0; i < 15; i++) {
    engineCommit.push(
      await timed(() =>
        editor.api("command.execute", {
          label: "perf opacity",
          commands: [
            {
              operation_id: `perf-eng-${String(i)}-${String(Date.now())}`,
              type: "set_property",
              clip: someClip.id,
              prop: "opacity",
              value: 0.2 + i * 0.05,
            },
          ],
        }),
      ),
    );
  }
  for (let i = 0; i < 15; i++) {
    engineUndo.push(await timed(() => editor.api("command.undo")));
    engineUndo.push(await timed(() => editor.api("command.redo")));
  }

  // ---- scrub do preview (arrasto do playhead na régua)
  await page.evaluate(() => window.__capiaPerf?.reset());
  const ob = await page.evaluate(() => window.__capiaTimeline?.canvasOrigin());
  if (!ob) throw new Error("sem origem");
  await page.mouse.move(ob.x + 100, ob.y + 12);
  await page.mouse.down();
  for (let i = 0; i < 80; i++) {
    await page.mouse.move(ob.x + 100 + i * 8, ob.y + 12);
    await page.waitForTimeout(8);
  }
  await page.mouse.up();
  await page.waitForTimeout(500);
  const preview = stat((await page.evaluate(() => window.__capiaPerf?.samples("preview"))) ?? []);
  const metricsText = await page.getByTestId("preview-metrics").textContent();

  // ---- miniaturas: busca fria (primeira vez) após reabrir
  const thumb = stat((await page.evaluate(() => window.__capiaPerf?.samples("thumb"))) ?? []);

  const report = {
    generated: new Date().toISOString(),
    env: { platform: process.platform, strict: STRICT, ci: Boolean(process.env.CI) },
    clips: 5000,
    bulkCommitMs,
    openMs,
    visibleClipsAtFit: fitStats?.visibleClips ?? 0,
    paintMsDuringZoomScroll: paint,
    dragGestureMs: dragStat,
    commitMs: commitStat,
    commitRpcMs: commitRpc,
    commitApplyMs: commitApply,
    engineOnlyCommitMs: stat(engineCommit),
    engineOnlyUndoRedoMs: stat(engineUndo),
    undoRedoMs: history,
    undoRedoRpcMs: historyRpc,
    undoRedoApplyMs: historyApply,
    previewScrubLatencyMs: preview,
    thumbnailLatencyMs: thumb,
    previewMetrics: metricsText,
    targets: {
      "scroll/zoom paint p95 <= 16.7 ms": paint.p95 <= 16.7,
      "drag gesture p95 < 16 ms": dragStat.n > 0 && dragStat.p95 < 16,
      "commit p95 < 30 ms (UI perceived)": commitStat.n > 0 && commitStat.p95 < 30,
      "commit p95 < 30 ms (engine only)": stat(engineCommit).p95 < 30,
      "undo/redo p95 < 50 ms (UI perceived)": history.n > 0 && history.p95 < 50,
      "undo/redo p95 < 50 ms (engine only)": stat(engineUndo).p95 < 50,
      "scrub preview p95 < 100 ms": preview.n > 0 && preview.p95 < 100,
      "thumbnail p95 < 200 ms": thumb.n === 0 || thumb.p95 < 200,
    },
    note: "Chromium headless sem GPU real (CPU raster): números de pintura são um piso pessimista; a medição estrita com GPU é o benchmark local (CAPIA_PERF_STRICT=1).",
  };
  const outDir = join(ROOT, "target/perf");
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "phase3-ui-perf.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  void MEDIA;

  // smoke (CI): só regressões grosseiras; estrito: as metas do documento
  if (STRICT) {
    for (const [name, ok] of Object.entries(report.targets)) expect(ok, name).toBe(true);
  } else {
    expect(paint.p95).toBeLessThan(80);
    expect(commitStat.p95).toBeLessThan(400);
    expect(history.p95).toBeLessThan(400);
    expect(preview.p95).toBeLessThan(1500);
    expect(openMs).toBeLessThan(20_000);
  }
});
