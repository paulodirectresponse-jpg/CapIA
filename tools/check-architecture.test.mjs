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
  pkg("capia-project", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["serde"],
    ["serde_json", "dev"],
  ]),
  pkg("capia-desktop", [["capia-project"], ["tauri"], ["tauri-build", "build"]]),
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

test("Tauri is allowed only in the desktop shell", () => {
  const p = base();
  p[3] = pkg("capia-project", [
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
