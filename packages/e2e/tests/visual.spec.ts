import { copyFileSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { test, expect, MEDIA, ROOT } from "./fixtures";

/**
 * Smoke visual (PHASE3 §52): poucas capturas-chave + sondas de pixel (não é golden frágil).
 * As imagens vão para `target/e2e-visual/` (artefato do CI) para revisão humana.
 */
const OUT = join(ROOT, "target/e2e-visual");

async function shot(page: Page, name: string): Promise<void> {
  mkdirSync(OUT, { recursive: true });
  await page.screenshot({ path: join(OUT, `${name}.png`) });
}

/** Cores distintas amostradas num <canvas> (quadro em branco ⇒ 1). */
async function distinctColors(page: Page, testId: string): Promise<number> {
  return page.evaluate((id) => {
    const src = document.querySelector<HTMLCanvasElement>(`[data-testid="${id}"]`);
    if (!src) return 0;
    const c = document.createElement("canvas");
    c.width = src.width;
    c.height = src.height;
    const ctx = c.getContext("2d");
    if (!ctx) return 0;
    ctx.drawImage(src, 0, 0);
    const d = ctx.getImageData(0, 0, c.width, c.height).data;
    const seen = new Set<number>();
    for (let i = 0; i < d.length; i += 4 * 7) {
      seen.add(((d[i] ?? 0) << 16) | ((d[i + 1] ?? 0) << 8) | (d[i + 2] ?? 0));
    }
    return seen.size;
  }, testId);
}

async function centerPixel(page: Page, testId: string): Promise<[number, number, number]> {
  return page.evaluate((id) => {
    const src = document.querySelector<HTMLCanvasElement>(`[data-testid="${id}"]`);
    const c = document.createElement("canvas");
    if (!src) return [0, 0, 0];
    c.width = src.width;
    c.height = src.height;
    const ctx = c.getContext("2d");
    if (!ctx) return [0, 0, 0];
    ctx.drawImage(src, 0, 0);
    const d = ctx.getImageData(Math.floor(c.width / 2), Math.floor(c.height / 2), 1, 1).data;
    return [d[0] ?? 0, d[1] ?? 0, d[2] ?? 0];
  }, testId);
}

test("visual smoke: shell, timeline with clips, preview, offline, inspector, export dialog", async ({
  editor,
  page,
  server,
}) => {
  await editor.goto();
  await editor.createProject();
  await shot(page, "1-shell-empty");
  await expect(page.getByTestId("preview-canvas")).toBeVisible();

  const seq = await editor.sequence();
  const overlay = seq.tracks.find((t) => t.role === "overlay");
  const text = seq.tracks.find((t) => t.role === "text");
  if (!overlay || !text) throw new Error("faixas");
  const F = 23_520_000;
  await editor.api("command.execute", {
    label: "visual",
    commands: [
      ["#FF0000", 0, "red"],
      ["#00AA00", 70, "green"],
      ["#0000FF", 140, "blue"],
    ].map(([color, start, name], i) => ({
      operation_id: `v${String(i)}`,
      type: "insert_clip",
      track: overlay.id,
      start: Number(start) * F,
      clip: {
        name,
        duration: 60 * F,
        content: { type: "solid", color },
        source_in: 0,
        speed: "1",
        reversed: false,
        properties: {},
      },
    })),
  });
  const red = (await editor.clips()).find((c) => c.name === "red");
  if (!red) throw new Error("clip");
  await editor.clipPoint(red.id); // espera a UI receber o evento
  await page.waitForTimeout(600);

  // timeline com clips: o canvas tem várias cores (régua + faixas + 3 clips)
  expect(await distinctColors(page, "timeline-canvas")).toBeGreaterThan(5);
  // preview no playhead 0: o clip vermelho cobre o quadro
  await expect
    .poll(async () => (await centerPixel(page, "preview-canvas"))[0], { timeout: 8000 })
    .toBeGreaterThan(200);
  const [r, g, b] = await centerPixel(page, "preview-canvas");
  expect(g).toBeLessThan(40);
  expect(b).toBeLessThan(40);
  await shot(page, "2-timeline-with-clips");

  // seleção + inspector
  const p = await editor.clipPoint(red.id);
  await page.mouse.click(p.x, p.y);
  await expect(page.getByTestId("inspector-basic")).toBeVisible();
  await shot(page, "3-inspector");

  // mídia offline
  const copy = join(server.dir, "will_vanish.wav");
  copyFileSync(join(MEDIA, "audio.wav"), copy);
  await editor.openImportAfterRail(copy);
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
  rmSync(copy);
  await expect
    .poll(async () => (await editor.assets())[0]?.status, { timeout: 15_000 })
    .toBe("offline");
  await page.getByTestId("rail-media").click();
  await expect(page.getByTestId("media-panel")).toContainText("offline", { ignoreCase: true });
  await shot(page, "4-offline-media");

  // diálogo de exportação
  await page.getByTestId("export-btn").click();
  await expect(page.getByTestId("export-dialog")).toBeVisible();
  await shot(page, "5-export-dialog");
  expect(r).toBeGreaterThan(200);
});
