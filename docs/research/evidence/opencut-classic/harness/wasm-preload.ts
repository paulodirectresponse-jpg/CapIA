// Harness-only: Bun cannot ESM-import .wasm (bundler-target wasm-bindgen output).
// Re-implement opencut_wasm.js with explicit WebAssembly.instantiate. Does NOT change any test.
import { plugin } from "bun";
plugin({
  name: "opencut-wasm-esm-shim",
  setup(build) {
    build.onLoad({ filter: /opencut-wasm[\\/]opencut_wasm\.js$/ }, async (args) => {
      const dir = args.path.replace(/opencut_wasm\.js$/, "");
      const src = `
import * as bg from ${JSON.stringify(dir + "opencut_wasm_bg.js")};
import { readFileSync } from "node:fs";
const bytes = readFileSync(${JSON.stringify(dir + "opencut_wasm_bg.wasm")});
const { instance } = await WebAssembly.instantiate(bytes, { "./opencut_wasm_bg.js": bg });
bg.__wbg_set_wasm(instance.exports);
instance.exports.__wbindgen_start();
export * from ${JSON.stringify(dir + "opencut_wasm_bg.js")};
`;
      return { contents: src, loader: "js" };
    });
  },
});
