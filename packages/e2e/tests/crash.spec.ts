import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { test, expect, launchDevserver } from "./fixtures";

/** Crash/recovery smoke (PHASE3 §53): reaproveita as garantias do backend; aqui só o ponta a ponta. */

test("kill -9 after edits: reopen keeps every committed edit and the file validates", async ({
  editor,
  page,
  server,
}) => {
  test.skip(
    process.env.CAPIA_E2E_TARGET === "tauri",
    "mata o devserver; o app nativo tem o seu teste",
  );
  await editor.goto();
  await editor.createProject();
  await page.getByTestId("rail-text").click();
  for (let i = 0; i < 3; i++) await page.getByTestId("text-add-title").click();
  await expect.poll(async () => (await editor.clips()).length).toBe(3);
  const before = JSON.stringify(await editor.clips());

  server.child.kill("SIGKILL"); // sem fechar projeto nem flush: só o que já foi confirmado sobrevive
  const second = await launchDevserver();
  try {
    const call = async (m: string, p: Record<string, unknown> = {}) => {
      const r = await fetch(`${second.url}/api/${m}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(p),
      });
      expect(r.ok, `${m}: ${String(r.status)}`).toBe(true);
      return (await r.json()) as Record<string, unknown>;
    };
    const snap = (await call("project.open", { path: join(server.dir, "project.capia") })) as {
      sequences: { id: string }[];
    };
    const seq = snap.sequences[0];
    if (!seq) throw new Error("sem sequence após reabrir");
    const body = (await call("sequence.get", { sequence: seq.id })) as {
      clips: Record<string, { start: number }>;
    };
    const after = JSON.stringify(Object.values(body.clips).sort((a, b) => a.start - b.start));
    expect(Object.keys(body.clips).length).toBe(3);
    expect(after).toBe(before);
  } finally {
    second.child.kill();
  }
});

test("killing the app mid-export never publishes a partial file", async ({
  editor,
  page,
  server,
}) => {
  test.skip(process.env.CAPIA_E2E_TARGET === "tauri", "mata o devserver");
  await editor.goto();
  await editor.createProject();
  await editor.importMedia("video_audio.mp4");
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
  const asset = await editor.assetIdByName("video_audio.mp4");
  const seq = await editor.sequence();
  const main = seq.tracks.find((t) => t.role === "main");
  if (!main) throw new Error("sem main");
  // sequence longa o bastante para o export demorar: o mesmo vídeo repetido
  const frame = 23_520_000;
  await editor.api("command.execute", {
    label: "long",
    commands: Array.from({ length: 40 }, (_, i) => ({
      operation_id: `long-${String(i)}`,
      type: "insert_clip",
      track: main.id,
      start: i * 30 * frame,
      clip: {
        name: `v${String(i)}`,
        duration: 30 * frame,
        content: { type: "media", asset, has_video: true, has_audio: false },
        source_in: 0,
        speed: "1",
        reversed: false,
        properties: {},
      },
    })),
  });
  const out = join(server.dir, "mid_export");
  await page.getByTestId("export-btn").click();
  await page.getByTestId("export-preset").selectOption("intermediate");
  await page.getByRole("textbox", { name: "Width" }).fill("640");
  await page.getByRole("textbox", { name: "Width" }).press("Enter");
  await page.getByRole("textbox", { name: "Height" }).fill("360");
  await page.getByRole("textbox", { name: "Height" }).press("Enter");
  await page.getByTestId("export-path").fill(out);
  await page.getByTestId("export-start").click();
  await expect(page.getByTestId("export-badge")).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(800);
  server.child.kill("SIGKILL");
  await page.waitForTimeout(300);
  expect(existsSync(out), "o destino final não pode existir antes do rename atômico").toBe(false);
  // o diretório do projeto não deve ter nenhum arquivo parcial com o nome final
  expect(readdirSync(server.dir).filter((f) => f === "mid_export")).toEqual([]);
});

test("corrupt UI preferences never crash the app and are reported", async ({ editor, page }) => {
  await page.addInitScript(() => {
    try {
      window.localStorage.setItem("capia.prefs.v1", "{{{ not json");
    } catch {
      /* sem storage */
    }
  });
  await editor.goto();
  await editor.createProject();
  await page.getByTestId("settings-btn").click();
  await expect(page.getByTestId("settings-dialog")).toContainText("reset to defaults");
});
