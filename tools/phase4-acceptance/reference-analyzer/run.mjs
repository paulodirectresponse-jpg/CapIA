#!/usr/bin/env node
// Reference Analyzer: erro de detecção de cortes no corpus anotado (critério: ≤ 5 %).
//   (sem args)         corpus sintético anotado por construção (ffmpeg/lavfi) + held-out
//   --corpus <dir>     corpus REAL anotado por humanos: <dir>/annotations.json
//                      { "arquivo.mp4": [ { "t_s": 2.4 }, … ] }  (+ devserver rodando em --server)
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const out = join(root, "target/phase4-acceptance");
mkdirSync(out, { recursive: true });
const arg = (k) => {
  const i = process.argv.indexOf(k);
  return i >= 0 ? process.argv[i + 1] : undefined;
};

function synthetic() {
  const r = spawnSync(
    "cargo",
    ["test", "-q", "-p", "capia-intelligence", "--test", "scene_corpus", "--", "--nocapture"],
    { cwd: root, encoding: "utf8" },
  );
  const text = `${r.stdout}${r.stderr}`;
  const m = text.match(/tp=(\d+) fp=(\d+) fn=(\d+) total=(\d+) error=([0-9.]+)/);
  const held = text.match(/held-out: SceneEval \{ truth: (\d+).*?error_rate: ([0-9.]+)/);
  const res = {
    kind: "synthetic-annotated-by-construction",
    executed: r.status === 0 && !!m,
    tp: m ? +m[1] : null,
    fp: m ? +m[2] : null,
    fn: m ? +m[3] : null,
    truth: m ? +m[4] : null,
    error_rate: m ? +m[5] : null,
    heldout_error_rate: held ? +held[2] : null,
    threshold: 0.05,
    passed: !!m && +m[5] <= 0.05,
  };
  writeFileSync(join(out, "reference-scenes-synthetic.json"), JSON.stringify(res, null, 2));
  console.log(res);
  process.exit(res.passed ? 0 : 1);
}

async function real(dir, base) {
  const ann = JSON.parse(readFileSync(join(dir, "annotations.json"), "utf8"));
  const call = async (m, p = {}) => {
    const r = await fetch(`${base}/api/${m}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(p),
    });
    const j = await r.json();
    if (!r.ok) throw new Error(`${m}: ${JSON.stringify(j)}`);
    return j;
  };
  const tmp = mkdtempSync(join(tmpdir(), "capia-ref-"));
  await call("project.create", { path: join(tmp, "p.capia") });
  let tp = 0,
    fp = 0,
    fn = 0,
    total = 0;
  const rows = [];
  for (const [file, cuts] of Object.entries(ann)) {
    const path = join(dir, file);
    if (!existsSync(path)) throw new Error(`mídia ausente: ${path}`);
    await call("assets.import", { paths: [path] });
    let id;
    for (let i = 0; i < 600 && !id; i++) {
      const ev = await call("events.poll");
      id = ev.events.find((e) => e.kind === "import_finalized")?.result.asset_id;
      if (!id) await new Promise((r) => setTimeout(r, 100));
    }
    const { task_id } = await call("ai.scenes.detect", { asset_id: id });
    let det;
    for (let i = 0; i < 3000 && !det; i++) {
      const t = await call("ai.task.get", { task_id });
      if (t.state === "done") det = t.result.boundaries;
      else if (t.state !== "running") throw new Error(`falhou: ${JSON.stringify(t)}`);
      else await new Promise((r) => setTimeout(r, 100));
    }
    const times = det.map((b) => b.frame / 25);
    const used = new Set();
    let ftp = 0;
    for (const c of cuts) {
      const tol = c.tolerance_s ?? 0.12;
      const k = times.findIndex((t, i) => !used.has(i) && Math.abs(t - c.t_s) <= tol);
      if (k >= 0) {
        used.add(k);
        ftp++;
      }
    }
    tp += ftp;
    fn += cuts.length - ftp;
    fp += times.length - used.size;
    total += cuts.length;
    rows.push({ file, annotated: cuts.length, detected: times.length, tp: ftp });
  }
  const error = (fp + fn) / total;
  const res = {
    kind: "human-annotated-corpus",
    executed: true,
    files: rows.length,
    tp,
    fp,
    fn,
    truth: total,
    error_rate: error,
    threshold: 0.05,
    passed: error <= 0.05,
    rows,
  };
  writeFileSync(join(out, "reference-scenes-real.json"), JSON.stringify(res, null, 2));
  console.log(res);
  process.exit(res.passed ? 0 : 1);
}

const corpus = arg("--corpus");
if (corpus) await real(resolve(corpus), arg("--server") ?? "http://127.0.0.1:5199");
else synthetic();
