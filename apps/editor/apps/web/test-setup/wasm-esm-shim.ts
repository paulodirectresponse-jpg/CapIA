/**
 * Test harness only (CapIA). Bun cannot ESM-import `.wasm` (the wasm-bindgen
 * "bundler" target emits `import * as wasm from "./x_bg.wasm"`, which webpack /
 * Turbopack resolve and Bun does not). This plugin re-creates opencut_wasm.js
 * with an explicit WebAssembly.instantiate. It changes no test and no product code.
 */
import { plugin } from "bun";

plugin({
	name: "opencut-wasm-esm-shim",
	setup(build) {
		build.onLoad({ filter: /opencut-wasm[\\/]opencut_wasm\.js$/ }, (args) => {
			const dir = args.path.replace(/opencut_wasm\.js$/, "");
			const bg = JSON.stringify(`${dir}opencut_wasm_bg.js`);
			const wasmFile = JSON.stringify(`${dir}opencut_wasm_bg.wasm`);
			return {
				loader: "js",
				contents: `
import * as bg from ${bg};
import { readFileSync } from "node:fs";
const bytes = readFileSync(${wasmFile});
const { instance } = await WebAssembly.instantiate(bytes, { "./opencut_wasm_bg.js": bg });
bg.__wbg_set_wasm(instance.exports);
instance.exports.__wbindgen_start();
export * from ${bg};
`,
			};
		});
	},
});
