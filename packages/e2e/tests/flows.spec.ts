import { copyFileSync, existsSync, rmSync } from "node:fs";
import { join } from "node:path";
import { test, expect, MEDIA } from "./fixtures";

/**
 * Os 17 fluxos críticos da Fase 3 (PHASE3 §51), com o **engine real**. Cada passo confere a
 * verdade persistida pelo engine (`sequence.get`), não só o que a UI desenha.
 */
test("17 critical editor flows", async ({ editor, page, server }) => {
  test.setTimeout(240_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));

  await test.step("1. launch app", async () => {
    await editor.goto();
    await expect(page.getByTestId("welcome")).toBeVisible();
  });

  await test.step("2. create project", async () => {
    await editor.createProject();
    const seq = await editor.sequence();
    expect(seq.tracks.length).toBeGreaterThanOrEqual(4);
    await expect(page.getByTestId("project-name")).toHaveText("project.capia");
  });

  await test.step("3. import media", async () => {
    await editor.importMedia("video_audio.mp4", "image.jpg", "tone_44k.wav");
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(3);
    await expect(page.locator("[data-testid^=asset-]")).toHaveCount(3);
  });

  const seq0 = await editor.sequence();
  const main = seq0.tracks.find((t) => t.role === "main");
  const overlay = seq0.tracks.find((t) => t.role === "overlay");
  if (!main || !overlay) throw new Error("tracks padrão ausentes");
  const video = await editor.assetIdByName("video_audio.mp4");
  const image = await editor.assetIdByName("image.jpg");

  await test.step("3b. preview transport (P2 SharedBuffer no app real)", async () => {
    const transport = await page.getByTestId("preview-canvas").getAttribute("data-transport");
    if (process.env.CAPIA_REQUIRE_SHARED_BUFFER === "1") expect(transport).toBe("shared-buffer");
    else expect(["ipc", "shared-buffer"]).toContain(transport);
  });

  await test.step("4. drag to timeline", async () => {
    const to = await editor.rowPoint(main.id, 120);
    await editor.dragAssetTo(video, to);
    await expect.poll(async () => (await editor.clips()).length).toBeGreaterThanOrEqual(1);
    const clips = await editor.clips();
    expect(clips.some((c) => c.content.type === "media" && c.track === main.id)).toBe(true);
  });

  const videoClip = (await editor.clips()).find((c) => c.track === main.id);
  if (!videoClip) throw new Error("clip de vídeo não criado");

  await test.step("5. trim", async () => {
    const before = videoClip.duration;
    const right = await editor.clipPoint(videoClip.id, 1, 0.5);
    await page.mouse.move(right.x - 2, right.y);
    await page.mouse.down();
    await page.mouse.move(right.x - 62, right.y, { steps: 8 });
    await page.mouse.up();
    await expect
      .poll(async () => (await editor.clips()).find((c) => c.id === videoClip.id)?.duration)
      .toBeLessThan(before);
  });

  await test.step("6. split", async () => {
    const c = (await editor.clips()).find((x) => x.id === videoClip.id);
    if (!c) throw new Error("clip sumiu");
    const mid = await editor.clipPoint(videoClip.id, 0.5, 0.5);
    // clique na régua (topo do canvas) posiciona o playhead
    const origin = await page.evaluate(() => window.__capiaTimeline?.canvasOrigin());
    if (!origin) throw new Error("sem canvas");
    await page.mouse.click(mid.x, origin.y + 12);
    await page.getByTestId("split").click();
    await expect
      .poll(async () => (await editor.clips()).filter((x) => x.track === main.id).length)
      .toBe(2);
  });

  await test.step("7. move", async () => {
    const to = await editor.rowPoint(overlay.id, 300);
    await editor.dragAssetTo(image, to);
    await expect
      .poll(async () => (await editor.clips()).some((c) => c.track === overlay.id))
      .toBe(true);
    const img = (await editor.clips()).find((c) => c.track === overlay.id);
    if (!img) throw new Error("imagem não colocada");
    const from = await editor.clipPoint(img.id, 0.5, 0.5);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x + 90, from.y, { steps: 8 });
    await page.mouse.up();
    await expect
      .poll(async () => (await editor.clips()).find((c) => c.id === img.id)?.start)
      .toBeGreaterThan(img.start);
  });

  await test.step("8. undo / redo", async () => {
    const moved = (await editor.clips()).find((c) => c.track === overlay.id);
    if (!moved) throw new Error("sem clip na overlay");
    await page.getByTestId("undo").click();
    await expect
      .poll(async () => (await editor.clips()).find((c) => c.id === moved.id)?.start)
      .toBeLessThan(moved.start);
    await page.getByTestId("redo").click();
    await expect
      .poll(async () => (await editor.clips()).find((c) => c.id === moved.id)?.start)
      .toBe(moved.start);
    // histórico visível
    await page.getByTestId("history-btn").click();
    await expect(page.getByTestId("history-list").locator("li").first()).toBeVisible();
    await page.keyboard.press("Escape");
  });

  await test.step("9. add text", async () => {
    await page.getByTestId("rail-text").click();
    await page.getByTestId("text-add-title").click();
    await expect
      .poll(async () => (await editor.clips()).some((c) => c.content.type === "text"))
      .toBe(true);
  });

  await test.step("10. add caption", async () => {
    await page.getByTestId("rail-captions").click();
    await page.getByTestId("caption-add").click();
    await expect(page.getByTestId("caption-list").locator("li")).toHaveCount(1);
    const ta = page.getByTestId("caption-list").locator("textarea").first();
    await ta.fill("Olá, mundo!");
    await ta.blur();
    await expect
      .poll(async () =>
        (await editor.clips()).some(
          (c) => c.content.type === "text" && c.content.text === "Olá, mundo!",
        ),
      )
      .toBe(true);
  });

  await test.step("11. add transition", async () => {
    const [a, b] = (await editor.clips()).filter((c) => c.track === main.id);
    if (!a || !b) throw new Error("faltam clips adjacentes");
    const p = await editor.clipPoint(b.id, 0.5, 0.5);
    await page.mouse.click(p.x, p.y);
    await page.getByTestId("rail-transitions").click();
    await page.getByTestId("transition-dissolve").click();
    await expect
      .poll(async () => (await editor.clips()).find((c) => c.id === b.id)?.transition_in?.kind)
      .toBe("dissolve");
  });

  await test.step("12. edit audio", async () => {
    const target = (await editor.clips()).find(
      (c) => c.track === main.id && c.content.type === "media" && c.content.has_audio,
    );
    if (!target) throw new Error("clip de vídeo com áudio ausente");
    const p = await editor.clipPoint(target.id, 0.5, 0.5);
    await page.mouse.click(p.x, p.y);
    await page.getByTestId("inspector").getByRole("tab", { name: "Audio" }).click();
    const vol = page.getByRole("textbox", { name: "Volume (dB) (dB)" });
    await vol.fill("-6");
    await vol.press("Enter");
    await expect
      .poll(async () =>
        JSON.stringify((await editor.clips()).find((c) => c.id === target.id)?.properties),
      )
      .toContain("-6");
    const fade = page.getByRole("textbox", { name: "Fade in (s)" });
    await fade.fill("0.1");
    await fade.press("Enter");
    await expect
      .poll(async () =>
        JSON.stringify((await editor.clips()).find((c) => c.id === target.id)?.properties),
      )
      .toContain("fade_in");
    // separar o áudio cria um clip de áudio numa faixa de áudio
    await page.getByTestId("detach-audio").first().click();
    await expect
      .poll(async () => {
        const sq = await editor.sequence();
        const audioTracks = new Set(sq.tracks.filter((t) => t.kind === "audio").map((t) => t.id));
        return Object.values(sq.clips).some((c) => audioTracks.has(c.track));
      })
      .toBe(true);
  });

  await test.step("13. nested sequence navigation", async () => {
    await page.getByTestId("rail-project").click();
    await page.getByTestId("new-sequence").click();
    await expect.poll(async () => (await editor.snapshot()).sequences.length).toBe(2);
    const snap = await editor.snapshot();
    const first = snap.sequences[0];
    const second = snap.sequences[1];
    if (!first || !second) throw new Error("faltam sequences");
    // arrasta a primeira sequence para a timeline da nova (nested)
    const secondMain = (await editor.sequence(second.id)).tracks.find((t) => t.role === "main");
    if (!secondMain) throw new Error("sem main");
    const to = await editor.rowPoint(secondMain.id, 140);
    const src = page.getByTestId(`seq-${first.id}`);
    const box = await src.boundingBox();
    if (!box) throw new Error("sem caixa da sequence");
    await page.mouse.move(box.x + 20, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(to.x, to.y, { steps: 12 });
    await page.mouse.up();
    await expect
      .poll(async () => (await editor.clips(second.id)).some((c) => c.content.type === "nested"))
      .toBe(true);
    const nested = (await editor.clips(second.id)).find((c) => c.content.type === "nested");
    if (!nested) throw new Error("nested ausente");
    const p = await editor.clipPoint(nested.id, 0.5, 0.5);
    await page.mouse.dblclick(p.x, p.y);
    await expect(page.getByTestId("breadcrumb")).toBeVisible();
    // volta à primeira sequence
    await page.getByTestId(`seq-${first.id}`).dblclick();
  });

  await test.step("14. offline / relink", async () => {
    const copy = join(server.dir, "tone_copy.wav");
    copyFileSync(join(MEDIA, "audio.wav"), copy);
    await page.getByTestId("rail-media").click();
    await editor.openImportByPath();
    await page.getByTestId("import-paths").fill(copy);
    await page.getByTestId("import-confirm").click();
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(4);
    rmSync(copy);
    const id = await editor.assetIdByName("tone_copy.wav");
    await expect
      .poll(async () => (await editor.assets()).find((a) => a.id === id)?.status)
      .toBe("offline");
    // UI reflete offline e oferece relink
    await page.getByTestId("media-filter").selectOption("audio");
    await page.getByTestId(`relink-${id}`).click();
    copyFileSync(join(MEDIA, "audio.wav"), copy);
    await page.getByTestId("relink-path").fill(copy);
    await page.getByTestId("relink-confirm").click();
    await expect
      .poll(async () => (await editor.assets()).find((a) => a.id === id)?.status)
      .toBe("online");
    expect(existsSync(copy)).toBe(true);
  });

  await test.step("15. export", async () => {
    await page.getByTestId("export-btn").click();
    await expect(page.getByTestId("export-dialog")).toBeVisible();
    const caps = await editor.api<{ available: boolean; codec: string }[]>("export.encoders");
    const h264 = caps.some((c) => c.available && c.codec.toLowerCase().includes("264"));
    if (process.env.CAPIA_REQUIRE_H264 === "1") expect(h264, "H.264 aprovado exigido").toBe(true);
    if (h264) {
      await page.getByRole("textbox", { name: "Width" }).fill("270");
      await page.getByRole("textbox", { name: "Width" }).press("Enter");
      await page.getByRole("textbox", { name: "Height" }).fill("480");
      await page.getByRole("textbox", { name: "Height" }).press("Enter");
      await page.getByTestId("export-path").fill(join(server.dir, "out.mp4"));
      await page.getByTestId("export-start").click();
      await expect(page.getByTestId("export-report")).toBeVisible({ timeout: 120_000 });
      // codec real reportado pelo ffprobe pós-export (nunca x264/x265: o catálogo os proíbe)
      await expect(page.getByTestId("export-report")).toContainText("h264");
      await expect(page.getByTestId("export-report")).not.toContainText("libx26");
      expect(existsSync(join(server.dir, "out.mp4"))).toBe(true);
    } else {
      await expect(page.getByTestId("export-no-encoder")).toBeVisible();
      await page.getByTestId("export-preset").selectOption("intermediate");
      await page.getByRole("textbox", { name: "Width" }).fill("270");
      await page.getByRole("textbox", { name: "Width" }).press("Enter");
      await page.getByRole("textbox", { name: "Height" }).fill("480");
      await page.getByRole("textbox", { name: "Height" }).press("Enter");
      await page.getByTestId("export-path").fill(join(server.dir, "out_intermediate"));
      await page.getByTestId("export-start").click();
      await expect(page.getByTestId("export-report")).toBeVisible({ timeout: 120_000 });
    }
    await page.keyboard.press("Escape");
  });

  const before = JSON.stringify(
    (await editor.clips()).map((c) => ({ ...c, id: undefined })).sort((a, b) => a.start - b.start),
  );

  await test.step("16. reopen project", async () => {
    await page.getByTestId("close-project").click();
    await expect(page.getByTestId("welcome")).toBeVisible();
    await page.getByTestId("project-path").fill(join(server.dir, "project.capia"));
    await page.getByTestId("project-open").click();
    await expect(page.getByTestId("editor")).toBeVisible();
    await page.getByTestId("rail-media").click();
    await expect(page.locator("[data-testid^=asset-]").first()).toBeVisible({ timeout: 15_000 });
  });

  await test.step("17. validate persisted result", async () => {
    const after = JSON.stringify(
      (await editor.clips())
        .map((c) => ({ ...c, id: undefined }))
        .sort((a, b) => a.start - b.start),
    );
    expect(after).toBe(before);
    expect(errors).toEqual([]);
  });
});
