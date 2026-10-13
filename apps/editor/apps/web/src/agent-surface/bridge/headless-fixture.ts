/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- the headless host has no storage: only the fields the domain reads are provided */
import { EditorCore } from "@/core";
import type { MediaAsset } from "@/media/types";
import type { TProject } from "@/project/types";
import { buildDefaultScene } from "@/timeline/scenes";
import { AgentSurface } from "../surface";

export interface FixtureAsset {
	id: string;
	name: string;
	type: "video" | "image" | "audio";
	durationSeconds?: number;
}

/**
 * Boots a headless EditorCore (no DOM, no browser) with one project/sequence and a list of
 * catalog assets. Used by the stdio host and by cross-language integration tests; the
 * domain code is exactly the one the UI runs.
 */
export function openHeadlessProject({
	assets,
	name = "Headless project",
}: {
	assets: FixtureAsset[];
	name?: string;
}): AgentSurface {
	EditorCore.reset();
	const editor = EditorCore.getInstance();
	editor.save.pause();
	const scene = buildDefaultScene({ name: "Main scene", isMain: true });
	editor.project.setActiveProject({
		// The headless host has no storage; only the fields the domain reads are provided.
		project: {
			id: "headless-project",
			metadata: {
				id: "headless-project",
				name,
				thumbnail: "",
				duration: 0,
				createdAt: new Date(0),
				updatedAt: new Date(0),
			},
			scenes: [scene],
			currentSceneId: scene.id,
			settings: {
				fps: { numerator: 30, denominator: 1 },
				canvasSize: { width: 1280, height: 720 },
				canvasSizeMode: "preset",
				lastCustomCanvasSize: null,
				originalCanvasSize: { width: 1280, height: 720 },
				background: { type: "color", color: "#000000" },
			},
			version: 31,
		} as unknown as TProject,
	});
	editor.scenes.initializeScenes({ scenes: [scene] });
	editor.media.setAssets({
		assets: assets.map(
			(asset) =>
				({
					id: asset.id,
					name: asset.name,
					type: asset.type,
					duration: asset.durationSeconds,
					file: new File([], asset.name),
				}) as unknown as MediaAsset,
		),
	});
	return new AgentSurface(editor);
}
