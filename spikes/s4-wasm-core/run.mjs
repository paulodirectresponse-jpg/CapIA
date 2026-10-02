// S4 harness: load WASM in Node (V8, same engine family as WebView2), check parity with native, measure call latency + JSON patch cost.
import fs from "node:fs";
const bytes = fs.readFileSync(new URL("./target/wasm32-unknown-unknown/release/s4_wasm_core.wasm", import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, {});
const w = instance.exports;
const t0 = performance.now(); const n = w.init(50, 200, 12345); const tInit = performance.now() - t0;
const t1 = performance.now(); const h = w.run_ops_hash(99, 200000); const tRun = performance.now() - t1;
console.log(JSON.stringify({ clips: n, init_ms: +tInit.toFixed(1), wasm_hash: h.toString(), run_200k_ms: +tRun.toFixed(1), us_per_op: +(tRun * 1000 / 200000).toFixed(3), hash_type: typeof h }));

// Ghost-move latency from JS at 10k clips, as a drag would call it (fresh state, f64 ticks at boundary)
w.init(50, 200, 12345);
const frame = 23543520, N = 100000; let s = 1, acc = 0, valid = 0;
const lat = new Float64Array(2000);
for (let i = 0; i < 2000; i++) { // sample individual call latency
  const a = performance.now();
  const r = w.ghost_move((i * 7919) % 10000, w.clip_start((i * 7919) % 10000) + (i % 50) * frame, 6 * frame);
  lat[i] = (performance.now() - a) * 1000;
}
const sorted = [...lat].sort((a, b) => a - b);
const q = p => +sorted[Math.floor(p * (sorted.length - 1))].toFixed(2);
const t2 = performance.now();
for (let i = 0; i < N; i++) { const idx = (i * 7919) % 10000; const r = w.ghost_move(idx, w.clip_start(idx) + (i % 97) * frame, 6 * frame); acc += r; if (r >= 0) valid++; }
const tBulk = performance.now() - t2;
console.log(JSON.stringify({ ghost_call_us: { p50: q(0.5), p95: q(0.95), p99: q(0.99), max: q(1) }, bulk_100k_calls_ms: +tBulk.toFixed(1), avg_us_per_call_incl_clip_start: +(tBulk * 1000 / N).toFixed(3), valid }));

// i64 across the boundary: BigInt vs f64
const big = BigInt(24 * 3600) * 705600000n;
console.log(JSON.stringify({ max_ticks_24h: big.toString(), is_safe_integer: Number.isSafeInteger(Number(big)), max_safe_hours: +(Number.MAX_SAFE_INTEGER / 705600000 / 3600).toFixed(0) }));

// IPC proxy: serialization cost of a 147-op and 2000-op transaction's patches (NOT a Tauri measurement)
const mk = (k) => Array.from({ length: k }, (_, i) => ({ op: "set", id: "0192f3a0-7c3e-7000-8000-" + String(i).padStart(12, "0"), path: "clip.start", old: 23543520 * i, new: 23543520 * (i + 3) }));
for (const k of [147, 2000, 20000]) {
  const patches = mk(k); let json, back; const reps = 200;
  const a = performance.now(); for (let r = 0; r < reps; r++) json = JSON.stringify(patches); const ser = (performance.now() - a) / reps;
  const b = performance.now(); for (let r = 0; r < reps; r++) back = JSON.parse(json); const par = (performance.now() - b) / reps;
  console.log(JSON.stringify({ patches: k, json_KB: +(json.length / 1024).toFixed(1), stringify_ms: +ser.toFixed(3), parse_ms: +par.toFixed(3) }));
}
