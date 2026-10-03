import assert from "node:assert/strict";
import { test } from "node:test";
import { firstDifference } from "./check-wasm-parity.mjs";

test("saídas idênticas não têm diferença", () => {
  assert.equal(firstDifference("a\nb\n", "a\nb\n"), null);
});

test("aponta a primeira linha divergente", () => {
  assert.deepEqual(firstDifference("a\nb\nc", "a\nX\nc"), { line: 2, native: "b", wasm: "X" });
});

test("tamanhos diferentes divergem no fim", () => {
  assert.deepEqual(firstDifference("a\nb", "a"), { line: 2, native: "b", wasm: "<fim>" });
});
