import type { ExportFormat } from "@/export";

export type AudioCodecDecision = {
	codec: "aac" | "opus";
	// preenchido quando o codec efetivo difere do pedido pelo formato (nunca silencioso)
	fallbackFrom?: "aac";
};

export class ExportCodecError extends Error {
	readonly code = "AAC_UNAVAILABLE";
	constructor() {
		super(
			"Este sistema não oferece o encoder AAC necessário para MP4 com áudio. " +
				"Exporte em WebM (Opus) ou permita explicitamente áudio Opus dentro do MP4 " +
				"(menos compatível com players e editores).",
		);
		this.name = "ExportCodecError";
	}
}

export function decideAudioCodec({
	format,
	aacSupported,
	allowOpusFallback,
}: {
	format: ExportFormat;
	aacSupported: boolean;
	allowOpusFallback: boolean;
}): AudioCodecDecision {
	if (format === "webm") return { codec: "opus" };
	if (aacSupported) return { codec: "aac" };
	if (allowOpusFallback) return { codec: "opus", fallbackFrom: "aac" };
	throw new ExportCodecError();
}
