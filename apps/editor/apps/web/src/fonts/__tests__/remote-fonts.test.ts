import { describe, expect, test } from "bun:test";
import { isRemoteFontsEnabled, setRemoteFontsEnabled } from "../remote-fonts";

function memoryStorage() {
	const map = new Map<string, string>();
	return {
		getItem: (key: string) => map.get(key) ?? null,
		setItem: (key: string, value: string) => void map.set(key, value),
		removeItem: (key: string) => void map.delete(key),
	};
}

describe("remote fonts opt-in", () => {
	test("desabilitado por padrão", () => {
		expect(
			isRemoteFontsEnabled({ storage: memoryStorage(), envFlag: undefined }),
		).toBe(false);
	});
	test("habilita e desabilita explicitamente", () => {
		const storage = memoryStorage();
		setRemoteFontsEnabled({ enabled: true, storage });
		expect(isRemoteFontsEnabled({ storage, envFlag: undefined })).toBe(true);
		setRemoteFontsEnabled({ enabled: false, storage });
		expect(isRemoteFontsEnabled({ storage, envFlag: undefined })).toBe(false);
	});
	test("storage que lança não habilita", () => {
		const storage = {
			getItem: () => {
				throw new Error("blocked");
			},
		};
		expect(isRemoteFontsEnabled({ storage, envFlag: undefined })).toBe(false);
	});
	test("flag de ambiente habilita (builds de dev)", () => {
		expect(isRemoteFontsEnabled({ storage: undefined, envFlag: "1" })).toBe(
			true,
		);
	});
});
