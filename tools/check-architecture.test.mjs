import assert from "node:assert/strict";
import { test } from "node:test";
import { checkDesktopTestkit, checkJs, checkRust, findCycles } from "./check-architecture.mjs";

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
  pkg("capia-secrets", [["zeroize"], ["keyring"]]),
  pkg("capia-support", [
    ["capia-secrets"],
    ["capia-store"],
    ["serde"],
    ["serde_json"],
    ["getrandom"],
    ["zip"],
  ]),
  pkg("capia-updater", [
    ["serde"],
    ["serde_json"],
    ["sha2"],
    ["semver"],
    ["ed25519-dalek"],
    ["getrandom", "dev"],
  ]),
  pkg("capia-ai", [
    ["capia-secrets"],
    ["serde"],
    ["serde_json"],
    ["sha2"],
    ["tokio"],
    ["reqwest"],
    ["async-trait"],
  ]),
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
    ["capia-secrets"],
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
  pkg("capia-fixtures", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["capia-store"],
    ["capia-project"],
    ["capia-assets"],
    ["capia-media"],
    ["capia-editor-api"],
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
  pkg("capia-intelligence", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["capia-media"],
    ["capia-assets"],
    ["capia-store"],
    ["capia-project"],
    ["capia-editor-api"],
    ["capia-ai"],
    ["capia-secrets"],
    ["serde"],
    ["serde_json"],
  ]),
  pkg("capia-devserver", [
    ["capia-editor-api"],
    ["capia-intelligence"],
    ["capia-secrets"],
    ["serde_json"],
  ]),
  pkg("capia-server", [
    ["capia-editor-api"],
    ["capia-intelligence"],
    ["capia-store"],
    ["capia-secrets"],
    ["capia-ai"],
    ["capia-commands"],
    ["serde"],
    ["serde_json"],
    ["getrandom"],
    ["sha2"],
    ["base64"],
    ["jsonschema"],
    ["tokio"],
  ]),
  pkg("capia-timeline-wasm", [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["serde"],
    ["serde_json"],
  ]),
  pkg("capia-webview-surface", [["tauri"], ["webview2-com"], ["windows"]]),
  pkg("capia-desktop", [
    ["capia-project"],
    ["capia-editor-api"],
    ["capia-webview-surface"],
    ["capia-intelligence"],
    ["capia-secrets"],
    ["capia-support"],
    ["capia-updater"],
    ["capia-store"],
    ["tauri"],
    ["tauri-plugin-dialog"],
    ["serde_json"],
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
    checkRust({ packages: [...base(), pkg("capia-newthing")] }).join("\n"),
    /capia-newthing.*não está na matriz/,
  );
});

test("JS: engine-bindings must not know React or Tauri; UI must not know Tauri", () => {
  const ok = [
    { name: "@capia/engine-bindings", dependencies: {} },
    { name: "@capia/ui-kit", dependencies: { react: "1" } },
    { name: "@capia/ui-timeline", dependencies: { "@capia/engine-bindings": "workspace:*" } },
    {
      name: "@capia/editor-ui",
      dependencies: {
        "@capia/engine-bindings": "workspace:*",
        "@capia/ui-kit": "workspace:*",
        "@capia/ui-timeline": "workspace:*",
        react: "1",
      },
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
  badUi[3].dependencies["@tauri-apps/api"] = "2";
  assert.match(checkJs(badUi).join("\n"), /editor-ui não pode depender de @tauri-apps\/api/);
});

test("AI/provider boundaries (Fase 4): providers never see the project; the core never sees providers", () => {
  const swap = (name, deps) => {
    const p = base();
    const i = p.findIndex((x) => x.name === name);
    p[i] = pkg(name, deps);
    return checkRust({ packages: p }).join("\n");
  };
  assert.match(
    swap("capia-ai", [["capia-secrets"], ["capia-project"]]),
    /capia-ai não pode depender de capia-project/,
  );
  assert.match(swap("capia-ai", [["capia-model"]]), /capia-ai não pode depender de capia-model/);
  assert.match(
    swap("capia-secrets", [["capia-ai"]]),
    /capia-secrets não pode depender de capia-ai/,
  );
  assert.match(
    swap("capia-project", [["capia-ai"]]),
    /capia-project não pode depender de capia-ai/,
  );
  assert.match(
    swap("capia-editor-api", [["capia-intelligence"]]),
    /capia-editor-api.*capia-intelligence/,
  );
  assert.match(swap("capia-commands", [["capia-time"], ["reqwest"]]), /reqwest/);
});

test("o cérebro de E2E só existe como feature opt-in do desktop", async () => {
  const { mkdtempSync, mkdirSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const mk = (toml) => {
    const root = mkdtempSync(join(tmpdir(), "arch-"));
    mkdirSync(join(root, "apps/desktop/src-tauri"), { recursive: true });
    writeFileSync(join(root, "apps/desktop/src-tauri/Cargo.toml"), toml);
    return root;
  };
  const ok = '[features]\ne2e-testkit = ["capia-intelligence/testkit"]\n';
  assert.deepEqual(checkDesktopTestkit(mk(ok)), []);
  // testkit ligado na dependência de produto
  assert.ok(
    checkDesktopTestkit(mk(ok + 'capia-intelligence = { features = ["testkit"] }\n')).length > 0,
  );
  // feature padrão
  assert.ok(checkDesktopTestkit(mk(ok + 'default = ["e2e-testkit"]\n')).length > 0);
  // feature removida
  assert.ok(checkDesktopTestkit(mk("[features]\n")).length > 0);
});

test("capia-fixtures é DEV-ONLY: só como dev-dependency, sem ciclo de produto (Fase 6 D-1)", () => {
  const swap = (name, deps) => {
    const p = base();
    p[p.findIndex((x) => x.name === name)] = pkg(name, deps);
    return checkRust({ packages: p }).join("\n");
  };
  const projectDeps = [
    ["capia-time"],
    ["capia-model"],
    ["capia-commands"],
    ["capia-store"],
    ["serde"],
    ["serde_json"],
  ];
  // dev-dependency de capia-project (o ciclo project ⇄ fixtures é de teste e é permitido)
  assert.deepEqual(swap("capia-project", [...projectDeps, ["capia-fixtures", "dev"]]), "");
  // dependência normal/build de produto é proibida
  assert.match(
    swap("capia-project", [...projectDeps, ["capia-fixtures"]]),
    /capia-project depende de capia-fixtures como normal/,
  );
  assert.match(
    swap("capia-cli", [["capia-project"], ["capia-fixtures"]]),
    /capia-cli depende de capia-fixtures como normal/,
  );
  assert.match(
    swap("capia-store", [["capia-fixtures", "dev"]]),
    /capia-store não pode depender de capia-fixtures/,
  );
  // a fixture não pode conhecer IA/servidor/UI
  assert.match(
    swap("capia-fixtures", [["capia-project"], ["capia-ai"]]),
    /capia-fixtures não pode depender de capia-ai/,
  );
  assert.match(swap("capia-fixtures", [["capia-project"], ["reqwest"]]), /capia-fixtures.*reqwest/);
});
