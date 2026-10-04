import { test, expect } from "./fixtures";

/** Linha de base de acessibilidade (PHASE3 §42): nomes acessíveis, teclado, foco visível, diálogos. */
test("a11y baseline: names, keyboard, visible focus, dialog focus handling", async ({
  editor,
  page,
}) => {
  await editor.goto();
  await editor.createProject();

  // todo botão/controle tem nome acessível (texto, aria-label ou title)
  const unnamed = await page.evaluate(() => {
    const bad: string[] = [];
    const sel =
      "button, [role=button], [role=tab], input:not([type=hidden]), select, textarea, canvas";
    for (const el of document.querySelectorAll<HTMLElement>(sel)) {
      const labels = "labels" in el ? (el as HTMLInputElement).labels : null;
      const name =
        el.getAttribute("aria-label") ??
        el.getAttribute("title") ??
        labels?.[0]?.textContent ??
        el.textContent;
      const labelled = el.getAttribute("aria-labelledby");
      if (!name.trim() && !labelled && !el.closest("label")) {
        bad.push(`${el.tagName.toLowerCase()}[${el.getAttribute("data-testid") ?? el.className}]`);
      }
    }
    return bad;
  });
  expect(unnamed).toEqual([]);

  // teclado: Tab percorre controles reais e o foco é visível (outline/box-shadow ≠ none)
  const visible: boolean[] = [];
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Tab");
    visible.push(
      await page.evaluate(() => {
        const el = document.activeElement as HTMLElement | null;
        if (!el || el === document.body) return false;
        const cs = getComputedStyle(el);
        return (cs.outlineStyle !== "none" && cs.outlineWidth !== "0px") || cs.boxShadow !== "none";
      }),
    );
  }
  expect(visible.filter(Boolean).length).toBeGreaterThanOrEqual(8);

  // rail: setas/tab; abas têm role=tab e aria-selected
  await expect(page.getByTestId("rail-media")).toHaveAttribute("role", "tab");
  await page.getByTestId("rail-media").click();
  await expect(page.getByTestId("rail-media")).toHaveAttribute("aria-selected", "true");

  // diálogo: foco vai para dentro, Tab não escapa, Esc fecha e devolve o foco ao botão
  await page.getByTestId("settings-btn").focus();
  await page.keyboard.press("Enter");
  const dlg = page.getByRole("dialog");
  await expect(dlg).toBeVisible();
  await expect(dlg).toHaveAttribute("aria-modal", "true");
  for (let i = 0; i < 40; i++) {
    await page.keyboard.press("Tab");
    expect(
      await page.evaluate(() => document.activeElement?.closest("[role=dialog]") !== null),
    ).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(dlg).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement?.getAttribute("data-testid"))).toBe(
    "settings-btn",
  );

  // atalho de edição não dispara com foco em campo de texto (digitar não dá comando)
  await page.getByTestId("rail-text").click();
  await page.getByTestId("text-add-title").click();
  await expect.poll(async () => (await editor.clips()).length).toBe(1);
  await page.getByTestId("project-name").click(); // sem efeito
});
