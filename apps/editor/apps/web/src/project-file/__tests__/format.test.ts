/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- fixtures parciais do domínio */
import { describe, expect, test } from "bun:test";
import type { TProject } from "@/project/types";
import {
	ProjectFileError,
	canonicalJson,
	mediaMatches,
	parseProjectFile,
	serializeProjectFile,
	type MediaReference,
} from "../format";

const project = {
	metadata: {
		id: "p1",
		name: "Anúncio",
		duration: 1_440_000,
		createdAt: new Date("2026-01-01T00:00:00Z"),
		updatedAt: new Date("2026-01-02T00:00:00Z"),
	},
	scenes: [
		{
			id: "s1",
			name: "Main",
			isMain: true,
			tracks: { overlay: [], audio: [] },
		},
	],
	currentSceneId: "s1",
	settings: { fps: { numerator: 30, denominator: 1 } },
	version: 31,
} as unknown as TProject;

const media: MediaReference[] = [
	{
		id: "m1",
		name: "main.mp4",
		type: "video",
		relativePath: "media/main.mp4",
		absolutePath: null,
		sizeBytes: 1234,
		sha256: "sha256:abc",
	},
];

async function saved() {
	return serializeProjectFile({
		project,
		media,
		revision: 7,
		savedAt: "2026-10-14T00:00:00Z",
	});
}

describe("project file (ADR-132)", () => {
	test("roundtrip preserva documento, mídia e revisão", async () => {
		const text = await saved();
		const file = await parseProjectFile({ text, maxSupportedEditorSchema: 31 });
		expect(file.revision).toBe(7);
		expect(file.media).toEqual(media);
		expect(file.editorSchemaVersion).toBe(31);
		const p = file.project as { metadata: { createdAt: string; name: string } };
		expect(p.metadata.name).toBe("Anúncio");
		expect(p.metadata.createdAt).toBe("2026-01-01T00:00:00.000Z");
	});

	test("serialização é determinística (mesmos bytes)", async () => {
		expect(await saved()).toBe(await saved());
	});

	test("canonicalJson ordena chaves e ignora undefined", () => {
		expect(canonicalJson({ b: 1, a: { d: undefined, c: [2, 1] } })).toBe(
			'{"a":{"c":[2,1]},"b":1}',
		);
	});

	test("adulteração é detectada pelo SHA-256", async () => {
		const text = (await saved()).replace("Anúncio", "Outro");
		await expect(
			parseProjectFile({ text, maxSupportedEditorSchema: 31 }),
		).rejects.toMatchObject({ code: "INTEGRITY" });
	});

	test("arquivo truncado/não-JSON/outro formato são rejeitados com código", async () => {
		const text = await saved();
		await expect(
			parseProjectFile({
				text: text.slice(0, 40),
				maxSupportedEditorSchema: 31,
			}),
		).rejects.toMatchObject({ code: "NOT_JSON" });
		await expect(
			parseProjectFile({
				text: '{"format":"x"}',
				maxSupportedEditorSchema: 31,
			}),
		).rejects.toMatchObject({ code: "WRONG_FORMAT" });
	});

	test("projeto de editor mais novo é recusado (sem tocar o arquivo)", async () => {
		const text = await saved();
		const err = await parseProjectFile({
			text,
			maxSupportedEditorSchema: 30,
		}).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(ProjectFileError);
		expect((err as ProjectFileError).code).toBe("UNSUPPORTED_VERSION");
	});

	test("formatVersion desconhecida é recusada", async () => {
		const text = (await saved()).replace(
			'"formatVersion":1',
			'"formatVersion":9',
		);
		await expect(
			parseProjectFile({ text, maxSupportedEditorSchema: 31 }),
		).rejects.toMatchObject({ code: "UNSUPPORTED_VERSION" });
	});

	test("relink só aceita o mesmo conteúdo, nunca por nome", () => {
		expect(
			mediaMatches({
				reference: media[0],
				sizeBytes: 1234,
				sha256: "sha256:abc",
			}),
		).toBe(true);
		expect(
			mediaMatches({
				reference: media[0],
				sizeBytes: 1234,
				sha256: "sha256:zzz",
			}),
		).toBe(false);
		expect(
			mediaMatches({
				reference: media[0],
				sizeBytes: 99,
				sha256: "sha256:abc",
			}),
		).toBe(false);
	});
});
