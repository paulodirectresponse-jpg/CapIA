import assert from "node:assert/strict";
import { test } from "node:test";
import { classifyLicense, evaluate } from "./check-licenses.mjs";

test("permissive licenses are allowed", () => {
  for (const l of [
    "MIT",
    "ISC",
    "Apache-2.0",
    "BSD-3-Clause",
    "(MIT OR Apache-2.0)",
    "Apache-2.0 WITH LLVM-exception",
  ]) {
    assert.equal(classifyLicense(l), "allow", l);
  }
});

test("GPL, AGPL, non-commercial and unlicensed are denied", () => {
  for (const l of [
    "GPL-3.0-only",
    "GPL-2.0+",
    "AGPL-3.0-or-later",
    "SSPL-1.0",
    "BUSL-1.1",
    "PolyForm-Noncommercial-1.0.0",
    "CC-BY-NC-4.0",
    "UNLICENSED",
    "",
    "UNKNOWN",
  ]) {
    assert.equal(classifyLicense(l), "deny", l);
  }
});

test("weak copyleft and unknown licenses need human review, never auto-approval", () => {
  for (const l of [
    "MPL-2.0",
    "LGPL-3.0-or-later",
    "EPL-2.0",
    "SEE LICENSE IN LICENSE.md",
    "Some-Custom-1.0",
  ]) {
    assert.equal(classifyLicense(l), "review", l);
  }
});

test("OR lets us elect the permissive option; AND requires every license to pass", () => {
  assert.equal(classifyLicense("MIT OR GPL-3.0-only"), "allow");
  assert.equal(classifyLicense("MIT AND GPL-3.0-only"), "deny");
  assert.equal(classifyLicense("MIT AND MPL-2.0"), "review");
  assert.equal(classifyLicense("(MIT OR Apache-2.0) AND Unicode-3.0"), "allow");
});

test("evaluate groups packages and honours human exceptions only when listed", () => {
  const json = {
    MIT: [{ name: "a", versions: ["1.0.0"] }],
    "GPL-3.0-only": [{ name: "bad", versions: ["2.0.0"] }],
    "MPL-2.0": [{ name: "weak", versions: ["3.0.0"] }],
  };
  const plain = evaluate(json);
  assert.deepEqual(
    plain.deny.map((e) => e.id),
    ["bad@2.0.0"],
  );
  assert.deepEqual(
    plain.review.map((e) => e.id),
    ["weak@3.0.0"],
  );
  const excepted = evaluate(json, {
    "bad@2.0.0": { license: "GPL-3.0-only", reason: "x", approvedBy: "PO", date: "2026-10-02" },
  });
  assert.equal(excepted.deny.length, 0);
  assert.equal(excepted.excepted.length, 1);
});
