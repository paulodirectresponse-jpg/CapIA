/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- fixtures build partial domain objects (project, media assets) for headless tests */
import { EditorCore } from "@/core";
import { buildDefaultScene } from "@/timeline/scenes";
import type { MediaAsset } from "@/media/types";
import type { TProject } from "@/project/types";
import {
	AGENT_SURFACE_SCHEMA_VERSION,
	AgentSurface,
	type ClipSummary,
	type Operation,
	type Plan,
} from "@/agent-surface";

export const AGENT = { kind: "agent", id: "test" } as const;
export const T = 120_000; // ticks per second at baseline

export function createFixture() {
	EditorCore.reset();
	const editor = EditorCore.getInstance();
	editor.save.pause();
	const scene = buildDefaultScene({ name: "Main scene", isMain: true });
	const project = {
		id: "p1",
		metadata: {
			id: "p1",
			name: "Fixture",
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
	} as unknown as TProject;
	editor.project.setActiveProject({ project });
	editor.scenes.initializeScenes({ scenes: [scene] });
	const video = {
		id: "asset-video",
		name: "main.webm",
		type: "video",
		duration: 10,
		width: 1280,
		height: 720,
		file: new File([], "main.webm"),
	} as unknown as MediaAsset;
	const audio = {
		id: "asset-audio",
		name: "music.ogg",
		type: "audio",
		duration: 30,
		file: new File([], "music.ogg"),
	} as unknown as MediaAsset;
	editor.media.setAssets({ assets: [video, audio] });
	return { editor, surface: new AgentSurface(editor) };
}

export function makePlan({
	revision,
	planId,
	operations,
}: {
	revision: number;
	planId: string;
	operations: Operation[];
}): Plan {
	return {
		schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
		planId,
		actor: AGENT,
		baseRevision: revision,
		operations,
	};
}

export function allClips(surface: AgentSurface): ClipSummary[] {
	const result = surface.inspect_tracks();
	if (!result.ok) throw new Error(result.error.message);
	return result.tracks.flatMap((track) => track.clips);
}
