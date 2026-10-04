import assert from "node:assert/strict";
import { test } from "node:test";
import { checkJs, checkRust, findCycles } from "./check-architecture.mjs";

const pkg = (name, deps = []) => ({
  name,
  dependencies: deps.map(([n, kind]) => ({ name: n, kind: kind ?? null })),
});
const base = () => [
  pkg("capia-time"),
  pkg("capia-model", [["capia-time"]]),
  pkg("capia-commands", [["capia-time"], ["capia-model"]]),
  pkg("capia-jobs", [["serde"], ["serde_json"]]),
  pkg("capia-render", [["capia-time"], ["capia-model"], ["capia-commands", "dev"], ["ab_glyph"]]),
  pkg("capia-media", [["capia-time"], ["serde"], ["serde_json"], ["sha2"], ["capia-media", "dev"]]),
  pkg("capia-preview", [["capia-time"], ["capia-model"], ["capia-render"]]),
  pkg("capia-decode", [["capia-time"], ["capia-media"]]),
  pkg("capia-assets", [
    ["capia-time"],
    ["capia-model"],
    ["capia-media"],
    ["serde"],
    ["serde_json"],
    ["sha2"],
  ]),
  pkg("capia-store", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["capia-assets"],
    ["capia-media"],
    ["serde"],
    ["serde_json"],
    ["rusqlite"],
    ["getrandom"],
    ["capia-store", "dev"],
  ]),
  pkg("capia-project", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["capia-store"],
    ["capia-assets"],
    ["capia-media"],
    ["serde"],
    ["serde_json"],
  ]),
  pkg("capia-cli", [["capia-project"], ["capia-commands"], ["capia-model"], ["serde_json"]]),
  pkg("capia-editor-api", [
    ["capia-project"],
    ["capia-commands"],
    ["capia-model"],
    ["capia-time"],
    ["capia-store"],
    ["capia-assets"],
    ["capia-media"],
    ["serde"],
    ["serde_json"],
  ]),
  pkg("capia-desktop", [
    ["capia-project"],
    ["capia-editor-api"],
    ["tauri"],
    ["tauri-build", "build"],
  ]),
];

test("the approved layout passes", () => {
  assert.deepEqual(checkRust({ packages: base() }), []);
});

test("an upward dependency is rejected (model -> commands)", () => {
  const p = base();
  p[1] = pkg("capia-model", [["capia-time"], ["capia-commands"]]);
  assert.match(
    checkRust({ packages: p }).join("\n"),
    /capia-model não pode depender de capia-commands/,
  );
});

test("a cycle is detected", () => {
  assert.deepEqual(findCycles({ a: ["b"], b: ["c"], c: ["a"] })[0], ["a", "b", "c", "a"]);
  assert.deepEqual(findCycles({ a: ["b"], b: [] }), []);
});

test("Tauri/wgpu/AI/IO crates are rejected in the core", () => {
  for (const bad of [
    "tauri",
    "wgpu",
    "ffmpeg-next",
    "rusqlite",
    "reqwest",
    "async-openai",
    "anthropic-sdk",
  ]) {
    const p = base();
    p[0] = pkg("capia-time", [[bad]]);
    assert.match(checkRust({ packages: p }).join("\n"), new RegExp(`${bad}.*núcleo|${bad}`));
  }
});

test("SQLite lives only in capia-store (self dev-dependency for failpoints is allowed)", () => {
  assert.deepEqual(checkRust({ packages: base() }), []);
  const p = base();
  const project = p.findIndex((x) => x.name === "capia-project");
  p[project] = pkg("capia-project", [["capia-commands"], ["rusqlite"]]);
  assert.match(checkRust({ packages: p }).join("\n"), /capia-project.*rusqlite/);
  const q = base();
  const store = q.findIndex((x) => x.name === "capia-store");
  q[store] = pkg("capia-store", [["capia-project"]]);
  assert.match(
    checkRust({ packages: q }).join("\n"),
    /capia-store não pode depender de capia-project/,
  );
  const r = base();
  const cli = r.findIndex((x) => x.name === "capia-cli");
  r[cli] = pkg("capia-cli", [["capia-project"], ["rusqlite"]]);
  assert.match(checkRust({ packages: r }).join("\n"), /capia-cli.*rusqlite/);
});

test("media and assets layers keep their boundaries (M07)", () => {
  const swap = (name, deps) => {
    const p = base();
    p[p.findIndex((x) => x.name === name)] = pkg(name, deps);
    return checkRust({ packages: p }).join("\n");
  };
  // capia-media só conhece o tempo: nada de modelo, comandos, store ou SQLite
  assert.match(
    swap("capia-media", [["capia-time"], ["capia-model"]]),
    /capia-media não pode depender de capia-model/,
  );
  assert.match(swap("capia-media", [["capia-time"], ["rusqlite"]]), /capia-media.*rusqlite/);
  // capia-assets não fala com o engine nem com o banco
  assert.match(
    swap("capia-assets", [["capia-media"], ["capia-commands"]]),
    /capia-assets não pode depender de capia-commands/,
  );
  assert.match(
    swap("capia-assets", [["capia-media"], ["capia-store"]]),
    /capia-assets não pode depender de capia-store/,
  );
  assert.match(swap("capia-assets", [["capia-media"], ["rusqlite"]]), /capia-assets.*rusqlite/);
  // o núcleo puro continua sem mídia
  assert.match(
    swap("capia-commands", [["capia-time"], ["capia-model"], ["capia-media"]]),
    /capia-commands não pode depender de capia-media/,
  );
  assert.match(
    swap("capia-model", [["capia-time"], ["capia-assets"]]),
    /capia-model não pode depender de capia-assets/,
  );
});

test("a normal (non-dev) self dependency is still a cycle", () => {
  const p = base();
  const store = p.findIndex((x) => x.name === "capia-store");
  p[store] = pkg("capia-store", [["capia-store"]]);
  assert.notDeepEqual(checkRust({ packages: p }), []);
});

test("Tauri is allowed only in the desktop shell", () => {
  const p = base();
  p[p.findIndex((x) => x.name === "capia-project")] = pkg("capia-project", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["serde"],
    ["tauri"],
  ]);
  assert.match(
    checkRust({ packages: p }).join("\n"),
    /capia-project tem dependência normal não permitida: tauri/,
  );
});

test("a new crate must be added to the matrix on purpose", () => {
  assert.match(
    checkRust({ packages: [...base(), pkg("capia-ai")] }).join("\n"),
    /capia-ai.*não está na matriz/,
  );
});

test("JS: engine-bindings must not know React or Tauri; UI must not know Tauri", () => {
  const ok = [
    { name: "@capia/engine-bindings", dependencies: {} },
    {
      name: "@capia/editor-ui",
      dependencies: { "@capia/engine-bindings": "workspace:*", react: "1" },
    },
    {
      name: "@capia/desktop",
      dependencies: { "@capia/editor-ui": "workspace:*", "@tauri-apps/api": "2" },
    },
  ];
  assert.deepEqual(checkJs(ok), []);
  const badBindings = structuredClone(ok);
  badBindings[0].dependencies = { react: "1" };
  assert.match(checkJs(badBindings).join("\n"), /engine-bindings não pode depender de react/);
  const badUi = structuredClone(ok);
  badUi[1].dependencies["@tauri-apps/api"] = "2";
  assert.match(checkJs(badUi).join("\n"), /editor-ui não pode depender de @tauri-apps\/api/);
});
