/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- fixtures: Canvas/Renderer/File falsos só para identidade de objeto */
import { describe, expect, test } from "bun:test";
import type { CanvasRenderer } from "../canvas-renderer";
import { buildFrameDescriptor } from "../compositor/frame-descriptor";
import {
	decideExternalTexture,
	type ExternalCacheState,
} from "../compositor/texture-cache-policy";
import type { TextureUploadDescriptor } from "../compositor/types";
import { BlurBackgroundNode } from "../nodes/blur-background-node";
import { ImageNode } from "../nodes/image-node";
import { RootNode } from "../nodes/root-node";
import { VideoNode } from "../nodes/video-node";
import {
	STATIC_SOURCE_REVISION,
	videoFrameRevision,
} from "../../video-cache/revision";

function fakeCanvas(): CanvasImageSource {
	return {} as CanvasImageSource;
}

const SIZE = { width: 640, height: 360 };
const renderer = { width: 1280, height: 720 } as unknown as CanvasRenderer;

function state(partial: Partial<ExternalCacheState> = {}): ExternalCacheState {
	return {
		source: fakeCanvas(),
		revision: "0.5:0.033",
		...SIZE,
		...partial,
	};
}

describe("política do cache de textura externa", () => {
	test("A: mesmo Canvas + mesma revisão => HIT", () => {
		const previous = state();
		expect(decideExternalTexture({ previous, next: { ...previous } })).toEqual({
			hit: true,
			reason: "same",
		});
	});

	test("B: mesmo Canvas + revisão diferente => novo upload", () => {
		const previous = state();
		expect(
			decideExternalTexture({
				previous,
				next: { ...previous, revision: "2.5:0.033" },
			}),
		).toEqual({ hit: false, reason: "revision" });
	});

	test("C: outro objeto Canvas => upload", () => {
		const previous = state();
		expect(
			decideExternalTexture({
				previous,
				next: { ...previous, source: fakeCanvas() },
			}),
		).toEqual({ hit: false, reason: "source" });
	});

	test("D: imagem estática repetida => HIT (cache preservado)", () => {
		const image = fakeCanvas();
		const previous = state({ source: image, revision: STATIC_SOURCE_REVISION });
		for (let i = 0; i < 5; i++) {
			expect(
				decideExternalTexture({
					previous,
					next: { ...previous },
				}).hit,
			).toBe(true);
		}
	});

	test("tamanho diferente => upload; primeiro uso => upload", () => {
		const previous = state();
		expect(
			decideExternalTexture({ previous, next: { ...previous, width: 1 } })
				.reason,
		).toBe("size");
		expect(
			decideExternalTexture({ previous: undefined, next: previous }).reason,
		).toBe("first");
	});
});

describe("seek aleatório com Canvas reaproveitado pelo decoder", () => {
	/** Simula o compositor: guarda só o último estado enviado (como `cache.get(texture.id)`). */
	function run(
		frames: Array<{ canvas: CanvasImageSource; timestamp: number }>,
	) {
		let cached: ExternalCacheState | undefined;
		const uploads: number[] = [];
		for (const frame of frames) {
			const next = {
				source: frame.canvas,
				revision: videoFrameRevision({
					timestamp: frame.timestamp,
					duration: 1 / 30,
				}),
				...SIZE,
			};
			if (!decideExternalTexture({ previous: cached, next }).hit) {
				uploads.push(frame.timestamp);
				cached = next;
			}
		}
		return uploads;
	}

	test("E: o MESMO Canvas com timestamps diferentes sobe a cada quadro novo", () => {
		const pooled = fakeCanvas();
		const uploads = run(
			[0.5, 2.5, 3.5, 5.5].map((timestamp) => ({ canvas: pooled, timestamp })),
		);
		expect(uploads).toEqual([0.5, 2.5, 3.5, 5.5]);
	});

	test("F: seek para trás reenvia o quadro correto; mesmo quadro repetido não", () => {
		const pooled = fakeCanvas();
		const uploads = run(
			[0.5, 2.5, 3.5, 5.5, 0.5, 3.5, 3.5, 2.5].map((timestamp) => ({
				canvas: pooled,
				timestamp,
			})),
		);
		expect(uploads).toEqual([0.5, 2.5, 3.5, 5.5, 0.5, 3.5, 2.5]);
	});

	test("revisão é determinada pelo conteúdo (timestamp+duração), não pelo objeto", () => {
		expect(videoFrameRevision({ timestamp: 1, duration: 0.5 })).toBe("1:0.5");
		expect(videoFrameRevision({ timestamp: 1, duration: 0.5 })).not.toBe(
			videoFrameRevision({ timestamp: 1.5, duration: 0.5 }),
		);
	});
});

function externalTexture({
	textures,
	suffix,
}: {
	textures: TextureUploadDescriptor[];
	suffix: string;
}) {
	const found = textures.find((t) => t.id.endsWith(suffix));
	if (!found) throw new Error(`textura ${suffix} não encontrada`);
	return found;
}

async function describeVideo({
	canvas,
	revision,
}: {
	canvas: CanvasImageSource;
	revision: string | undefined;
}) {
	const node = new VideoNode({} as never);
	node.resolved = {
		localTime: 0,
		transform: {
			position: { x: 0, y: 0 },
			scaleX: 1,
			scaleY: 1,
			rotate: 0,
		},
		opacity: 1,
		effectPasses: [],
		source: canvas,
		sourceRevision: revision,
		sourceWidth: 640,
		sourceHeight: 360,
	} as never;
	const root = new RootNode({} as never);
	root.add(node);
	return buildFrameDescriptor({ node: root, renderer });
}

describe("descritor de frame propaga a revisão", () => {
	test("vídeo: o mesmo Canvas em quadros diferentes gera revisões diferentes", async () => {
		const pooled = fakeCanvas();
		const a = await describeVideo({ canvas: pooled, revision: "0.5:0.033" });
		const b = await describeVideo({ canvas: pooled, revision: "2.5:0.033" });
		const ta = externalTexture({ textures: a.textures, suffix: ":source" });
		const tb = externalTexture({ textures: b.textures, suffix: ":source" });
		expect(ta.kind).toBe("external");
		expect(tb.kind).toBe("external");
		if (ta.kind === "external" && tb.kind === "external") {
			expect(ta.source).toBe(tb.source);
			expect(ta.revision).not.toBe(tb.revision);
		}
	});

	test("fonte sem revisão (imagem/sticker) usa a revisão estática", async () => {
		const image = new ImageNode({} as never);
		image.resolved = {
			localTime: 0,
			transform: {
				position: { x: 0, y: 0 },
				scaleX: 1,
				scaleY: 1,
				rotate: 0,
			},
			opacity: 1,
			effectPasses: [],
			source: fakeCanvas(),
			sourceWidth: 100,
			sourceHeight: 100,
		} as never;
		const root = new RootNode({} as never);
		root.add(image);
		const { textures } = await buildFrameDescriptor({ node: root, renderer });
		const t = externalTexture({ textures, suffix: ":source" });
		expect(t.kind === "external" && t.revision).toBe(STATIC_SOURCE_REVISION);
	});
});

async function describeBlur({
	canvas,
	revision,
}: {
	canvas: CanvasImageSource;
	revision: string | undefined;
}) {
	const node = new BlurBackgroundNode({} as never);
	node.resolved = {
		backdropSource: { source: canvas, revision, width: 640, height: 360 },
		passes: [],
	};
	const root = new RootNode({} as never);
	root.add(node);
	return buildFrameDescriptor({ node: root, renderer });
}

describe("G: blur background com vídeo", () => {
	test("o hash do backdrop muda entre quadros mesmo com o Canvas reaproveitado", async () => {
		const pooled = fakeCanvas();
		const a = await describeBlur({ canvas: pooled, revision: "0.5:0.033" });
		const b = await describeBlur({ canvas: pooled, revision: "2.5:0.033" });
		const ta = externalTexture({
			textures: a.textures,
			suffix: ":blur-background",
		});
		const tb = externalTexture({
			textures: b.textures,
			suffix: ":blur-background",
		});
		expect(ta.kind === "rendered" && tb.kind === "rendered").toBe(true);
		if (ta.kind === "rendered" && tb.kind === "rendered") {
			expect(ta.contentHash).not.toBe(tb.contentHash);
		}
	});

	test("backdrop estático (imagem) mantém o mesmo hash => cache preservado", async () => {
		const image = fakeCanvas();
		const a = await describeBlur({ canvas: image, revision: undefined });
		const b = await describeBlur({ canvas: image, revision: undefined });
		const ta = externalTexture({
			textures: a.textures,
			suffix: ":blur-background",
		});
		const tb = externalTexture({
			textures: b.textures,
			suffix: ":blur-background",
		});
		if (ta.kind === "rendered" && tb.kind === "rendered") {
			expect(ta.contentHash).toBe(tb.contentHash);
		}
	});
});
