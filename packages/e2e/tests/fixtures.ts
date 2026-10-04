import { test as base, expect, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, existsSync } from "node:fs";
import { createServer } from "node:net";
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
  for (const profile of ["release", "debug"]) {
    const p = join(ROOT, "target", profile, exe);
    if (existsSync(p)) return p;
  }
  throw new Error("capia-devserver não compilado: rode `cargo build -p capia-devserver`");
}

export interface Server {
  url: string;
  dir: string;
}

export const test = base.extend<{ server: Server; editor: Editor }, object>({
  server: async ({}, use) => {
    const port = await freePort();
    const dir = mkdtempSync(join(tmpdir(), "capia-e2e-"));
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
    await use({ url, dir });
    child.kill();
  },
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
    await this.page.goto(`${this.server.url}/?e2e=1`);
    await expect(this.page.getByTestId("welcome")).toBeVisible();
  }

  async createProject(name = "project"): Promise<void> {
    await this.page.getByTestId("project-path").fill(join(this.server.dir, `${name}.capia`));
    await this.page.getByTestId("project-create").click();
    await expect(this.page.getByTestId("editor")).toBeVisible();
  }

  /** Chama o engine direto (a verdade persistida, independente do que a UI mostra). */
  async api<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
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

  async importMedia(...files: string[]): Promise<void> {
    await this.page.getByTestId("rail-media").click();
    await this.page.getByTestId("import-media").click();
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
      stats(): { fps: number; paintMs: number; visibleClips: number; frames: number };
      viewState(): { pps: number; origin: number; scrollY: number; playhead: number };
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
