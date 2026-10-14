import type { ExternalTextureDescriptor } from "./types";

export type ExternalCacheState = {
	source: CanvasImageSource;
	revision: string;
	width: number;
	height: number;
};

export type ExternalCacheDecision = {
	hit: boolean;
	reason: "first" | "source" | "revision" | "size" | "same";
};

/**
 * Textura externa só é reaproveitada se objeto, REVISÃO de conteúdo e tamanho forem iguais.
 * (Antes só objeto+tamanho: um Canvas de vídeo reaproveitado pelo decoder dava HIT com pixels novos
 * e o compositor continuava desenhando o quadro antigo.)
 */
export function decideExternalTexture({
	previous,
	next,
}: {
	previous: ExternalCacheState | undefined;
	next: Pick<
		ExternalTextureDescriptor,
		"source" | "revision" | "width" | "height"
	>;
}): ExternalCacheDecision {
	if (!previous) return { hit: false, reason: "first" };
	if (previous.source !== next.source) return { hit: false, reason: "source" };
	if (previous.revision !== next.revision) {
		return { hit: false, reason: "revision" };
	}
	if (previous.width !== next.width || previous.height !== next.height) {
		return { hit: false, reason: "size" };
	}
	return { hit: true, reason: "same" };
}
