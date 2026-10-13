import type { EditorCore } from "@/core";
import type { SceneTracks, TScene } from "@/timeline";

/**
 * Cheap semantic fingerprint of the editable document. Immutable updates keep the
 * identity of untouched elements, so equality is O(clips) pointer compares even
 * when a command rebuilt every track object without changing anything.
 */
export interface DocumentFingerprint {
	scenes: TScene[];
	assets: unknown;
	settings: unknown;
	projectName: string | null;
}

type AnyTrack =
	| SceneTracks["main"]
	| SceneTracks["overlay"][number]
	| SceneTracks["audio"][number];

export function takeFingerprint(editor: EditorCore): DocumentFingerprint {
	const project = editor.project.getActiveOrNull();
	return {
		scenes: editor.scenes.getScenes(),
		assets: editor.media.getAssets(),
		settings: project?.settings ?? null,
		projectName: project?.metadata.name ?? null,
	};
}

function sameElements({
	a,
	b,
}: {
	a: readonly unknown[];
	b: readonly unknown[];
}): boolean {
	return a.length === b.length && a.every((element, i) => element === b[i]);
}

function sameTrack({ a, b }: { a: AnyTrack; b: AnyTrack }): boolean {
	if (a === b) return true;
	const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
	for (const key of keys) {
		if (key === "elements") continue;
		if (Reflect.get(a, key) !== Reflect.get(b, key)) return false;
	}
	return sameElements({ a: a.elements, b: b.elements });
}

export function sameSceneTracks({
	a,
	b,
}: {
	a: SceneTracks;
	b: SceneTracks;
}): boolean {
	if (a === b) return true;
	if (
		a.overlay.length !== b.overlay.length ||
		a.audio.length !== b.audio.length
	) {
		return false;
	}
	return (
		sameTrack({ a: a.main, b: b.main }) &&
		a.overlay.every((track, i) => sameTrack({ a: track, b: b.overlay[i] })) &&
		a.audio.every((track, i) => sameTrack({ a: track, b: b.audio[i] }))
	);
}

function sameScene({ a, b }: { a: TScene; b: TScene }): boolean {
	return (
		a === b ||
		(a.id === b.id &&
			a.name === b.name &&
			a.isMain === b.isMain &&
			a.bookmarks === b.bookmarks &&
			sameSceneTracks({ a: a.tracks, b: b.tracks }))
	);
}

export function fingerprintsEqual({
	a,
	b,
}: {
	a: DocumentFingerprint;
	b: DocumentFingerprint;
}): boolean {
	return (
		a.assets === b.assets &&
		a.settings === b.settings &&
		a.projectName === b.projectName &&
		a.scenes.length === b.scenes.length &&
		a.scenes.every((scene, i) => sameScene({ a: scene, b: b.scenes[i] }))
	);
}
