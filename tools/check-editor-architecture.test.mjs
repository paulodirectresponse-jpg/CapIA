import assert from "node:assert/strict";
import { test } from "node:test";
import { checkFile, importsOf } from "./check-editor-architecture.mjs";

test("importsOf reads static and dynamic imports", () => {
  assert.deepEqual(importsOf('import a from "x"; const b = await import("y"); export * from "z";'), ["x", "y", "z"]);
});

test("agent-surface may not import UI or storage services", () => {
  const bad = checkFile({ rel: "agent-surface/surface.ts", source: 'import { X } from "@/components/ui/button";\nimport s from "@/services/storage/service";' });
  assert.equal(bad.length, 2);
});

test("agent-surface may import the domain", () => {
  const ok = checkFile({ rel: "agent-surface/surface.ts", source: 'import { EditorCore } from "@/core";\nimport { BatchCommand } from "@/commands/batch-command";' });
  assert.deepEqual(ok, []);
});

test("only app/ and tests import @/agent-surface outside the module", () => {
  assert.equal(checkFile({ rel: "timeline/x.ts", source: 'import a from "@/agent-surface";' }).length >= 1, true);
  assert.deepEqual(checkFile({ rel: "app/editor/page.tsx", source: 'import a from "@/agent-surface/bridge/page-bridge";' }), []);
});

test("SaaS imports are forbidden", () => {
  assert.equal(checkFile({ rel: "app/x.ts", source: 'import { auth } from "better-auth";' }).length, 1);
});

test("commands cannot import components", () => {
  assert.equal(checkFile({ rel: "commands/x.ts", source: 'import b from "@/components/ui/button";' }).length, 1);
});
