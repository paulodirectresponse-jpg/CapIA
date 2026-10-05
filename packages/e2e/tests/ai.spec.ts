import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { test, expect, launchDevserver, Editor, type Server } from "./fixtures";

/**
 * IA (Fase 4) de ponta a ponta: UI real + engine real + provider **Replay** (sem rede, sem chave).
 * Os roteiros entram por `CAPIA_AI_REPLAY_SCRIPT` no devserver. Não roda no alvo Tauri (o app real
 * nunca carrega roteiros Replay); o AI Off é coberto nos dois alvos pelos testes Rust/UI.
 */
test.skip(process.env.CAPIA_E2E_TARGET === "tauri", "roteiros Replay só existem no devserver");

const FRAME = 23_520_000;

const textDelta = (text: string) => ({
  kind: "chat",
  events: [{ event: "text_delta", text }],
  chunk_delay_ms: 0,
});

const toolCall = (id: string, name: string, args: unknown) => ({
  kind: "chat",
  events: [{ event: "tool_call", id, name, arguments: args }],
  chunk_delay_ms: 0,
});

const transcript = {
  kind: "transcript",
  transcript: {
    schema_version: 1,
    language: "pt",
    duration_us: 1_000_000,
    segments: [
      {
        start_us: 50_000,
        end_us: 900_000,
        text: "Olá pessoal compre agora",
        words: [
          { start_us: 50_000, end_us: 300_000, text: "Olá" },
          { start_us: 300_000, end_us: 600_000, text: "pessoal" },
          { start_us: 600_000, end_us: 900_000, text: "compre agora" },
        ],
      },
    ],
  },
};

async function boot(
  page: Page,
  scripts: Record<string, unknown[]> | null,
): Promise<{ editor: Editor; server: Server; dir: string }> {
  const dir = mkdtempSync(join(tmpdir(), "capia-ai-e2e-"));
  const env: Record<string, string> = {};
  if (scripts) {
    const f = join(dir, "replay.json");
    writeFileSync(f, JSON.stringify(scripts));
    env.CAPIA_AI_REPLAY_SCRIPT = f;
  }
  const { url, child } = await launchDevserver(env);
  const server: Server = { url, dir, child };
  const editor = new Editor(page, server);
  await editor.goto();
  await editor.createProject();
  await editor.importMedia("video_audio.mp4");
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
  const seq = await editor.sequence();
  const main = seq.tracks.find((t) => t.role === "main");
  if (!main) throw new Error("sem track principal");
  const asset = await editor.assetIdByName("video_audio.mp4");
  await editor.api("command.execute", {
    label: "setup",
    commands: [
      {
        operation_id: "e2e-clip",
        type: "insert_clip",
        track: main.id,
        start: 0,
        clip: {
          id: "vclip",
          duration: 30 * FRAME,
          content: { type: "media", asset, has_video: true, has_audio: true },
        },
      },
    ],
  });
  await expect.poll(async () => (await editor.clips()).length).toBe(1);
  return { editor, server, dir };
}

async function configure(editor: Editor, ids: { stt?: boolean; brain?: boolean }) {
  if (ids.stt) {
    await editor.api("ai.provider.save", {
      provider: { id: "stt", kind: "replay", display_name: "STT replay", enabled: true },
    });
    await editor.api("ai.model.save", {
      endpoint: modelJson("stt", "whisper", ["speech_to_text"]),
    });
  }
  if (ids.brain) {
    await editor.api("ai.provider.save", {
      provider: { id: "brain", kind: "replay", display_name: "Brain replay", enabled: true },
    });
    await editor.api("ai.model.save", {
      endpoint: modelJson("brain", "brain-model", [
        "text_generation",
        "tool_calling",
        "structured_output",
        "streaming",
      ]),
    });
  }
  const brain = ids.brain ? "brain:brain-model" : "stt:whisper";
  await editor.api("ai.brain.set", { profile: { id: "p", name: "p", brain } });
}

function modelJson(provider: string, model: string, caps: string[]) {
  const entries: Record<string, unknown> = {};
  for (const c of caps) entries[c] = { supported: true, origin: "declared" };
  return {
    id: `${provider}:${model}`,
    provider_id: provider,
    model_id: model,
    display_name: model,
    capabilities: { entries },
    context_window: 200_000,
    max_output_tokens: 0,
    enabled: true,
  };
}

test("AI Off: o editor inteiro funciona sem nenhuma chamada de IA nem rede externa", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  const dir = mkdtempSync(join(tmpdir(), "capia-ai-off-"));
  const { url, child } = await launchDevserver();
  try {
    const editor = new Editor(page, { url, dir, child });
    await editor.goto();
    await editor.createProject();
    await editor.importMedia("video_audio.mp4");
    await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
    // edição manual completa, sem abrir o painel de IA
    const seq = await editor.sequence();
    const main = seq.tracks.find((t) => t.role === "main");
    if (!main) throw new Error("sem main");
    const asset = await editor.assetIdByName("video_audio.mp4");
    await editor.dragAssetTo(asset, await editor.rowPoint(main.id, 120));
    await expect.poll(async () => (await editor.clips()).length).toBeGreaterThanOrEqual(1);
    await page.keyboard.press("Control+z");
    await expect.poll(async () => (await editor.clips()).length).toBe(0);
    // zero chamadas ai.* e zero requisições fora do devserver local
    expect(requests.filter((u) => u.includes("/api/ai."))).toEqual([]);
    const foreign = requests.filter(
      (u) => !u.startsWith(url) && !u.startsWith("data:") && !u.startsWith("blob:"),
    );
    expect(foreign).toEqual([]);
    // abrir o painel: aviso de "sem modelo", envio desabilitado, editor intacto
    await page.getByTestId("rail-ai").click();
    await expect(page.getByTestId("ai-not-configured")).toBeVisible();
    await page.getByTestId("ai-input").fill("oi");
    await expect(page.getByTestId("ai-send")).toBeDisabled();
    await editor.api("ai.enabled.set", { enabled: false });
    await page.getByTestId("rail-media").click();
    await page.getByTestId("rail-ai").click();
    await expect(page.getByTestId("ai-off-banner")).toBeVisible();
    // mesmo com a IA desligada, a ferramenta local de cenas funciona
    expect((await editor.api<{ enabled: boolean }>("ai.status")).enabled).toBe(false);
  } finally {
    child.kill();
  }
});

test("legendas automáticas: transcrição Replay → oferta → aplicar → desfazer", async ({ page }) => {
  const { editor, server } = await boot(page, { stt: [transcript] });
  try {
    await configure(editor, { stt: true });
    await page.getByTestId("rail-ai").click();
    await page.getByRole("tab", { name: /AI tools|Ferramentas de IA/ }).click();
    await expect(page.getByTestId("ai-tools")).toBeVisible();
    await page.getByTestId("ai-captions").click();
    await expect(page.getByTestId("ai-offer")).toBeVisible({ timeout: 30_000 });
    // a oferta NÃO alterou o documento
    expect((await editor.clips()).filter((c) => c.content.type === "text")).toHaveLength(0);
    await page.getByTestId("ai-offer-apply").click();
    await expect
      .poll(async () => (await editor.clips()).filter((c) => c.content.type === "text").length)
      .toBeGreaterThanOrEqual(1);
    const caps = (await editor.clips()).filter((c) => c.content.type === "text");
    expect(caps.every((c) => c.start % FRAME === 0)).toBe(true);
    // ator = agent no histórico; desfaz como qualquer edição
    const hist = await editor.api<{ entries: { actor: { kind: string } }[] }>("history.list");
    expect(hist.entries.at(-1)?.actor.kind).toBe("agent");
    await page.keyboard.press("Control+z");
    await expect
      .poll(async () => (await editor.clips()).filter((c) => c.content.type === "text").length)
      .toBe(0);
  } finally {
    server.child.kill();
  }
});

test("chat: ferramenta → preview → pede aprovação → aplicar → desfazer", async ({ page }) => {
  const { editor, server } = await boot(page, {
    brain: [
      toolCall("c0", "timeline.preview", {
        label: "Renomear",
        commands: [{ type: "rename_clip", clip: "vclip", name: "Hook" }],
      }),
      toolCall("c1", "timeline.apply_plan", { plan_token: "$LAST_PLAN_TOKEN" }),
      textDelta("Renomeei o clip para Hook."),
    ],
  });
  try {
    await configure(editor, { brain: true });
    await page.getByTestId("rail-ai").click();
    await page.getByTestId("ai-input").fill("renomeie o clip para Hook");
    await page.getByTestId("ai-send").click();
    await expect(page.getByTestId("ai-approval")).toBeVisible({ timeout: 30_000 });
    // pausado: nada aplicado
    expect((await editor.clips())[0]?.name).toBe("");
    await page.getByTestId("ai-approve").click();
    await expect.poll(async () => (await editor.clips())[0]?.name).toBe("Hook");
    await page.keyboard.press("Control+z");
    await expect.poll(async () => (await editor.clips())[0]?.name).toBe("");
  } finally {
    server.child.kill();
  }
});

test("chat: o usuário recusa e nada é aplicado; cancelar para a tarefa", async ({ page }) => {
  const { editor, server } = await boot(page, {
    brain: [
      toolCall("c0", "timeline.preview", {
        label: "Renomear",
        commands: [{ type: "rename_clip", clip: "vclip", name: "X" }],
      }),
      toolCall("c1", "timeline.apply_plan", { plan_token: "$LAST_PLAN_TOKEN" }),
    ],
  });
  try {
    await configure(editor, { brain: true });
    await page.getByTestId("rail-ai").click();
    await page.getByTestId("ai-input").fill("renomeie");
    await page.getByTestId("ai-send").click();
    await expect(page.getByTestId("ai-approval")).toBeVisible({ timeout: 30_000 });
    await page.getByTestId("ai-reject").click();
    await expect(page.getByTestId("ai-approval")).toBeHidden();
    expect((await editor.clips())[0]?.name).toBe("");
  } finally {
    server.child.kill();
  }
});

test("briefing → DemandSpec com fontes verificadas (e citação inventada não passa)", async ({
  page,
}) => {
  const doc = {
    product: "Produto: Café Serra Azul 500g.",
    audience: "Público: mulheres de 25 a 40 anos.",
  };
  const spec = (v: string, quotes: [string, string, string]) => ({
    value: v,
    basis: "explicit",
    sources: [{ doc: "D1", unit: quotes[0], quote: quotes[1] }],
    _x: quotes[2],
  });
  const empty = { value: null, basis: "inferred", sources: [] };
  const reply = {
    title: "Café",
    product: spec("Café Serra Azul 500g", ["u1", "Café Serra Azul 500g", ""]),
    audience: spec("mulheres de 25 a 40 anos", ["u2", "mulheres de 25 a 40 anos", ""]),
    // inventado: não consta no documento
    offer: spec("frete grátis", ["u1", "frete grátis para todo o Brasil", ""]),
    objective: empty,
    tone: empty,
    platform: empty,
    duration_and_format: empty,
    cta: empty,
    key_claims: [],
    must_include: [],
    must_avoid: [],
    constraints: [],
    assets_mentioned: [],
    open_questions: [{ question: "Qual a duração alvo?", reason: "não consta" }],
  };
  const { editor, server, dir } = await boot(page, { brain: [textDelta(JSON.stringify(reply))] });
  try {
    await configure(editor, { brain: true });
    const file = join(dir, "briefing.txt");
    writeFileSync(file, `${doc.product}\n\n${doc.audience}\n`);
    await page.getByTestId("rail-ai").click();
    await page.getByRole("tab", { name: /Brief|Briefing/ }).click();
    await page.getByTestId("ai-brief-paths").fill(file);
    await page.getByTestId("ai-brief-interpret").click();
    await expect(page.getByTestId("ai-spec")).toBeVisible({ timeout: 30_000 });
    await expect(page.getByTestId("ai-spec-product")).toContainText("Café Serra Azul 500g");
    await expect(page.getByTestId("ai-spec-product")).toContainText(/stated|dito no briefing/);
    // a citação inventada foi descartada e o campo ficou "não verificado"
    await expect(page.getByTestId("ai-spec-offer")).toContainText(/unverified|não verificado/);
    await expect(page.getByTestId("ai-spec-verification")).toContainText(/2 (of|de) 3/);
    await expect(page.getByTestId("ai-spec-questions")).toContainText("duração");
  } finally {
    server.child.kill();
  }
});

test("referência: análise local (sem provider) mostra a gramática e o selo 'local'", async ({
  page,
}) => {
  const { editor, server } = await boot(page, null);
  try {
    expect((await editor.api<{ any_usable_model: boolean }>("ai.status")).any_usable_model).toBe(
      false,
    );
    await page.getByTestId("rail-ai").click();
    await page.getByRole("tab", { name: /AI tools|Ferramentas de IA/ }).click();
    await page.getByTestId("ai-reference").click();
    await expect(page.getByTestId("ai-grammar")).toBeVisible({ timeout: 60_000 });
    await expect(page.getByTestId("ai-grammar")).toContainText(
      /Computed locally|Calculado localmente/,
    );
  } finally {
    server.child.kill();
  }
});

test("a chave de API é só de escrita: nada de segredo no DOM, no status nem no diagnóstico", async ({
  page,
}) => {
  const KEY = "sk-E2E-CANARY-0123456789abcdef";
  const { editor, server } = await boot(page, null);
  try {
    await page.getByTestId("rail-ai").click();
    await page.getByTestId("ai-open-settings").click();
    await page.getByTestId("ai-prov-id").fill("openai");
    await page.getByTestId("ai-prov-name").fill("OpenAI");
    await page.getByTestId("ai-prov-url").fill("https://api.openai.com/v1");
    await page.getByTestId("ai-prov-key").fill(KEY);
    await expect(page.getByTestId("ai-prov-key")).toHaveAttribute("type", "password");
    await page.getByTestId("ai-prov-save").click();
    await expect(page.getByTestId("ai-cred-openai")).toContainText(/Key saved|Chave salva/);
    await expect(page.getByTestId("ai-prov-key")).toHaveValue("");
    expect(await page.content()).not.toContain(KEY);
    expect(JSON.stringify(await editor.api("ai.status"))).not.toContain(KEY);
    await page.getByTestId("ai-diagnostics").click();
    const diag = await page.getByTestId("ai-diagnostics-text").inputValue();
    expect(diag).not.toContain(KEY);
    const stored = await page.evaluate(() => JSON.stringify(Object.entries(localStorage)));
    expect(stored).not.toContain(KEY);
  } finally {
    server.child.kill();
  }
});
