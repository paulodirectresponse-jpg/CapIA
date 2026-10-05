import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, expect, Editor, ROOT, MEDIA } from "./fixtures";

/**
 * Fluxo externo (Fase 6): uma ferramenta externa usa o `capia-server` por REST
 * (copy + vídeo bruto + referência → projeto → assets → AI Run → aprovações → variantes → export)
 * e DEPOIS a UI (devserver no Linux, app Tauri/WebView2 no Windows) abre o MESMO arquivo de projeto
 * e encontra exatamente o mesmo estado editável. O servidor roda com o cérebro Replay de teste
 * (build com `--features testkit`; o produto não liga essa feature). Nada é pulado em nenhum alvo;
 * só o export H.264 depende de um encoder aprovado na máquina (declarado pelo próprio servidor).
 */
test.use({ aiDemo: true });
test.setTimeout(420_000);

const BRIEF = "Produto: Demo. CTA: Compre agora.";

function serverBinary(): string {
  const forced = process.env.CAPIA_SERVER_BIN;
  if (forced) return forced;
  const exe = process.platform === "win32" ? "capia-server.exe" : "capia-server";
  for (const profile of ["release", "debug"]) {
    const p = join(ROOT, "target", profile, exe);
    if (existsSync(p)) return p;
  }
  throw new Error("capia-server não compilado: cargo build -p capia-server --features testkit");
}

interface Running {
  child: ChildProcess;
  base: string;
  token: string;
  dir: string;
}

async function startServer(): Promise<Running> {
  const dir = mkdtempSync(join(tmpdir(), "capia-srv-e2e-"));
  const created = spawnSync(
    serverBinary(),
    ["token", "create", "--data-dir", dir, "--name", "e2e", "--scopes", allScopes()],
    { encoding: "utf8" },
  );
  if (created.status !== 0) throw new Error(`token create falhou: ${created.stderr}`);
  const token = (JSON.parse(created.stdout) as { secret: string }).secret;
  const child = spawn(serverBinary(), ["serve", "--data-dir", dir, "--port", "0"], {
    env: { ...process.env, CAPIA_AI_DEMO_BRAIN: "1" },
    stdio: ["ignore", "pipe", "inherit"],
  });
  const base = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error("capia-server não subiu"));
    }, 60_000);
    let buf = "";
    child.stdout.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (http:\/\/[^\s]+)/.exec(buf);
      if (m?.[1]) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    child.on("exit", (c) => {
      reject(new Error(`capia-server saiu cedo (${String(c)})`));
    });
  });
  return { child, base, token, dir };
}

function allScopes(): string {
  return [
    "project:read",
    "project:write",
    "media:read",
    "media:write",
    "run:read",
    "run:start",
    "run:approve",
    "export:read",
    "export:start",
    "webhook:manage",
    "admin:tokens",
  ].join(",");
}

async function stopServer(s: Running): Promise<void> {
  const r = spawnSync(serverBinary(), ["stop", "--data-dir", s.dir]);
  if (r.status !== 0) s.child.kill();
  await new Promise<void>((resolve) => {
    const t = setTimeout(() => {
      s.child.kill();
      resolve();
    }, 30_000);
    s.child.on("exit", () => {
      clearTimeout(t);
      resolve();
    });
    if (s.child.exitCode !== null) {
      clearTimeout(t);
      resolve();
    }
  });
}

class Rest {
  constructor(private readonly s: Running) {}

  async call<T = Record<string, unknown>>(
    method: string,
    path: string,
    body?: unknown,
    expect = 200,
  ): Promise<T> {
    const r = await fetch(`${this.s.base}${path}`, {
      method,
      headers: {
        authorization: `Bearer ${this.s.token}`,
        ...(body === undefined ? {} : { "content-type": "application/json" }),
        ...(method === "POST" ? { "idempotency-key": `e2e-${randomUUID()}` } : {}),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const text = await r.text();
    if (r.status !== expect && !(expect === 200 && r.status === 202)) {
      throw new Error(`${method} ${path} → ${String(r.status)} ${text}`);
    }
    return JSON.parse(text) as T;
  }

  async upload(file: string, name: string): Promise<string> {
    const bytes = readFileSync(join(MEDIA, file));
    const r = await fetch(`${this.s.base}/v1/uploads`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${this.s.token}`,
        "content-type": "application/octet-stream",
        "x-capia-filename": encodeURIComponent(name),
      },
      body: bytes,
    });
    const text = await r.text();
    if (r.status !== 201) throw new Error(`upload ${name} → ${String(r.status)} ${text}`);
    return (JSON.parse(text) as { upload: { upload_id: string } }).upload.upload_id;
  }
}

async function until<T>(
  what: string,
  fn: () => Promise<T | undefined>,
  timeoutMs = 180_000,
): Promise<T> {
  const t0 = Date.now();
  for (;;) {
    const v = await fn();
    if (v !== undefined) return v;
    if (Date.now() - t0 > timeoutMs) throw new Error(`timeout esperando: ${what}`);
    await new Promise((r) => setTimeout(r, 300));
  }
}

interface RunSnap {
  run: {
    status: string;
    pending: { id: string; kind: string } | null;
    sequences: { sequence_id: string; role: string }[];
  };
}

async function finishRun(rest: Rest, pid: string, runId: string): Promise<RunSnap> {
  return until(`run ${runId} concluída`, async () => {
    const v = await rest.call<RunSnap>("GET", `/v1/projects/${pid}/runs/${runId}`);
    if (v.run.status === "waiting_user" && v.run.pending) {
      await rest.call("POST", `/v1/projects/${pid}/runs/${runId}/approvals`, {
        decision_id: v.run.pending.id,
        option: "approve",
      });
      return undefined;
    }
    if (v.run.status === "failed" || v.run.status === "cancelled") {
      throw new Error(`a Run terminou em ${v.run.status}`);
    }
    return v.run.status === "completed" ? v : undefined;
  });
}

test("REST (capia-server) → mesmo estado editável na UI", async ({ page, editor, server }) => {
  const s = await startServer();
  const rest = new Rest(s);
  const state = {
    projectPath: "",
    restSequences: [] as { id: string; clip_count: number }[],
    runIds: [] as string[],
  };
  try {
    // 1) projeto + mídia bruta + referência (uploads seguros → import pelo sistema de assets)
    const created = await rest.call<{ project: { id: string } }>(
      "POST",
      "/v1/projects",
      { name: "externo" },
      201,
    );
    const pid = created.project.id;
    state.projectPath = join(s.dir, "projects", pid, "project.capia");
    const raw = await rest.upload("video_audio.mp4", "bruto.mp4");
    const ref = await rest.upload("video_only.mp4", "referencia.mp4");
    for (const up of [raw, ref]) {
      const imp = await rest.call<{ ticket_id: string }>(
        "POST",
        `/v1/projects/${pid}/assets`,
        { upload_id: up },
        202,
      );
      await until(`import ${imp.ticket_id}`, async () => {
        const v = await rest.call<{ import: { state: string } }>(
          "GET",
          `/v1/projects/${pid}/imports/${imp.ticket_id}`,
        );
        if (v.import.state === "failed") throw new Error("import falhou");
        return v.import.state === "finalized" ? true : undefined;
      });
    }
    const assets = (
      await rest.call<{ assets: { id: string }[] }>("GET", `/v1/projects/${pid}/assets`)
    ).assets;
    expect(assets).toHaveLength(2);
    // 2) AI Run (copy + bruto + referência) com aprovações pela API
    const [rawAsset, refAsset] = assets;
    if (!rawAsset || !refAsset) throw new Error("assets ausentes");
    const run = await rest.call<{ run: { id: string } }>(
      "POST",
      `/v1/projects/${pid}/runs`,
      {
        brief_text: BRIEF,
        assets: [rawAsset.id],
        references: [refAsset.id],
      },
      202,
    );
    const done = await finishRun(rest, pid, run.run.id);
    expect(done.run.sequences.length).toBeGreaterThan(0);
    // 3) variantes
    const vr = await rest.call<{ run: { id: string } }>(
      "POST",
      `/v1/projects/${pid}/runs/${run.run.id}/variants`,
      { count: 2 },
      202,
    );
    const vdone = await finishRun(rest, pid, vr.run.id);
    expect(
      vdone.run.sequences.filter((x) => x.role !== "superseded").length,
    ).toBeGreaterThanOrEqual(2);
    state.runIds = [run.run.id, vr.run.id];
    // 4) export (só se houver um encoder aprovado nesta máquina; o servidor informa)
    const enc = await rest.call<{ encoders: { approved?: boolean; codec?: string }[] }>(
      "GET",
      "/v1/exports/encoders",
    );
    const hasH264 = enc.encoders.some((e) => e.approved === true && e.codec === "h264");
    if (hasH264) {
      const master = done.run.sequences.find((x) => x.role !== "superseded");
      if (!master) throw new Error("sem master");
      const ex = await rest.call<{ exports: { id: string }[] }>(
        "POST",
        `/v1/projects/${pid}/exports`,
        {
          items: [{ sequence: master.sequence_id, preset: "h264-mp4", name: "final" }],
        },
        202,
      );
      const first = ex.exports[0];
      if (!first) throw new Error("export não criado");
      await until("export concluído", async () => {
        const v = await rest.call<{ export: { state: string } }>(
          "GET",
          `/v1/projects/${pid}/exports/${first.id}`,
        );
        if (v.export.state === "failed") throw new Error("export falhou");
        return v.export.state === "completed" ? true : undefined;
      });
    }
    state.restSequences = (
      await rest.call<{ sequences: { id: string; clip_count: number }[] }>(
        "GET",
        `/v1/projects/${pid}/sequences`,
      )
    ).sequences;
  } finally {
    await stopServer(s);
  }
  expect(state.restSequences.length).toBeGreaterThanOrEqual(2);

  // 5) a UI abre o MESMO arquivo e encontra o mesmo estado editável
  await editor.goto();
  await page.getByTestId("project-path").fill(state.projectPath);
  await page.getByTestId("project-open").click();
  await expect(page.getByTestId("editor")).toBeVisible({ timeout: 60_000 });
  const e = new Editor(page, server);
  const ui = await e.snapshot();
  const byId = (a: { id: string; clip_count: number }[]) =>
    a.map((x) => `${x.id}:${String(x.clip_count)}`).sort();
  expect(byId(ui.sequences as unknown as { id: string; clip_count: number }[])).toEqual(
    byId(state.restSequences),
  );
  const uiRuns = await e.api<{ runs: { id: string; status: string }[] }>("ai.run.list");
  expect(uiRuns.runs.map((r) => r.id).sort()).toEqual([...state.runIds].sort());
  expect(uiRuns.runs.every((r) => r.status === "completed")).toBe(true);
  // e continua editável à mão: um comando comum da UI sobre a sequence produzida pela IA
  const first = state.restSequences[0];
  if (!first) throw new Error("sem sequence");
  await e.api("command.execute", {
    label: "manual",
    commands: [
      {
        operation_id: "ui-manual-1",
        type: "add_track",
        sequence: first.id,
        id: "manual_track",
        kind: "visual",
      },
    ],
  });
  const after = await e.api<{ tracks: Record<string, unknown> }>("sequence.get", {
    sequence: first.id,
  });
  expect(JSON.stringify(after.tracks)).toContain("manual_track");
});
