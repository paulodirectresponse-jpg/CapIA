/**
 * RC3 — redesign: rail reduzida, inspector contextual, Texto/Legendas simples, sequências à vista,
 * sem dados técnicos permanentes e legível em 1366×768 e nas escalas 125%/150% do Windows
 * (o viewport CSS de 1920×1080 a 125% é 1536×864; a 150% é 1280×720; 1366×768 a 125% é 1093×614).
 */
import { expect, MEDIA, test } from "./fixtures";
import { join } from "node:path";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("capia.prefs.v1", JSON.stringify({ language: "pt-BR" }));
  });
});

test("layout enxuto: 5 itens na rail, inspector simples sem seleção e sem dados técnicos", async ({
  page,
  editor,
}) => {
  await editor.goto();
  await editor.createProject("ux", { bare: true });

  const tabs = page.getByRole("tablist", { name: "Biblioteca" }).getByRole("tab");
  await expect(tabs).toHaveText(["Mídia", "Texto", "Áudio", "Transições", "IA"]);

  // nada selecionado → inspector da sequência, só o essencial; avançado recolhido
  await expect(page.getByTestId("inspector-sequence")).toBeVisible();
  await expect(page.getByTestId("seq-name")).toBeVisible();
  await expect(page.getByTestId("inspector-advanced-sequence")).not.toHaveAttribute("open", "");

  // nenhum dado técnico permanente (fps/latência do preview) na interface normal
  await expect(page.getByTestId("preview-metrics")).toHaveCount(0);
  await expect(page.getByTestId("preview")).not.toContainText("fps");
});

test("texto e legendas: adicionar texto/título/legenda, editar, entrada pronta e desfazer", async ({
  page,
  editor,
}) => {
  test.setTimeout(120_000);
  await editor.goto();
  await editor.createProject("txt", { bare: true });
  await editor.importAbsolute(join(MEDIA, "video_only.mp4"));
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
  await editor.dragAssetTo(
    await editor.assetIdByName("video_only.mp4"),
    await editor.emptyPoint(200),
  );
  await expect.poll(async () => (await editor.clips()).length).toBe(1);

  await page.getByTestId("rail-text").click();
  await expect(page.getByRole("button", { name: "Adicionar texto" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Adicionar título" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Adicionar legenda" })).toBeVisible();

  await page.getByRole("button", { name: "Adicionar texto" }).click();
  await expect
    .poll(async () => (await editor.clips()).filter((c) => c.content.type === "text").length)
    .toBe(1);
  await page.getByRole("button", { name: "Adicionar título" }).click();
  await page.getByRole("button", { name: "Adicionar legenda" }).click();
  await expect
    .poll(async () => (await editor.clips()).filter((c) => c.content.type === "text").length)
    .toBe(3);

  // seleciona o primeiro texto: conteúdo + estilo + posição + animação à mão; avançado recolhido
  const text = (await editor.clips()).find((c) => c.content.type === "text");
  if (!text) throw new Error("sem clip de texto");
  const p = await editor.clipPoint(text.id);
  await page.mouse.click(p.x, p.y);
  const area = page.getByTestId("clip-text");
  await expect(area).toBeVisible();
  await area.fill("Oferta só hoje");
  await area.blur();
  await expect
    .poll(async () => (await editor.clips()).find((c) => c.id === text.id)?.content.text)
    .toBe("Oferta só hoje");
  await expect(page.getByTestId("inspector-advanced-clip")).not.toHaveAttribute("open", "");
  await expect(page.getByRole("textbox", { name: "Posição X", exact: true })).toBeVisible();

  await page.getByTestId("text-entrance-fade").click();
  await expect
    .poll(async () => {
      const o = (await editor.clips()).find((c) => c.id === text.id)?.properties.opacity;
      return JSON.stringify(o ?? null).includes("animated");
    })
    .toBe(true);
  await page.getByTestId("undo").click();
  await expect
    .poll(async () => {
      const o = (await editor.clips()).find((c) => c.id === text.id)?.properties.opacity;
      return JSON.stringify(o ?? null).includes("animated");
    })
    .toBe(false);
  await expect(page.getByTestId("error-boundary")).toHaveCount(0);
});

test("sequências à vista: lista, + Nova sequência, arrastar para usar, dois cliques entra, migalhas voltam", async ({
  page,
  editor,
}) => {
  test.setTimeout(120_000);
  await editor.goto();
  await editor.createProject("seq", { bare: true });
  await editor.importAbsolute(join(MEDIA, "video_only.mp4"));
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
  await editor.dragAssetTo(
    await editor.assetIdByName("video_only.mp4"),
    await editor.emptyPoint(200),
  );
  await expect.poll(async () => (await editor.clips()).length).toBe(1);

  await page.getByTestId("rail-media").click();
  await page.getByTestId("media-tab-sequences").click();
  await expect(page.getByTestId("project-panel")).toBeVisible();
  await expect(page.getByTestId("sequence-hint")).toContainText("Arraste uma sequência");
  await expect(page.getByTestId("project-panel")).not.toContainText(/nested|aninhad/i);
  await page.getByTestId("new-sequence").click();
  await expect.poll(async () => (await editor.snapshot()).sequences.length).toBe(2);
  const [first, second] = (await editor.snapshot()).sequences;
  if (!first || !second) throw new Error("faltam sequências");

  const o = await page.evaluate(() => window.__capiaTimeline?.canvasOrigin());
  if (!o) throw new Error("sem timeline");
  const box = await page.getByTestId(`seq-${first.id}`).boundingBox();
  if (!box) throw new Error("sem caixa");
  await page.mouse.move(box.x + 20, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(o.x + 160, o.y + 90, { steps: 12 });
  await page.mouse.up();
  await expect
    .poll(async () => (await editor.clips(second.id)).some((c) => c.content.type === "nested"))
    .toBe(true);
  const used = (await editor.clips(second.id)).find((c) => c.content.type === "nested");
  if (!used) throw new Error("sequência usada ausente");
  const pt = await editor.clipPoint(used.id, 0.1, 0.5);
  await page.mouse.dblclick(pt.x, pt.y);
  await expect(page.getByTestId("breadcrumb")).toBeVisible();
  await page.getByTestId("breadcrumb").getByRole("button").first().click();
  await expect(page.getByTestId("error-boundary")).toHaveCount(0);
});

const SIZES = [
  { name: "1366x768", w: 1366, h: 768 },
  { name: "125pct-1536x864", w: 1536, h: 864 },
  { name: "150pct-1280x720", w: 1280, h: 720 },
  { name: "1366-at-125pct-1093x614", w: 1093, h: 614 },
] as const;

for (const s of SIZES) {
  test(`legível sem rolagem da página em ${s.name}`, async ({ page, editor }) => {
    await page.setViewportSize({ width: s.w, height: s.h });
    await editor.goto();
    await editor.createProject("scale", { bare: true });
    await editor.importAbsolute(join(MEDIA, "video_only.mp4"));
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
    await editor.dragAssetTo(
      await editor.assetIdByName("video_only.mp4"),
      await editor.emptyPoint(200),
    );
    await expect.poll(async () => (await editor.clips()).length).toBe(1);

    const over = await page.evaluate(() => ({
      sw: document.documentElement.scrollWidth,
      sh: document.documentElement.scrollHeight,
      iw: window.innerWidth,
      ih: window.innerHeight,
    }));
    expect(over.sw).toBeLessThanOrEqual(over.iw);
    expect(over.sh).toBeLessThanOrEqual(over.ih + 1); // arredondamento subpixel (body é overflow:hidden)
    const root = await page.getByTestId("editor").boundingBox();
    expect((root?.height ?? 0) + (root?.y ?? 0)).toBeLessThanOrEqual(over.ih + 1);
    for (const id of [
      "export-btn",
      "undo",
      "redo",
      "rail-media",
      "rail-ai",
      "timeline",
      "preview",
    ]) {
      const loc = page.getByTestId(id);
      if ((await loc.count()) === 0) continue;
      const b = await loc.first().boundingBox();
      expect(b, id).not.toBeNull();
      if (b) {
        expect(b.x, id).toBeGreaterThanOrEqual(0);
        expect(b.y, id).toBeGreaterThanOrEqual(0);
        expect(b.x + b.width, id).toBeLessThanOrEqual(s.w + 1);
        expect(b.y + b.height, id).toBeLessThanOrEqual(s.h + 1);
      }
    }
    // a timeline mantém altura útil e o inspector aparece
    const tl = await page.getByTestId("timeline").boundingBox();
    expect(tl?.height ?? 0).toBeGreaterThanOrEqual(120);
    await expect(page.getByTestId("inspector")).toBeVisible();
    await page.screenshot({ path: `../../target/e2e-results/rc3-scale-${s.name}.png` });
  });
}
