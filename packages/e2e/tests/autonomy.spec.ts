import { join } from "node:path";
import type { Page } from "@playwright/test";
import { test, expect, relaunch, demoEnv, Editor, type Server } from "./fixtures";

/**
 * Autonomia (Fase 5) de ponta a ponta, **no alvo real**: devserver (Linux) e app Tauri/WebView2
 * (Windows) com FFmpeg real e o "cérebro" Replay determinístico de demonstração. O cérebro só
 * existe em build/modo de teste: no devserver por `CAPIA_AI_DEMO_BRAIN`, no app desktop pela feature
 * de cargo `e2e-testkit` (que o app de produto não liga) MAIS a mesma env. Sem rede, sem chave.
 * Nenhum teste aqui é pulado em nenhum alvo.
 */
test.use({ aiDemo: true });
test.setTimeout(300_000);

const BRIEF = "Produto: Demo. CTA: Compre agora. [demo:needs-correction]";

interface RunView {
  run: {
    id: string;
    status: string;
    stage: string;
    pending: { id: string; kind: string } | null;
    sequences: { sequence_id: string; role: string; deliverable: string }[];
    usage: { review_loops: number; provider_calls: number };
  };
  stages: { stage: string; status: string }[];
}

async function boot(editor: Editor): Promise<void> {
  await editor.goto();
  await editor.createProject();
  await editor.importMedia("video_audio.mp4");
  await expect.poll(async () => (await editor.snapshot()).assets.length).toBe(1);
}

async function openRuns(page: Page): Promise<void> {
  await page.getByTestId("rail-ai").click();
  await page.getByRole("tab", { name: /Runs|Execuções/ }).click();
  await expect(page.getByTestId("ai-runs-panel")).toBeVisible();
}

async function startRun(page: Page, brief = BRIEF): Promise<void> {
  await page.getByTestId("ai-run-brief").fill(brief);
  await expect(page.getByTestId("ai-run-start")).toBeEnabled();
  await page.getByTestId("ai-run-start").click();
}

const runs = (editor: Editor) =>
  editor.api<{ runs: { id: string; status: string }[] }>("ai.run.list").then((r) => r.runs);

const view = (editor: Editor, id: string) => editor.api<RunView>("ai.run.get", { run_id: id });

const runActors = async (editor: Editor): Promise<string[]> => {
  const h = await editor.api<{ entries: { actor: { id: string } }[] }>("history.list");
  return h.entries.map((e) => e.actor.id).filter((a) => a.startsWith("run:"));
};

/** Aprova pela UI cada decisão que a Run pedir (spec do briefing, plano…) até `done()` valer. */
async function approveUntil(
  page: Page,
  editor: Editor,
  runId: string,
  done: () => Promise<boolean>,
  opts: { stopAtPlan?: boolean } = {},
): Promise<string[]> {
  const kinds: string[] = [];
  for (let i = 0; i < 40; i++) {
    if (await done()) return kinds;
    const v = await view(editor, runId);
    const pending = v.run.pending;
    if (v.run.status === "waiting_user" && pending) {
      if (opts.stopAtPlan && pending.kind === "plan_approval") return kinds;
      const approve = page.getByTestId("ai-run-option-approve");
      if (await approve.isVisible()) {
        kinds.push(pending.kind);
        await approve.click();
      }
    }
    await page.waitForTimeout(600);
  }
  throw new Error(`a Run ${runId} não chegou ao estado esperado`);
}

async function reopen(
  editor: Editor,
  page: Page,
  server: Server,
  hard = true,
): Promise<{ editor: Editor; page: Page }> {
  const p = await relaunch(server, demoEnv(server.dir), page, hard);
  const e = new Editor(p, server);
  await reopenProject(e, p, server);
  return { editor: e, page: p };
}

async function reopenProject(editor: Editor, page: Page, server: Server): Promise<void> {
  await editor.goto();
  await page.getByTestId("project-path").fill(join(server.dir, "project.capia"));
  await page.getByTestId("project-open").click();
  await expect(page.getByTestId("editor")).toBeVisible({ timeout: 45_000 });
}

test("brief + bruto → plano aprovado → EDIT → REVIEW → CORRECT → 2 variantes → undo seletivo → reabrir", async ({
  page,
  editor,
  server,
}) => {
  await boot(editor);
  const base = (await editor.snapshot()).sequences.length;
  await openRuns(page);
  await startRun(page);

  // 1) o plano espera a aprovação humana e NADA foi escrito na timeline antes dela
  await expect(page.getByTestId("ai-run-decision")).toBeVisible({ timeout: 90_000 });
  const [first] = await runs(editor);
  if (!first) throw new Error("a Run não foi criada");
  const id = first.id;
  const kinds = await approveUntil(
    page,
    editor,
    id,
    async () => (await view(editor, id)).run.pending?.kind === "plan_approval",
  );
  expect(await runActors(editor)).toEqual([]);
  expect((await editor.snapshot()).sequences.length).toBe(base);
  await expect(page.getByTestId("ai-run-decision")).toContainText(/\w/);
  expect(kinds.length).toBeGreaterThanOrEqual(0);

  // 2) aprovar o plano → VALIDATE_PLAN → EDIT (preview→apply_plan) → REVIEW → CORRECT → concluir
  await page.getByTestId("ai-run-option-approve").click();
  await expect(page.getByTestId("ai-run-sequences")).toBeVisible({ timeout: 120_000 });
  await expect
    .poll(async () => (await view(editor, id)).run.status, { timeout: 120_000 })
    .toBe("completed");
  await expect(page.getByTestId("ai-run-progress")).toBeVisible();
  await expect(page.getByTestId("ai-run-cost")).toContainText(/Cost|Custo/);
  const done = await view(editor, id);
  expect(done.run.status).toBe("completed");
  const visited = done.stages.filter((s) => s.status === "completed").map((s) => s.stage);
  for (const s of ["understand", "plan", "validate_plan", "edit", "review", "correct"]) {
    expect(visited, `estágio ${s}`).toContain(s);
  }
  expect(done.run.usage.review_loops).toBeGreaterThanOrEqual(1);
  const master = done.run.sequences.find((s) => s.role !== "superseded");
  if (!master) throw new Error("sem sequence produzida");

  // 3) o resultado é uma sequence comum: clips reais, atribuída à Run, editável à mão
  const masterBody = await editor.api<{ clips: Record<string, { content: { type: string } }> }>(
    "sequence.get",
    { sequence: master.sequence_id },
  );
  expect(Object.keys(masterBody.clips).length).toBeGreaterThan(0);
  expect(await runActors(editor)).toContain(`run:${id}`);
  await editor.api("command.execute", {
    label: "manual",
    commands: [
      {
        operation_id: "e2e-manual-1",
        type: "add_track",
        sequence: master.sequence_id,
        id: "manual_track",
        kind: "visual",
      },
    ],
  });
  await page.getByTestId(`ai-run-open-${master.sequence_id}`).click();
  const masterAfterManual = JSON.stringify(
    await editor.api("sequence.get", { sequence: master.sequence_id }),
  );

  // 4) duas variantes (Run filha do mesmo grupo), também editáveis
  await page.getByTestId("ai-run-variants").click();
  await expect.poll(async () => (await runs(editor)).length, { timeout: 30_000 }).toBe(2);
  const child = (await runs(editor)).find((r) => r.id !== id);
  if (!child) throw new Error("a Run de variantes não foi criada");
  await approveUntil(
    page,
    editor,
    child.id,
    async () => (await view(editor, child.id)).run.status === "completed",
  );
  const vdone = await view(editor, child.id);
  const variants = vdone.run.sequences.filter((s) => s.role !== "superseded");
  expect(variants.length).toBeGreaterThanOrEqual(2);
  await expect(page.locator('[data-testid="ai-run-sequences"] li')).toHaveCount(variants.length);

  // 5) undo seletivo só da Run das variantes: o master (com a edição manual) continua intacto
  await page.getByTestId("ai-run-undo").click();
  await expect
    .poll(
      async () => {
        const ids = (await editor.snapshot()).sequences.map((s) => s.id);
        return variants.some((v) => ids.includes(v.sequence_id));
      },
      { timeout: 30_000 },
    )
    .toBe(false);
  expect(JSON.stringify(await editor.api("sequence.get", { sequence: master.sequence_id }))).toBe(
    masterAfterManual,
  );
  const afterUndo = await editor.api<{ entries: unknown[] }>("history.list");

  // 6) fechar o app (kill) e reabrir: Runs, histórico e timeline persistidos
  const re = await reopen(editor, page, server);
  await openRuns(re.page);
  await expect(re.page.locator('[data-testid="ai-run-list"] li')).toHaveCount(2);
  const all = await runs(re.editor);
  expect(all.map((r) => r.status)).toEqual(["completed", "completed"]);
  const hist = await re.editor.api<{ entries: unknown[] }>("history.list");
  expect(hist.entries.length).toBe(afterUndo.entries.length);
  expect(await runActors(re.editor)).toContain(`run:${id}`);
  expect(
    JSON.stringify(await re.editor.api("sequence.get", { sequence: master.sequence_id })),
  ).toBe(masterAfterManual);
  const ids = (await re.editor.snapshot()).sequences.map((s) => s.id);
  for (const v of variants) expect(ids).not.toContain(v.sequence_id);
});

test("WAITING_USER sobrevive ao restart; cancelar não escreve; Run interrompida retoma só quando o usuário manda", async ({
  page,
  editor,
  server,
}) => {
  await boot(editor);
  await openRuns(page);
  await startRun(page, "Produto: Demo. CTA: Compre agora.");

  // a) WAITING_USER no plano → reabrir o app → a MESMA decisão continua pendente → aprovar → conclui
  await expect(page.getByTestId("ai-run-decision")).toBeVisible({ timeout: 90_000 });
  const [r1] = await runs(editor);
  if (!r1) throw new Error("sem Run");
  await approveUntil(
    page,
    editor,
    r1.id,
    async () => (await view(editor, r1.id)).run.pending?.kind === "plan_approval",
  );
  const pending = (await view(editor, r1.id)).run.pending;
  expect(pending?.kind).toBe("plan_approval");
  const { editor: e2, page: p2 } = await reopen(editor, page, server);
  await openRuns(p2);
  const again = await view(e2, r1.id);
  expect(again.run.status).toBe("waiting_user");
  expect(again.run.pending?.id).toBe(pending?.id);
  expect(await runActors(e2)).toEqual([]);
  await p2.getByTestId(`ai-run-${r1.id}`).click();
  await p2.getByTestId("ai-run-option-approve").click();
  await expect(p2.getByTestId("ai-run-sequences")).toBeVisible({ timeout: 120_000 });
  await expect
    .poll(async () => (await view(e2, r1.id)).run.status, { timeout: 120_000 })
    .toBe("completed");

  // b) cancelar no plano: nada é escrito e a Run fica `cancelled`
  await p2.getByTestId("ai-run-back").click();
  await startRun(p2, "Produto: Demo. CTA: Compre agora.");
  await expect.poll(async () => (await runs(e2)).length, { timeout: 30_000 }).toBe(2);
  const r2 = (await runs(e2)).find((r) => r.id !== r1.id);
  if (!r2) throw new Error("sem segunda Run");
  const writesBefore = (await runActors(e2)).length;
  await approveUntil(
    p2,
    e2,
    r2.id,
    async () => (await view(e2, r2.id)).run.pending?.kind === "plan_approval",
  );
  await p2.getByTestId("ai-run-cancel").click();
  await expect
    .poll(async () => (await view(e2, r2.id)).run.status, { timeout: 30_000 })
    .toBe("cancelled");
  expect((await runActors(e2)).length).toBe(writesBefore);
  await expect(p2.getByTestId("ai-run-rerun")).toBeVisible();

  // c) Run em andamento é interrompida (kill) → reabrir marca `paused` (nunca retoma sozinha)
  //    → o usuário clica "Retomar" → conclui sem duplicar edição
  await p2.getByTestId("ai-run-back").click();
  const slow = await relaunch(server, demoEnv(server.dir, { CAPIA_AI_DEMO_DELAY_MS: "2500" }), p2);
  const e3 = new Editor(slow, server);
  await reopenProject(e3, slow, server);
  await openRuns(slow);
  await startRun(slow, "Produto: Demo. CTA: Compre agora.");
  await expect.poll(async () => (await runs(e3)).length, { timeout: 30_000 }).toBe(3);
  const r3 = (await runs(e3)).find((r) => r.id !== r1.id && r.id !== r2.id);
  if (!r3) throw new Error("sem terceira Run");
  await expect
    .poll(async () => (await view(e3, r3.id)).run.status, { timeout: 30_000 })
    .toBe("running");
  const fast = await relaunch(server, demoEnv(server.dir), slow);
  const e4 = new Editor(fast, server);
  await reopenProject(e4, fast, server);
  await openRuns(fast);
  const rec = await view(e4, r3.id);
  expect(rec.run.status).toBe("paused");
  await fast.waitForTimeout(1500);
  expect((await view(e4, r3.id)).run.status).toBe("paused"); // não retomou sozinha
  await fast.getByTestId(`ai-run-${r3.id}`).click();
  await fast.getByTestId("ai-run-resume").click();
  await approveUntil(
    fast,
    e4,
    r3.id,
    async () => (await view(e4, r3.id)).run.status === "completed",
  );
  const finalActors = (await runActors(e4)).filter((a) => a === `run:${r3.id}`);
  expect(finalActors.length).toBeGreaterThan(0);
  // sem edição duplicada: ids de operação do histórico da Run são únicos
  const h = await e4.api<{ entries: { actor: { id: string }; operation_id?: string }[] }>(
    "history.list",
  );
  const ops = h.entries.filter((e) => e.actor.id === `run:${r3.id}`).map((e) => e.operation_id);
  expect(new Set(ops).size).toBe(ops.length);
});

test("AI Off: o editor segue 100% manual e nenhuma Run nasce", async ({ page, editor }) => {
  const requests: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  await boot(editor);
  await editor.api("ai.enabled.set", { enabled: false });
  await openRuns(page);
  await expect(page.getByTestId("ai-off-banner")).toBeVisible();
  await page.getByTestId("ai-run-brief").fill(BRIEF);
  await expect(page.getByTestId("ai-run-start")).toBeDisabled();
  // a API também recusa (não depende da UI)
  await expect(
    editor.api("ai.run.create", { inputs: { brief_text: BRIEF, assets: [] } }),
  ).rejects.toThrow();
  expect(await runs(editor)).toEqual([]);
  // edição manual completa continua funcionando
  await page.getByTestId("rail-text").click();
  await page.getByTestId("text-add-title").click();
  await expect.poll(async () => (await editor.clips()).length).toBeGreaterThanOrEqual(1);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await editor.clips()).length).toBe(0);
  // nenhuma requisição externa
  const foreign = requests.filter(
    (u) =>
      !u.startsWith(editor.server.url) &&
      !u.startsWith("data:") &&
      !u.startsWith("blob:") &&
      !/^https?:\/\/((tauri|ipc)\.localhost|127\.0\.0\.1|localhost)/.test(u),
  );
  expect(foreign).toEqual([]);
});

test("a geração por IA e as fontes começam desligadas; nenhuma chamada externa", async ({
  page,
  editor,
}) => {
  const requests: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  await editor.goto();
  await editor.createProject();
  await openRuns(page);
  await page.getByRole("tab", { name: /Sources|Fontes/ }).click();
  await expect(page.getByTestId("ai-sources")).toBeVisible();
  await expect(page.getByTestId("ai-generation-toggle")).toHaveText(/Off|Desligada/);
  const foreign = requests.filter(
    (u) =>
      !u.startsWith(editor.server.url) &&
      !u.startsWith("data:") &&
      !u.startsWith("blob:") &&
      !/^https?:\/\/((tauri|ipc)\.localhost|127\.0\.0\.1|localhost)/.test(u),
  );
  expect(foreign).toEqual([]);
});
