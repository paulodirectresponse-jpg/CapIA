import { describe, expect, test } from "bun:test";
import { decideAudioCodec, ExportCodecError } from "../audio-codec";

describe("decideAudioCodec", () => {
	test("webm sempre Opus, sem fallback", () => {
		expect(
			decideAudioCodec({
				format: "webm",
				aacSupported: false,
				allowOpusFallback: false,
			}),
		).toEqual({ codec: "opus" });
	});
	test("mp4 com AAC disponível usa AAC", () => {
		expect(
			decideAudioCodec({
				format: "mp4",
				aacSupported: true,
				allowOpusFallback: false,
			}),
		).toEqual({ codec: "aac" });
	});
	test("mp4 sem AAC falha por padrão (nunca troca em silêncio)", () => {
		expect(() =>
			decideAudioCodec({
				format: "mp4",
				aacSupported: false,
				allowOpusFallback: false,
			}),
		).toThrow(ExportCodecError);
	});
	test("mp4 sem AAC com permissão explícita usa Opus e registra o fallback", () => {
		expect(
			decideAudioCodec({
				format: "mp4",
				aacSupported: false,
				allowOpusFallback: true,
			}),
		).toEqual({ codec: "opus", fallbackFrom: "aac" });
	});
});
