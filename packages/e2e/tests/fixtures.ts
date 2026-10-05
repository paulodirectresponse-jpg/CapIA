import { test as base, chromium, expect, type Page } from "@playwright/test";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, existsSync } from "node:fs";
import { connect, createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const ROOT = resolve(here, "../../..");
export const MEDIA = join(ROOT, "tests/fixtures/media");

function freePort(): Promise<number> {
  return new Promise((res, rej) => {
    const s = createServer();
    s.listen(0, "127.0.0.1", () => {
      const addr = s.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      s.close(() => {
        res(port);
      });
    });
    s.on("error", rej);
  });
}

function devserverBinary(): string {
  const exe = process.platform === "win32" ? "capia-devserver.exe" : "capia-devserver";
  const forced = process.env.CAPIA_DEVSERVER_PROFILE;
  for (const profile of forced ? [forced] : ["release", "debug"]) {
    const p = join(ROOT, "target", profile, exe);
    if (existsSync(p)) return p;
  }
  throw new Error("capia-devserver não compilado: rode `cargo build -p capia-devserver`");
}

export interface Server {
  url: string;
  dir: string;
  child: ChildProcess;
}

/** `CAPIA_E2E_TARGET=tauri`: dirige o app Tauri real (WebView2) por CDP, em vez do devserver. */
const TAURI = process.env.CAPIA_E2E_TARGET === "tauri";

function tauriBinary(): string {
  const exe = process.platform === "win32" ? "capia-desktop.exe" : "capia-desktop";
  const p = process.env.CAPIA_DESKTOP_EXE ?? join(ROOT, "target", "release", exe);
  if (!existsSync(p)) throw new Error(`app desktop não compilado: ${p} (rode pnpm desktop:build)`);
  return p;
}

/** Sobe outro devserver (testes de crash: matar o processo e reabrir o projeto noutro). */
export async function launchDevserver(): Promise<{ url: string; child: ChildProcess }> {
  const port = await freePort();
  const child = spawn(
    devserverBinary(),
    ["--port", String(port), "--static", join(ROOT, "apps/desktop/dist")],
    { stdio: "ignore", cwd: ROOT },
  );
  const url = `http://127.0.0.1:${String(port)}`;
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(`${url}/index.html`)).ok) break;
    } catch {
      /* subindo */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  return { url, child };
}

/** Espera a porta TCP local ficar livre (nada aceitando conexões). */
async function waitPortFree(port: number, timeoutMs = 30_000): Promise<void> {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    const inUse = await new Promise<boolean>((resolve) => {
      const sock = connect({ port, host: "127.0.0.1" });
      sock.once("connect", () => {
        sock.destroy();
        resolve(true);
      });
      sock.once("error", () => { resolve(false); });
    });
    if (!inUse) return;
    await new Promise((r) => setTimeout(r, 200));
  }
}

/** Encerra o app e (no Windows) a árvore de processos do WebView2, esperando a saída de verdade. */
async function killTree(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.pid === undefined) return;
  const exited = new Promise<void>((resolve) => child.once("exit", () => { resolve(); }));
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    child.kill();
  }
  await Promise.race([exited, new Promise((r) => setTimeout(r, 10_000))]);
}

export const test = base.extend<{ server: Server; editor: Editor }, object>({
  server: async ({}, use) => {
    const dir = mkdtempSync(join(tmpdir(), "capia-e2e-"));
    if (TAURI) {
      // porta fixa de `tauri.e2e.conf.json` (additionalBrowserArgs; o env do WebView2 é ignorado pelo wry)
      const port = 9222;
      // o app anterior (e o msedgewebview2 dele) pode ainda segurar a porta CDP fixa
      await waitPortFree(port);
      const child: ChildProcess = spawn(tauriBinary(), [], {
        stdio: "ignore",
        cwd: ROOT,
        env: {
          ...process.env,
          WEBVIEW2_USER_DATA_FOLDER: join(dir, "webview2"),
        },
      });
      await use({ url: `http://127.0.0.1:${String(port)}`, dir, child });
      await killTree(child);
      await waitPortFree(port);
      return;
    }
    const port = await freePort();
    const child: ChildProcess = spawn(
      devserverBinary(),
      ["--port", String(port), "--static", join(ROOT, "apps/desktop/dist")],
      { stdio: "ignore", cwd: ROOT },
    );
    const url = `http://127.0.0.1:${String(port)}`;
    for (let i = 0; i < 100; i++) {
      try {
        const r = await fetch(`${url}/index.html`);
        if (r.ok) break;
      } catch {
        /* ainda subindo */
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    await use({ url, dir, child });
    child.kill();
  },
  // No app nativo a "página" é a janela do WebView2 (CDP): não depende do `page` do Playwright, que
  // lançaria um Chromium que não existe (nem é necessário) nesse alvo.
  ...(TAURI
    ? {
        page: async ({ server }: { server: Server }, use: (p: Page) => Promise<void>) => {
          let browser = null;
          for (let i = 0; i < 150 && !browser; i++) {
            try {
              browser = await chromium.connectOverCDP(server.url);
            } catch {
              await new Promise((r) => setTimeout(r, 200));
            }
          }
          if (!browser) throw new Error("não foi possível conectar ao WebView2 por CDP");
          let tauriPage: Page | undefined;
          for (let i = 0; i < 100 && !tauriPage; i++) {
            tauriPage = browser.contexts()[0]?.pages()[0];
            if (!tauriPage) await new Promise((r) => setTimeout(r, 200));
          }
          if (!tauriPage) throw new Error("janela do app não encontrada");
          await use(tauriPage);
          await browser.close();
        },
      }
    : {}),
  editor: async ({ page, server }, use) => {
    await use(new Editor(page, server));
  },
});

export { expect };

/** Fachada de alto nível sobre a UI (sempre pelos `data-testid`/papéis acessíveis). */
export class Editor {
  constructor(
    readonly page: Page,
    readonly server: Server,
  ) {}

  async goto(): Promise<void> {
    if (TAURI) {
      // mesma origem do app (http://tauri.localhost no Windows): habilita os ganchos `?e2e=1`
      await expect.poll(() => this.page.url(), { timeout: 30_000 }).toMatch(/^https?:/);
      const origin = new URL(this.page.url()).origin;
      await this.page.goto(`${origin}/?e2e=1`);
    } else {
      await this.page.goto(`${this.server.url}/?e2e=1`);
    }
    await expect(this.page.getByTestId("welcome")).toBeVisible();
  }

  async createProject(name = "project"): Promise<void> {
    await this.page.getByTestId("project-path").fill(join(this.server.dir, `${name}.capia`));
    await this.page.getByTestId("project-create").click();
    await expect(this.page.getByTestId("editor")).toBeVisible();
  }

  /** Chama o engine direto (a verdade persistida, independente do que a UI mostra). */
  async api<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    if (TAURI) {
      // IPC fixo do shell: o mesmo `editor_call` que a UI usa
      return this.page.evaluate(
        async ([m, p]) => {
          const w = window as unknown as {
            __TAURI_INTERNALS__: { invoke(c: string, a: unknown): Promise<unknown> };
          };
          return w.__TAURI_INTERNALS__.invoke("editor_call", { method: m, params: p });
        },
        [method, params] as const,
      ) as Promise<T>;
    }
    const r = await fetch(`${this.server.url}/api/${method}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(params),
    });
    const body = (await r.json()) as T;
    if (!r.ok) throw new Error(`${method}: ${JSON.stringify(body)}`);
    return body;
  }

  async assets(): Promise<{ id: string; name: string; status: string }[]> {
    return this.api("assets.list");
  }

  async snapshot(): Promise<Snapshot> {
    return this.api<Snapshot>("project.snapshot");
  }

  /** Conteúdo (clips/tracks) da primeira sequence, direto do engine. */
  async sequence(id?: string): Promise<Seq> {
    const snap = await this.snapshot();
    const sid = id ?? snap.sequences[0]?.id;
    if (!sid) throw new Error("projeto sem sequence");
    return { id: sid, ...(await this.api<SeqBody>("sequence.get", { sequence: sid })) };
  }

  async clips(id?: string): Promise<ClipJson[]> {
    return Object.values((await this.sequence(id)).clips).sort((a, b) => a.start - b.start);
  }

  async assetIdByName(name: string): Promise<string> {
    const snap = await this.snapshot();
    const a = snap.assets.find((x) => x.name === name);
    if (!a) throw new Error(`asset ${name} não encontrado`);
    return a.id;
  }

  /** Posição (px de página) de um ponto da timeline: clip (id) + fração horizontal/vertical. */
  async clipPoint(id: string, fx = 0.5, fy = 0.5): Promise<{ x: number; y: number }> {
    // a UI recebe mudanças feitas por outros clientes via evento (poll de 250 ms)
    await this.page
      .waitForFunction((clip) => window.__capiaTimeline?.clipRect(clip) != null, id, {
        timeout: 20_000,
      })
      .catch(() => undefined);
    const p = await this.page.evaluate(
      ([clip]) => {
        const hook = window.__capiaTimeline;
        const r = hook?.clipRect(clip as string);
        const o = hook?.canvasOrigin();
        return r && o ? { x: o.x + r.x, y: o.y + r.y, w: r.w, h: r.h } : null;
      },
      [id],
    );
    if (!p) throw new Error(`clip ${id} não está visível na timeline`);
    return { x: p.x + p.w * fx, y: p.y + p.h * fy };
  }

  async rowPoint(track: string, x: number): Promise<{ x: number; y: number }> {
    // a view pode ainda estar carregando a sequence recém-aberta
    await this.page.waitForFunction((t) => window.__capiaTimeline?.rowRect(t) != null, track, {
      timeout: 10_000,
    });
    const p = await this.page.evaluate(
      ([t]) => {
        const hook = window.__capiaTimeline;
        const r = hook?.rowRect(t as string);
        const o = hook?.canvasOrigin();
        return r && o ? { y: o.y + r.y, h: r.h, ox: o.x } : null;
      },
      [track],
    );
    if (!p) throw new Error(`faixa ${track} não está visível`);
    return { x: p.ox + x, y: p.y + p.h / 2 };
  }

  async dragAssetTo(assetId: string, to: { x: number; y: number }): Promise<void> {
    const src = this.page.getByTestId(`asset-${assetId}`);
    await src.hover();
    const box = await src.boundingBox();
    if (!box) throw new Error("asset sem caixa");
    await this.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await this.page.mouse.down();
    await this.page.mouse.move(to.x, to.y, { steps: 12 });
    await this.page.mouse.up();
  }

  /** No app nativo "Importar" abre o diálogo do SO (não automatizável): usa "Por caminho…". */
  async openImportByPath(): Promise<void> {
    const byPath = this.page.getByTestId("import-by-path");
    if (await byPath.count()) await byPath.click();
    else await this.page.getByTestId("import-media").click();
  }

  /** Importa um caminho absoluto qualquer (fora de `tests/fixtures/media`). */
  async openImportAfterRail(absPath: string): Promise<void> {
    await this.page.getByTestId("rail-media").click();
    await this.openImportByPath();
    await this.page.getByTestId("import-paths").fill(absPath);
    await this.page.getByTestId("import-confirm").click();
  }

  async importMedia(...files: string[]): Promise<void> {
    await this.page.getByTestId("rail-media").click();
    await this.openImportByPath();
    await this.page.getByTestId("import-paths").fill(files.map((f) => join(MEDIA, f)).join("\n"));
    await this.page.getByTestId("import-confirm").click();
  }
}

declare global {
  interface Window {
    __capiaTimeline?: {
      clipRect(id: string): { x: number; y: number; w: number; h: number } | null;
      rowRect(track: string): { y: number; h: number } | null;
      canvasOrigin(): { x: number; y: number };
      stats(): {
        fps: number;
        paintMs: number;
        visibleClips: number;
        frames: number;
        paintSamples: number[];
        gestureSamples: number[];
      };
      resetSamples(): void;
      viewState(): { pps: number; origin: number; scrollY: number; playhead: number };
    };
    __capiaPerf?: {
      summary(): Record<string, { n: number; p50: number; p95: number; max: number }>;
      samples(key: "commit" | "history" | "thumb" | "preview" | "rpc" | "apply"): number[];
      reset(): void;
    };
  }
}

export interface ClipJson {
  id: string;
  track: string;
  start: number;
  duration: number;
  name: string;
  content: { type: string; text?: string; asset?: string; sequence?: string; has_audio?: boolean };
  properties: Record<string, unknown>;
  transition_in?: { kind: string; duration: number } | null;
  source_in: number;
}
export interface TrackJson {
  id: string;
  kind: string;
  role: string | { custom: string };
  magnetic: boolean;
  locked: boolean;
  muted: boolean;
}
interface SeqBody {
  header: { name: string; width: number; height: number };
  tracks: TrackJson[];
  clips: Record<string, ClipJson>;
}
export type Seq = SeqBody & { id: string };
export interface Snapshot {
  revision: number;
  sequences: { id: string; name: string; duration: number }[];
  assets: { id: string; name: string; status: string }[];
}
