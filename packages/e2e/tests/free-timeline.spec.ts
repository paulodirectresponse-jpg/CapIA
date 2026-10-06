/**
 * RC3 — timeline LIVRE, só pela interface (sem API para montar o cenário principal).
 *
 * Projeto novo sem nenhuma track → 3 vídeos + 4 áudios arrastados da biblioteca. Soltar no espaço
 * vazio cria uma track do tipo certo; clips coexistem livremente; mover entre tracks (inclusive para
 * o vazio); renomear, reordenar, travar, ocultar, mudo/solo, excluir track vazia; fechar/reabrir;
 * exportar. A API do engine só é usada para LER a verdade persistida.
 */
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, MEDIA, test, type Editor } from "./fixtures";

function wav(path: string, seconds: number, hz: number): void {
  const rate = 8000;
  const n = rate * seconds;
  const buf = Buffer.alloc(44 + n * 2);
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + n * 2, 4);
  buf.write("WAVEfmt ", 8);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(rate, 24);
  buf.writeUInt32LE(rate * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++)
    buf.writeInt16LE(Math.round(Math.sin((2 * Math.PI * hz * i) / rate) * 8000), 44 + i * 2);
  writeFileSync(path, buf);
}


async function structure(editor: Editor): Promise<{ kind: string; clips: number }[]> {
  const seq = await editor.sequence();
  return seq.tracks.map((t) => ({
    kind: t.kind,
    clips: Object.values(seq.clips).filter((c) => c.track === t.id).length,
  }));
}

test("timeline livre: 3 vídeos + 4 áudios, tracks criadas ao soltar, mover, reordenar, reabrir, exportar", async ({
  page,
  editor,
  server,
}) => {
  test.setTimeout(300_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.addInitScript(() => {
    localStorage.setItem(
      "capia.prefs.v1",
      JSON.stringify({ language: "pt-BR", panels: { timelineHeight: 560 } }),
    );
  });
  const a3 = join(server.dir, "tom_a3.wav");
  const a4 = join(server.dir, "tom_a4.wav");
  wav(a3, 1, 330);
  wav(a4, 1, 550);

  await editor.goto();
  await editor.createProject("livre", { bare: true });

  await test.step("a timeline começa simples, sem nenhuma track fixa", async () => {
    expect((await editor.sequence()).tracks).toHaveLength(0);
    await expect(page.getByTestId("timeline-empty-free")).toBeVisible();
  });

  await test.step("importa 3 vídeos + 4 áudios", async () => {
    await editor.importAbsolute(
      join(MEDIA, "video_only.mp4"),
      join(MEDIA, "vfr.mp4"),
      join(MEDIA, "cfr_gop.mp4"),
      join(MEDIA, "audio.wav"),
      join(MEDIA, "tone_44k.wav"),
      a3,
      a4,
    );
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(7);
  });

  const id = async (n: string) => editor.assetIdByName(n);
  const drop = async (name: string, x: number, at?: { track: string }) => {
    const to = at ? await editor.rowPoint(at.track, x) : await editor.emptyPoint(x);
    await editor.dragAssetTo(await id(name), to);
  };

  await test.step("vídeo 1 solto na área vazia vira a primeira camada", async () => {
    await drop("video_only.mp4", 200);
    await expect.poll(() => structure(editor)).toEqual([{ kind: "visual", clips: 1 }]);
  });
  await test.step("vídeo 2 solto no vazio cria uma NOVA track acima", async () => {
    await drop("vfr.mp4", 200);
    await expect
      .poll(() => structure(editor))
      .toEqual([
        { kind: "visual", clips: 1 },
        { kind: "visual", clips: 1 },
      ]);
  });
  await test.step("vídeo 3 cria outra track", async () => {
    await drop("cfr_gop.mp4", 200);
    await expect.poll(async () => (await structure(editor)).length).toBe(3);
  });
  await test.step("áudio 1 e 2 criam tracks de áudio", async () => {
    await drop("audio.wav", 200);
    await expect.poll(async () => (await structure(editor)).length).toBe(4);
    await drop("tone_44k.wav", 200);
    await expect.poll(async () => (await structure(editor)).length).toBe(5);
    const kinds = (await structure(editor)).map((t) => t.kind);
    expect(kinds.filter((k) => k === "audio")).toHaveLength(2);
    expect(kinds.filter((k) => k === "visual")).toHaveLength(3);
  });

  const audioTracks = async () => (await editor.sequence()).tracks.filter((t) => t.kind === "audio");
  await test.step("áudio 3 e 4 coexistem livremente nas tracks existentes", async () => {
    const [t1, t2] = await audioTracks();
    if (!t1 || !t2) throw new Error("faltam tracks de áudio");
    await drop("tom_a3.wav", 700, { track: t1.id });
    await drop("tom_a4.wav", 700, { track: t2.id });
    await expect
      .poll(async () => (await structure(editor)).filter((t) => t.kind === "audio"))
      .toEqual([
        { kind: "audio", clips: 2 },
        { kind: "audio", clips: 2 },
      ]);
  });

  await test.step("mover um clip para outra track compatível", async () => {
    const [t1, t2] = await audioTracks();
    if (!t1 || !t2) throw new Error("faltam tracks de áudio");
    const seq = await editor.sequence();
    const moving = Object.values(seq.clips).find((c) => c.track === t1.id && c.start > 0);
    if (!moving) throw new Error("clip a mover não encontrado");
    const from = await editor.clipPoint(moving.id);
    const to = await editor.rowPoint(t2.id, 900);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(to.x, to.y, { steps: 14 });
    await page.mouse.up();
    await expect
      .poll(async () => (await editor.sequence()).clips[moving.id]?.track)
      .toBe(t2.id);
  });

  await test.step("mover um clip para o espaço vazio cria uma track", async () => {
    const seq = await editor.sequence();
    const vis = seq.tracks.filter((t) => t.kind === "visual");
    const top = vis[vis.length - 1];
    if (!top) throw new Error("sem track visual");
    const clip = Object.values(seq.clips).find((c) => c.track === top.id);
    if (!clip) throw new Error("clip do topo não encontrado");
    const from = await editor.clipPoint(clip.id);
    const to = await editor.emptyPoint(from.x);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(to.x, to.y, { steps: 14 });
    await page.mouse.up();
    await expect.poll(async () => (await structure(editor)).length).toBe(6);
    // a track de onde saiu ficou vazia
    expect((await structure(editor)).some((t) => t.clips === 0)).toBe(true);
  });

  await test.step("renomear (duplo clique), reordenar pelo menu e excluir track vazia", async () => {
    const seq = await editor.sequence();
    const empty = seq.tracks.find(
      (t) => !Object.values(seq.clips).some((c) => c.track === t.id),
    );
    if (!empty) throw new Error("sem track vazia");
    const other = seq.tracks.find((t) => t.kind === "visual" && t.id !== empty.id);
    if (!other) throw new Error("sem outra track visual");

    await page.getByTestId(`track-name-${other.id}`).dblclick();
    const input = page.getByTestId(`track-name-input-${other.id}`);
    await input.fill("Meu logo");
    await input.press("Enter");
    await expect(page.getByTestId(`track-header-${other.id}`)).toContainText("Meu logo");

    const before = (await editor.sequence()).tracks.map((t) => t.id);
    await page.getByTestId(`track-header-${other.id}`).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Mover para cima" }).click();
    await expect
      .poll(async () => (await editor.sequence()).tracks.map((t) => t.id))
      .not.toEqual(before);

    await page.getByTestId(`lock-${other.id}`).click();
    await expect.poll(async () => (await editor.sequence()).tracks.find((t) => t.id === other.id)?.locked).toBe(true);
    await page.getByTestId(`lock-${other.id}`).click();
    await expect.poll(async () => (await editor.sequence()).tracks.find((t) => t.id === other.id)?.locked).toBe(false);

    const [au] = await audioTracks();
    if (!au) throw new Error("sem áudio");
    await page.getByTestId(`mute-${au.id}`).click();
    await expect.poll(async () => (await audioTracks())[0]?.muted).toBe(true);
    await page.getByTestId(`mute-${au.id}`).click();
    await page.getByTestId(`solo-${au.id}`).click();
    await page.getByTestId(`solo-${au.id}`).click();

    await page.getByTestId(`track-header-${empty.id}`).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Excluir track" }).click();
    await expect.poll(async () => (await structure(editor)).length).toBe(5);
  });

  const before = await structure(editor);
  const beforeNames = (await editor.sequence()).tracks.map((t) => t.id);

  await test.step("fechar e reabrir pela Home preserva a estrutura", async () => {
    await page.getByTestId("close-project").click();
    await expect(page.getByTestId("welcome")).toBeVisible();
    await page.getByTestId("recent-open").first().click();
    await expect(page.getByTestId("editor")).toBeVisible({ timeout: 45_000 });
    expect(await structure(editor)).toEqual(before);
    expect((await editor.sequence()).tracks.map((t) => t.id)).toEqual(beforeNames);
    await expect(page.getByText("Meu logo")).toBeVisible();
  });

  await test.step("exporta pela interface", async () => {
    await page.getByTestId("export-btn").click();
    await expect(page.getByTestId("export-dialog")).toBeVisible();
    await page.getByTestId("export-preset").selectOption("intermediate");
    await page.getByRole("textbox", { name: "Largura" }).fill("270");
    await page.getByRole("textbox", { name: "Largura" }).press("Enter");
    await page.getByRole("textbox", { name: "Altura" }).fill("480");
    await page.getByRole("textbox", { name: "Altura" }).press("Enter");
    await page.getByTestId("export-path").fill(join(server.dir, "livre_intermediate"));
    await page.getByTestId("export-start").click();
    await expect(page.getByTestId("export-report")).toBeVisible({ timeout: 120_000 });
    await page.keyboard.press("Escape");
  });

  await expect(page.getByTestId("error-boundary")).toHaveCount(0);
  expect(errors).toEqual([]);
});
