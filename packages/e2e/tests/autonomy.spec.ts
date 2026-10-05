import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, expect, launchDevserver, Editor } from "./fixtures";

/**
 * Autonomia (Fase 5) de ponta a ponta: UI real + engine real + FFmpeg real + "cérebro" Replay
 * determinístico (`CAPIA_AI_DEMO_BRAIN`, só no devserver; sem rede, sem chave). Não roda no alvo
 * Tauri (o app real nunca carrega o cérebro de demonstração).
 */
test.skip(process.env.CAPIA_E2E_TARGET === "tauri", "o cérebro demo só existe no devserver");

test("brief + bruto → plano aprovado → timeline editável → undo seletivo da Run", async ({
  page,
}) => {
  const dir = mkdtempSync(join(tmpdir(), "capia-auto-e2e-"));
  const { url, child } = await launchDevserver({ CAPIA_AI_DEMO_BRAIN: "1" });
  try {
    const editor = new Editor(page, { url, dir, child });
    await editor.goto();
    await editor.createProject();
    await editor.importMedia("video_audio.mp4");
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
    const before = (await editor.snapshot()).sequences.length;

    await page.getByTestId("rail-ai").click();
    await page.getByRole("tab", { name: /Runs|Execuções/ }).click();
    await expect(page.getByTestId("ai-runs-panel")).toBeVisible();
    await page.getByTestId("ai-run-brief").fill("Produto: Demo. Anúncio curto com CTA.");
    await expect(page.getByTestId("ai-run-start")).toBeEnabled();
    await page.getByTestId("ai-run-start").click();

    // 1) o plano espera a aprovação humana; nada foi escrito na timeline
    await expect(page.getByTestId("ai-run-decision")).toBeVisible({ timeout: 60_000 });
    expect((await editor.snapshot()).sequences.length).toBe(before);
    const hist0 = await editor.api<{ entries: { actor: { id: string } }[] }>("history.list");
    expect(hist0.entries.some((e) => e.actor.id.startsWith("run:"))).toBe(false);

    // 2) aprovar → editar → revisar → concluir
    // a Run pode pedir mais de uma decisão (spec do briefing, plano): aprova cada uma
    for (let i = 0; i < 4; i++) {
      if (await page.getByTestId("ai-run-sequences").isVisible()) break;
      const approve = page.getByTestId("ai-run-option-approve");
      if (await approve.isVisible()) await approve.click();
      await page.waitForTimeout(700);
    }
    await expect(page.getByTestId("ai-run-sequences")).toBeVisible({ timeout: 60_000 });
    await expect(page.getByTestId("ai-run-progress")).toBeVisible();

    // 3) a timeline é comum: sequence nova, clips reais, atribuída à Run, editável à mão
    await expect.poll(async () => (await editor.snapshot()).sequences.length).toBe(before + 1);
    const hist = await editor.api<{ entries: { actor: { id: string; kind: string } }[] }>(
      "history.list",
    );
    expect(
      hist.entries.some((e) => e.actor.kind === "agent" && e.actor.id.startsWith("run:")),
    ).toBe(true);

    // 4) desfazer só o que a Run fez (nova entrada de histórico); o projeto volta ao que era
    await page.getByTestId("ai-run-undo").click();
    await expect.poll(async () => (await editor.snapshot()).sequences.length).toBe(before);
    const after = await editor.api<{ entries: unknown[] }>("history.list");
    expect(after.entries.length).toBeGreaterThan(hist.entries.length);
  } finally {
    child.kill();
  }
});

test("a geração por IA e as fontes começam desligadas; nenhuma chamada externa", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  const dir = mkdtempSync(join(tmpdir(), "capia-auto-off-"));
  const { url, child } = await launchDevserver({ CAPIA_AI_DEMO_BRAIN: "1" });
  try {
    const editor = new Editor(page, { url, dir, child });
    await editor.goto();
    await editor.createProject();
    await page.getByTestId("rail-ai").click();
    await page.getByRole("tab", { name: /Runs|Execuções/ }).click();
    await page.getByRole("tab", { name: /Sources|Fontes/ }).click();
    await expect(page.getByTestId("ai-sources")).toBeVisible();
    await expect(page.getByTestId("ai-generation-toggle")).toHaveText(/Off|Desligada/);
    const foreign = requests.filter(
      (u) => !u.startsWith(url) && !u.startsWith("data:") && !u.startsWith("blob:"),
    );
    expect(foreign).toEqual([]);
  } finally {
    child.kill();
  }
});
