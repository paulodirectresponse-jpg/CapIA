import { test, expect } from "./fixtures";

/** Auditoria de vazamentos (PHASE3 §63): abrir/fechar projeto e trocar de sequence repetidamente. */
test("no heap or DOM growth across repeated project open/close and sequence switching", async ({
  editor,
  page,
  server,
}) => {
  test.skip(process.env.CAPIA_E2E_TARGET === "tauri", "usa flags do Chromium do Playwright");
  test.setTimeout(180_000);
  await editor.goto();
  await editor.createProject();
  await editor.importMedia("video_audio.mp4", "image.jpg");
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(2);
  const seq = await editor.sequence();
  const overlay = seq.tracks.find((t) => t.role === "overlay");
  if (!overlay) throw new Error("sem overlay");
  const F = 23_520_000;
  await editor.api("command.execute", {
    label: "seed",
    commands: Array.from({ length: 200 }, (_, i) => ({
      operation_id: `leak-${String(i)}`,
      type: "insert_clip",
      track: overlay.id,
      start: i * 40 * F,
      clip: {
        name: `c${String(i)}`,
        duration: 30 * F,
        content: { type: "solid", color: "#3366CC" },
        source_in: 0,
        speed: "1",
        reversed: false,
        properties: {},
      },
    })),
  });
  const heap = () =>
    page.evaluate(async () => {
      (window as unknown as { gc?: () => void }).gc?.();
      await new Promise((r) => setTimeout(r, 50));
      (window as unknown as { gc?: () => void }).gc?.();
      const m = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
      return {
        heap: m?.usedJSHeapSize ?? 0,
        canvases: document.querySelectorAll("canvas").length,
        nodes: document.getElementsByTagName("*").length,
      };
    });
  const cycle = async () => {
    await page.getByTestId("close-project").click();
    await expect(page.getByTestId("welcome")).toBeVisible();
    await page.getByTestId("project-path").fill(`${server.dir}/project.capia`);
    await page.getByTestId("project-open").click();
    await expect(page.getByTestId("editor")).toBeVisible();
    await page.waitForFunction(() => (window.__capiaTimeline?.stats().visibleClips ?? 0) > 0);
    // troca de painel e cria/fecha uma sequence
    await page.getByTestId("rail-media").click();
    await page.getByTestId("rail-text").click();
    await page.getByTestId("rail-media").click();
    await page.getByTestId("media-tab-sequences").click();
  };
  for (let i = 0; i < 3; i++) await cycle(); // aquece caches
  const before = await heap();
  for (let i = 0; i < 12; i++) await cycle();
  const after = await heap();
  expect(after.canvases).toBeLessThanOrEqual(before.canvases + 0);
  expect(after.nodes).toBeLessThan(before.nodes * 1.2 + 50);
  if (before.heap > 0) expect(after.heap - before.heap).toBeLessThan(30 * 1024 * 1024);
  console.log(
    `heap Δ ${String(Math.round((after.heap - before.heap) / 1024))} KiB; canvases ${String(before.canvases)}→${String(after.canvases)}; nodes ${String(before.nodes)}→${String(after.nodes)}`,
  );
});
