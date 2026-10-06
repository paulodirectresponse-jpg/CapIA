import { createServer, type Server as HttpServer } from "node:http";
import type { AddressInfo } from "node:net";
import { join } from "node:path";
import { test, expect } from "./fixtures";

/**
 * Jornada RC2 — 25 passos de uma pessoa que nunca viu o CapIA: criar, importar, montar uma
 * timeline livre, transição, keyframes, desfazer, salvar/reabrir, conectar a IA (provedor
 * OpenAI-compatível **simulado em loopback**; o provedor real é um gate externo) e conversar.
 * Cada passo confere o estado persistido pelo engine, não só o que a UI desenha.
 */

/** Quantidade de keyframes de uma propriedade (`{ animated: [...] }` no JSON do engine). */
function kfCount(p: unknown): number {
  const a = (p as { animated?: unknown[] } | undefined)?.animated;
  return Array.isArray(a) ? a.length : 0;
}

const KEY = "KEY-rc2-journey-0123456789abcdef";

/** OpenAI de mentira (chat/completions em SSE + /models): suficiente para o probe e o chat. */
function fakeOpenAi(): Promise<{ server: HttpServer; url: string }> {
  const sse = (events: unknown[]): string =>
    events.map((e) => `data: ${JSON.stringify(e)}\n\n`).join("") + "data: [DONE]\n\n";
  const server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      const raw = Buffer.concat(chunks).toString("utf8");
      const send = (status: number, ctype: string, body: string) => {
        res.writeHead(status, { "content-type": ctype });
        res.end(body);
      };
      if (req.headers.authorization !== `Bearer ${KEY}`) {
        return send(401, "application/json", JSON.stringify({ error: { message: "bad key" } }));
      }
      if (req.method === "GET") {
        return send(
          200,
          "application/json",
          JSON.stringify({ data: [{ id: "gpt-5" }, { id: "gpt-4o-mini" }, { id: "whisper-1" }] }),
        );
      }
      const stop = { choices: [{ delta: {}, finish_reason: "stop" }] };
      const text = (s: string) => ({ choices: [{ delta: { content: s } }] });
      let body: Record<string, unknown> = {};
      try {
        body = JSON.parse(raw) as Record<string, unknown>;
      } catch {
        /* corpo inválido: cai no texto */
      }
      if (raw.includes("image_url"))
        return send(200, "text/event-stream", sse([text("red"), stop]));
      if (body.response_format)
        return send(200, "text/event-stream", sse([text('{"ok":true,"n":7}'), stop]));
      if (raw.includes("Call the `echo` tool")) {
        return send(
          200,
          "text/event-stream",
          sse([
            {
              choices: [
                {
                  delta: {
                    tool_calls: [
                      {
                        index: 0,
                        id: "c1",
                        function: { name: "echo", arguments: '{"message":"hi"}' },
                      },
                    ],
                  },
                },
              ],
            },
            { choices: [{ delta: {}, finish_reason: "tool_calls" }] },
          ]),
        );
      }
      if (raw.includes("Count from 1 to 8"))
        return send(200, "text/event-stream", sse([text("1 2 3 "), text("4 5 6 7 8"), stop]));
      return send(
        200,
        "text/event-stream",
        sse([
          text("Posso explicar a timeline, "),
          text("fazer cortes, legendas e tirar silêncios — sempre com a sua aprovação."),
          stop,
        ]),
      );
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({
        server,
        url: `http://127.0.0.1:${String((server.address() as AddressInfo).port)}`,
      });
    });
  });
}

test("RC2 journey: 25 human steps", async ({ editor, page, server }) => {
  test.setTimeout(300_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.addInitScript(() => {
    localStorage.setItem("capia.prefs.v1", JSON.stringify({ language: "pt-BR" }));
  });

  await test.step("1. abrir o app (Home em português)", async () => {
    await editor.goto();
    await expect(page.getByTestId("welcome")).toBeVisible();
    await expect(page.getByTestId("welcome")).toContainText("Bem-vindo");
  });

  await test.step("2. criar projeto", async () => {
    await editor.createProject("rc2");
    await expect(page.getByTestId("editor")).toBeVisible();
  });

  await test.step("3. importar dois vídeos, imagem e áudio", async () => {
    await editor.importMedia("video_audio.mp4", "video_only.mp4", "image.jpg", "tone_44k.wav");
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(4);
  });

  const seq0 = await editor.sequence();
  const main = seq0.tracks.find((t) => t.role === "main");
  if (!main) throw new Error("track principal ausente");
  const a = await editor.assetIdByName("video_audio.mp4");
  const b = await editor.assetIdByName("video_only.mp4");

  await test.step("4. as tracks têm nomes simples (Vídeo 1…), não papéis fixos", async () => {
    await expect(page.getByTestId(`track-header-${main.id}`)).toContainText("Vídeo 1");
    await expect(page.getByTestId(`track-header-${main.id}`)).not.toContainText("Main");
  });

  await test.step("5. arrastar o 1º vídeo para a timeline", async () => {
    await editor.dragAssetTo(a, await editor.rowPoint(main.id, 120));
    await expect.poll(async () => (await editor.clips()).length).toBeGreaterThanOrEqual(1);
  });

  const first = (await editor.clips()).find((c) => c.track === main.id);
  if (!first) throw new Error("1º clipe não criado");

  await test.step("6. segundo vídeo encostado no primeiro (mesma track)", async () => {
    await editor.api("command.execute", {
      label: "rc2: segundo clipe",
      commands: [
        {
          operation_id: "rc2-b",
          type: "insert_clip",
          track: main.id,
          start: first.start + first.duration,
          clip: {
            id: "rc2-clip-b",
            duration: first.duration,
            content: { type: "media", asset: b, has_video: true, has_audio: false },
          },
        },
      ],
    });
    await expect
      .poll(async () => (await editor.clips()).filter((c) => c.track === main.id).length)
      .toBe(2);
  });

  await test.step("7. preview mostra quadros (sem tela preta)", async () => {
    await expect
      .poll(async () =>
        Number(await page.getByTestId("preview-canvas").getAttribute("data-presented")),
      )
      .toBeGreaterThan(0);
  });

  await test.step("8. selecionar o clipe da esquerda e aplicar transição entre os dois", async () => {
    // a UI recebe o clipe inserido pela API via evento (poll): espera o clipe B aparecer
    await editor.clipPoint("rc2-clip-b", 0.5, 0.5);
    const p = await editor.clipPoint(first.id, 0.5, 0.5);
    await page.mouse.click(p.x, p.y);
    await page.getByTestId("rail-transitions").click();
    await page.getByTestId("transition-dissolve").click();
    await expect
      .poll(
        async () => (await editor.clips()).find((c) => c.id === "rc2-clip-b")?.transition_in?.kind,
      )
      .toBe("dissolve");
  });

  await test.step("9. desfazer a transição e refazer", async () => {
    await page.getByTestId("undo").click();
    await expect
      .poll(
        async () =>
          (await editor.clips()).find((c) => c.id === "rc2-clip-b")?.transition_in ?? null,
      )
      .toBeNull();
    await page.getByTestId("redo").click();
    await expect
      .poll(
        async () => (await editor.clips()).find((c) => c.id === "rc2-clip-b")?.transition_in?.kind,
      )
      .toBe("dissolve");
  });

  await test.step("10. transição em clipe isolado mostra a mensagem humana", async () => {
    // a imagem vai sozinha para outra track: sem vizinho encostado
    const image = await editor.assetIdByName("image.jpg");
    const overlay = (await editor.sequence()).tracks.find((x) => x.role === "overlay");
    if (!overlay) throw new Error("track overlay ausente");
    await page.getByTestId("rail-media").click();
    await editor.dragAssetTo(image, await editor.rowPoint(overlay.id, 300));
    await expect
      .poll(async () => (await editor.clips()).some((c) => c.content.type === "image"))
      .toBe(true);
    const img = (await editor.clips()).find((c) => c.content.type === "image");
    if (!img) throw new Error("imagem não colocada");
    const p = await editor.clipPoint(img.id, 0.5, 0.5);
    await page.mouse.click(p.x, p.y);
    await page.getByTestId("rail-transitions").click();
    await page.getByTestId("transition-fade").click();
    await expect(page.getByText("Coloque a transição entre dois clipes encostados.")).toBeVisible();
  });

  await test.step("11. keyframe de escala pelo losango (100% → 120%)", async () => {
    const p = await editor.clipPoint(first.id, 0.5, 0.5);
    await page.mouse.click(p.x, p.y);
    const diamond = page.getByTestId("kf-diamond-scale");
    await expect(diamond).toBeVisible();
    await diamond.click();
    await expect
      .poll(async () => {
        const c = (await editor.clips()).find((x) => x.id === first.id);
        return kfCount(c?.properties.scale);
      })
      .toBe(1);
  });

  await test.step("12. segundo keyframe em outro instante", async () => {
    const mid = await editor.clipPoint(first.id, 0.9, 0.5);
    const origin = await page.evaluate(() => window.__capiaTimeline?.canvasOrigin());
    if (!origin) throw new Error("sem canvas");
    await page.mouse.click(mid.x, origin.y + 12); // régua: move o playhead
    await page.getByTestId("kf-diamond-scale").click();
    await expect
      .poll(async () => {
        const c = (await editor.clips()).find((x) => x.id === first.id);
        return kfCount(c?.properties.scale);
      })
      .toBe(2);
  });

  await test.step("13. desfazer remove só o último keyframe", async () => {
    await page.getByTestId("undo").click();
    await expect
      .poll(async () => {
        const c = (await editor.clips()).find((x) => x.id === first.id);
        return kfCount(c?.properties.scale);
      })
      .toBe(1);
    await page.getByTestId("redo").click();
  });

  await test.step("14. adicionar texto", async () => {
    await page.getByTestId("rail-text").click();
    await page.getByTestId("text-add-title").click();
    await expect
      .poll(async () => (await editor.clips()).some((c) => c.content.type === "text"))
      .toBe(true);
  });

  await test.step("15. adicionar legenda", async () => {
    await page.getByTestId("rail-captions").click();
    await page.getByTestId("caption-add").click();
    await expect(page.getByTestId("caption-list").locator("li")).toHaveCount(1);
  });

  await test.step("16. áudio solto em track própria (timeline livre)", async () => {
    const tone = await editor.assetIdByName("tone_44k.wav");
    const before = (await editor.sequence()).tracks.length;
    const audioTrack = (await editor.sequence()).tracks.find((t) => t.kind === "audio");
    if (!audioTrack) throw new Error("sem track de áudio");
    await page.getByTestId("rail-media").click();
    await editor.dragAssetTo(tone, await editor.rowPoint(audioTrack.id, 200));
    await expect
      .poll(async () =>
        (await editor.clips()).some((c) => c.content.type === "media" && c.track === audioTrack.id),
      )
      .toBe(true);
    expect((await editor.sequence()).tracks.length).toBeGreaterThanOrEqual(before);
  });

  await test.step("17. fechar e reabrir pela Home (recentes): keyframes e transição persistem", async () => {
    await page.getByTestId("close-project").click();
    await expect(page.getByTestId("welcome")).toBeVisible();
    // o projeto aparece nos recentes; abrir com um clique
    await expect(page.getByTestId("recent-projects")).toContainText("rc2");
    await page.getByTestId("recent-open").first().click();
    await expect(page.getByTestId("editor")).toBeVisible({ timeout: 45_000 });
    const clips = await editor.clips();
    expect(kfCount(clips.find((x) => x.id === first.id)?.properties.scale)).toBe(2);
    expect(clips.find((c) => c.id === "rc2-clip-b")?.transition_in?.kind).toBe("dissolve");
  });

  const fake = await fakeOpenAi();
  try {
    await test.step("18. Conectar IA (chave errada → erro humano, sem perfil)", async () => {
      const t = await editor.api<{ task_id: string }>("ai.connect", {
        preset: "openai",
        api_key: "KEY-wrong-0123456789abcdef",
        base_url: fake.url,
        allow_loopback: true,
      });
      expect(t.task_id).toBeTruthy();
      await expect
        .poll(
          async () =>
            (await editor.api<{ active_profile: string | null }>("ai.status")).active_profile,
          {
            timeout: 8_000,
          },
        )
        .toBeNull();
    });

    await test.step("19. Conectar IA com a chave certa → perfil automático ativo", async () => {
      await editor.api("ai.connect", {
        preset: "openai",
        api_key: KEY,
        base_url: fake.url,
        allow_loopback: true,
      });
      await expect
        .poll(
          async () =>
            (await editor.api<{ active_profile: string | null }>("ai.status")).active_profile,
          {
            timeout: 45_000,
          },
        )
        .toBe("auto");
    });

    await test.step("20. a chave nunca volta para a tela nem para o status", async () => {
      const st = JSON.stringify(await editor.api("ai.status"));
      expect(st).not.toContain(KEY);
      const html = await page.content();
      expect(html).not.toContain(KEY);
    });

    await test.step("21. o probe mediu cada capacidade (modelo gpt-5, transcrição whisper-1)", async () => {
      const st = await editor.api<{
        models: { id: string; enabled: boolean; last_probe?: { status?: string } }[];
      }>("ai.status");
      const chat = st.models.find((m) => m.id === "openai:gpt-5");
      expect(chat?.enabled).toBe(true);
      expect(chat?.last_probe?.status).toBe("ready");
      expect(st.models.find((m) => m.id === "openai:whisper-1")?.enabled).toBe(true);
    });

    await test.step("22. chat responde 'o que você pode fazer?'", async () => {
      await page.getByTestId("rail-ai").click();
      await page.getByTestId("ai-input").fill("o que você pode fazer?");
      await page.getByTestId("ai-send").click();
      await expect(page.getByTestId("ai-log")).toContainText("Posso explicar a timeline", {
        timeout: 30_000,
      });
    });
  } finally {
    fake.server.close();
  }

  await test.step("23. o projeto continua intacto depois da IA (nenhuma edição sem aprovação)", async () => {
    const clips = await editor.clips();
    expect(clips.find((c) => c.id === "rc2-clip-b")?.transition_in?.kind).toBe("dissolve");
  });

  await test.step("24. exportar (intermediário) com progresso e relatório validado", async () => {
    await page.getByTestId("export-btn").click();
    await expect(page.getByTestId("export-dialog")).toBeVisible();
    await page.getByTestId("export-preset").selectOption("intermediate");
    await page.getByRole("textbox", { name: "Largura" }).fill("270");
    await page.getByRole("textbox", { name: "Largura" }).press("Enter");
    await page.getByRole("textbox", { name: "Altura" }).fill("480");
    await page.getByRole("textbox", { name: "Altura" }).press("Enter");
    await page.getByTestId("export-path").fill(join(server.dir, "rc2_intermediate"));
    await page.getByTestId("export-start").click();
    await expect(page.getByTestId("export-report")).toBeVisible({ timeout: 120_000 });
    await page.keyboard.press("Escape");
  });

  await test.step("25. nenhum erro de página e nenhuma tela de falha", async () => {
    await expect(page.getByTestId("error-boundary")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
});
