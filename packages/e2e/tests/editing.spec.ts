import type { Page } from "@playwright/test";
import { test, expect, type ClipJson, type Editor } from "./fixtures";

/** Cria um projeto com N clips sólidos em faixas livres (via comandos do engine, como um import). */
async function seedSolids(
  editor: Editor,
  specs: { track: string; start: number; duration: number; name: string; color?: string }[],
): Promise<void> {
  await editor.api("command.execute", {
    label: "seed",
    commands: specs.map((s, i) => ({
      operation_id: `seed-${String(i)}-${String(Date.now())}`,
      type: "insert_clip",
      track: s.track,
      start: s.start,
      clip: {
        name: s.name,
        duration: s.duration,
        content: { type: "solid", color: s.color ?? "#3366CC" },
        source_in: 0,
        speed: "1",
        reversed: false,
        properties: {},
      },
    })),
  });
}

const F = 23_520_000; // 1 frame a 30 fps
const SEC = 30 * F;

async function setup(editor: Editor): Promise<{ overlay: string; text: string; main: string }> {
  await editor.goto();
  await editor.createProject();
  const seq = await editor.sequence();
  const by = (role: string) => {
    const t = seq.tracks.find((x) => x.role === role);
    if (!t) throw new Error(`faixa ${role} ausente`);
    return t.id;
  };
  return { overlay: by("overlay"), text: by("text"), main: by("main") };
}

async function editorOriginX(page: Page): Promise<number> {
  return (await page.evaluate(() => window.__capiaTimeline?.canvasOrigin().x)) ?? 0;
}

const byName = (clips: ClipJson[], n: string): ClipJson => {
  const c = clips.find((x) => x.name === n);
  if (!c) throw new Error(`clip ${n} não encontrado`);
  return c;
};

test("selection, marquee, group, copy/paste, duplicate and ripple delete", async ({
  editor,
  page,
}) => {
  const { overlay, text } = await setup(editor);
  await seedSolids(editor, [
    { track: overlay, start: 0, duration: 2 * SEC, name: "A" },
    { track: overlay, start: 3 * SEC, duration: 2 * SEC, name: "B" },
    { track: text, start: 0, duration: 2 * SEC, name: "C" },
  ]);
  const clips = await editor.clips();
  const [a, b, c] = [byName(clips, "A"), byName(clips, "B"), byName(clips, "C")];

  await test.step("click selects, shift-click adds, ctrl+A selects all", async () => {
    const pa = await editor.clipPoint(a.id);
    await page.mouse.click(pa.x, pa.y);
    await expect(page.getByTestId("inspector-basic")).toBeVisible();
    const pb = await editor.clipPoint(b.id);
    await page.keyboard.down("Shift");
    await page.mouse.click(pb.x, pb.y);
    await page.keyboard.up("Shift");
    await expect(page.getByTestId("inspector-multi")).toContainText("2");
    await page.keyboard.press("Escape");
    await page.keyboard.press("Control+a");
    await expect(page.getByTestId("inspector-multi")).toContainText("3");
    await page.keyboard.press("Escape");
  });

  await test.step("marquee selects the clips it touches", async () => {
    const seq = await editor.sequence();
    const main = seq.tracks.find((t) => t.role === "main");
    if (!main) throw new Error("sem main");
    const pa = await editor.clipPoint(a.id, 0, 0);
    const pb = await editor.clipPoint(b.id, 1, 1);
    // começa no espaço vazio da faixa Main (abaixo e à direita de B) e sobe até A (sem tocar em C)
    const start = await editor.rowPoint(main.id, pb.x - (await editorOriginX(page)) + 30);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(pa.x - 6, pa.y + 6, { steps: 10 });
    await page.mouse.up();
    await expect(page.getByTestId("inspector-multi")).toContainText("2");
  });

  await test.step("group moves clips together and ungroup releases them", async () => {
    await page.getByTestId("group").click();
    await expect
      .poll(async () => {
        const cl = await editor.clips();
        const g = new Set(
          cl.filter((x) => (x as { group?: string | null }).group).map((x) => x.name),
        );
        return [...g].sort().join(",");
      })
      .toBe("A,B");
    const pa = await editor.clipPoint(a.id);
    await page.mouse.move(pa.x, pa.y);
    await page.mouse.down();
    await page.mouse.move(pa.x + 60, pa.y, { steps: 8 });
    await page.mouse.up();
    await expect
      .poll(async () => (await editor.clips()).find((x) => x.id === a.id)?.start)
      .toBeGreaterThan(a.start);
    const moved = await editor.clips();
    const deltaA = byName(moved, "A").start - a.start;
    const deltaB = byName(moved, "B").start - b.start;
    expect(deltaB).toBe(deltaA); // o grupo andou junto
    await page.getByTestId("ungroup").click();
    await expect
      .poll(async () => (await editor.clips()).some((x) => (x as { group?: string | null }).group))
      .toBe(false);
    await page.keyboard.press("Escape");
  });

  await test.step("copy / paste at the playhead and duplicate", async () => {
    const before = (await editor.clips()).length;
    const pc = await editor.clipPoint(c.id);
    await page.mouse.click(pc.x, pc.y);
    await page.keyboard.press("Control+c");
    // playhead em ~6 s (régua)
    const o = await page.evaluate(() => window.__capiaTimeline?.canvasOrigin());
    const pps = await page.evaluate(() => window.__capiaTimeline?.viewState().pps ?? 80);
    if (!o) throw new Error("sem canvas");
    await page.mouse.click(o.x + 6 * pps, o.y + 12);
    await page.keyboard.press("Control+v");
    await expect.poll(async () => (await editor.clips()).length).toBe(before + 1);
    await page.getByTestId("duplicate").click();
    await expect.poll(async () => (await editor.clips()).length).toBe(before + 2);
  });

  await test.step("ripple delete closes the gap on a magnetic track", async () => {
    const seq = await editor.sequence();
    const main = seq.tracks.find((t) => t.role === "main");
    if (!main) throw new Error("sem main");
    await seedSolids(editor, [
      { track: main.id, start: 0, duration: 2 * SEC, name: "M1" },
      { track: main.id, start: 2 * SEC, duration: 2 * SEC, name: "M2" },
      { track: main.id, start: 4 * SEC, duration: 2 * SEC, name: "M3" },
    ]);
    const m2 = byName(await editor.clips(), "M2");
    const p = await editor.clipPoint(m2.id);
    await page.mouse.click(p.x, p.y);
    await page.getByTestId("ripple-delete").click();
    await expect.poll(async () => byName(await editor.clips(), "M3").start).toBe(2 * SEC);
  });
});

test("tracks: lock blocks edits, mute/hide flags persist, snapping toggles", async ({
  editor,
  page,
}) => {
  const { overlay } = await setup(editor);
  await seedSolids(editor, [{ track: overlay, start: SEC, duration: 2 * SEC, name: "L" }]);
  const clip = byName(await editor.clips(), "L");

  await page.getByTestId(`lock-${overlay}`).click();
  await expect
    .poll(async () => (await editor.sequence()).tracks.find((t) => t.id === overlay)?.locked)
    .toBe(true);
  const p = await editor.clipPoint(clip.id);
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.move(p.x + 80, p.y, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  expect((await editor.clips()).find((c) => c.id === clip.id)?.start).toBe(clip.start);

  await page.getByTestId(`lock-${overlay}`).click();
  await page.getByTestId(`hide-${overlay}`).click();
  await expect
    .poll(async () =>
      (await editor.sequence()).tracks.find(
        (t) => t.id === overlay && (t as { hidden?: boolean }).hidden,
      ),
    )
    .toBeTruthy();

  const snap = page.getByTestId("snapping");
  const was = await snap.getAttribute("aria-pressed");
  await snap.click();
  await expect(snap).not.toHaveAttribute("aria-pressed", was ?? "true");
});

test("inspector: transform, animation keyframes, text style and undo of all of it", async ({
  editor,
  page,
}) => {
  const { overlay } = await setup(editor);
  await seedSolids(editor, [{ track: overlay, start: 0, duration: 4 * SEC, name: "K" }]);
  const clip = byName(await editor.clips(), "K");
  const p = await editor.clipPoint(clip.id);
  await page.mouse.click(p.x, p.y);

  const inspector = page.getByTestId("inspector");
  const posX = inspector.getByRole("textbox", { name: "Position X", exact: true });
  await posX.fill("120");
  await posX.press("Enter");
  await expect
    .poll(async () =>
      JSON.stringify((await editor.clips()).find((c) => c.id === clip.id)?.properties),
    )
    .toContain("120");

  // animação: keyframe no playhead, move o playhead, outro keyframe com valor diferente
  await inspector.getByRole("tab", { name: "Animation" }).click();
  await page.getByTestId("kf-add-position_x").click();
  await expect(page.getByTestId("kf-list-position_x").locator("li")).toHaveCount(1);
  await page.getByTestId("step-forward").click({ clickCount: 15 });
  await inspector.getByRole("tab", { name: "Basic" }).click();
  await posX.fill("300");
  await posX.press("Enter");
  await inspector.getByRole("tab", { name: "Animation" }).click();
  await expect(page.getByTestId("kf-list-position_x").locator("li")).toHaveCount(2);
  const animated = JSON.stringify((await editor.clips()).find((c) => c.id === clip.id)?.properties);
  expect(animated).toContain("animated");

  // undo desfaz o último keyframe; redo devolve
  await page.getByTestId("undo").click();
  await expect(page.getByTestId("kf-list-position_x").locator("li")).toHaveCount(1);
  await page.getByTestId("redo").click();
  await expect(page.getByTestId("kf-list-position_x").locator("li")).toHaveCount(2);
});

test("text and captions: style, edit, split and merge", async ({ editor, page }) => {
  await setup(editor);
  await page.getByTestId("rail-captions").click();
  await page.getByTestId("caption-add").click();
  await expect(page.getByTestId("caption-list").locator("li")).toHaveCount(1);
  const list = page.getByTestId("caption-list");
  await list.locator("textarea").first().fill("primeira parte");
  await list.locator("textarea").first().blur();
  await expect
    .poll(async () => (await editor.clips()).find((c) => c.content.type === "text")?.content.text)
    .toBe("primeira parte");

  // divide no meio da legenda (playhead 0 + 1 s)
  const cap = (await editor.clips()).find((c) => c.content.type === "text");
  if (!cap) throw new Error("legenda ausente");
  await page.getByTestId("go-start").click();
  await page.getByTestId("step-forward").click({ clickCount: 30 });
  await list.getByRole("button", { name: "Split caption" }).first().click();
  await expect(list.locator("li")).toHaveCount(2);
  await list.getByRole("button", { name: "Merge with next" }).first().click();
  await expect(list.locator("li")).toHaveCount(1);

  // estilo aplicado a todas as legendas
  await page.getByTestId("caption-style-highlight").click();
  await expect
    .poll(async () => JSON.stringify((await editor.clips()).find((c) => c.content.type === "text")))
    .toContain("#FFE600");
});

test("language switch (pt-BR/en), keymap remap with conflict warning and panel persistence", async ({
  editor,
  page,
}) => {
  await setup(editor);
  await page.getByTestId("settings-btn").click();
  await page.getByTestId("settings-language").selectOption("pt-BR");
  await expect(page.getByTestId("settings-dialog")).toContainText("Atalhos");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("export-btn")).toHaveText("Exportar");

  // remapeia "Dividir" para uma tecla já usada ⇒ aviso de conflito
  await page.getByTestId("settings-btn").click();
  await page.getByTestId("keymap-split").click();
  await page.keyboard.press("m"); // M = marcador
  await expect(page.getByTestId("keymap-conflicts")).toBeVisible();
  await page.getByTestId("keymap-reset-all").click();
  await expect(page.getByTestId("keymap-conflicts")).toHaveCount(0);
  await page.keyboard.press("Escape");

  // largura do painel persiste após recarregar a página
  const left = page.getByTestId("left-panel");
  const w0 = (await left.boundingBox())?.width ?? 0;
  const handle = page.getByRole("separator").first();
  const hb = await handle.boundingBox();
  if (!hb) throw new Error("sem divisor");
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(hb.x + 80, hb.y + hb.height / 2, { steps: 6 });
  await page.mouse.up();
  const w1 = (await left.boundingBox())?.width ?? 0;
  expect(w1).toBeGreaterThan(w0 + 40);
  await page.reload();
  await page.getByTestId("project-path").fill(`${editor.server.dir}/project.capia`);
  await page.getByTestId("project-open").click();
  await expect(page.getByTestId("editor")).toBeVisible();
  const w2 = (await page.getByTestId("left-panel").boundingBox())?.width ?? 0;
  expect(Math.abs(w2 - w1)).toBeLessThan(2);
  // idioma também persistiu
  await expect(page.getByTestId("export-btn")).toHaveText("Exportar");
});
