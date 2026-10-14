import { EditorCore } from "@/core";
import {
	Command,
	CommandError,
	type CommandResult,
} from "@/commands/base-command";
import {
	DeleteElementsCommand,
	InsertElementCommand,
	MoveElementCommand,
	SplitElementsCommand,
	UpdateElementsCommand,
} from "@/commands/timeline";
import {
	buildElementFromMedia,
	buildTextElement,
} from "@/timeline/element-utils";
import { DEFAULT_NEW_ELEMENT_DURATION } from "@/timeline/creation";
import type { SceneTracks, TimelineElement } from "@/timeline";
import {
	fingerprintsEqual,
	takeFingerprint,
} from "@/core/managers/document-fingerprint";
import { mediaTime, mediaTimeFromSeconds } from "@/wasm";
import type { Operation } from "./types";

export function ticks({
	value,
	field,
}: {
	value: number;
	field: string;
}): ReturnType<typeof mediaTime> {
	if (!Number.isSafeInteger(value)) {
		throw new CommandError({
			code: "INVALID_OPERATION",
			message: `${field} must be an integer tick count, got ${String(value)}`,
			details: { field },
		});
	}
	return mediaTime({ ticks: value });
}

export function locateClip(clipId: string): {
	trackId: string;
	element: TimelineElement;
} | null {
	const tracks = EditorCore.getInstance().scenes.getActiveScene().tracks;
	for (const track of [...tracks.overlay, tracks.main, ...tracks.audio]) {
		const element = (track.elements as TimelineElement[]).find(
			(candidate) => candidate.id === clipId,
		);
		if (element) {
			return { trackId: track.id, element };
		}
	}
	return null;
}

/** Plan-scoped names → clip ids (created by earlier operations of the same plan). */
export type PlanRefs = Map<string, string>;

function resolveClipId({
	clipId,
	clipRef,
	refs,
}: {
	clipId?: string;
	clipRef?: string;
	refs: PlanRefs;
}): string {
	if ((clipId === undefined) === (clipRef === undefined)) {
		throw new CommandError({
			code: "INVALID_OPERATION",
			message: "exactly one of clipId or clipRef is required",
			details: {},
		});
	}
	if (clipId !== undefined) return clipId;
	const resolved = clipRef === undefined ? undefined : refs.get(clipRef);
	if (resolved === undefined) {
		throw new CommandError({
			code: "NOT_FOUND",
			message: `unknown clipRef: ${String(clipRef)}`,
			details: { clipRef },
		});
	}
	return resolved;
}

function requireClip(clipId: string): {
	trackId: string;
	element: TimelineElement;
} {
	const found = locateClip(clipId);
	if (!found) {
		throw new CommandError({
			code: "NOT_FOUND",
			message: `clip not found: ${clipId}`,
			details: { clipId },
		});
	}
	return found;
}

function buildInner({
	operation,
	refs,
}: {
	operation: Operation;
	refs: PlanRefs;
}): Command {
	const editor = EditorCore.getInstance();
	switch (operation.type) {
		case "insert_clip": {
			const asset = editor.media
				.getAssets()
				.find((candidate) => candidate.id === operation.mediaId);
			if (!asset) {
				throw new CommandError({
					code: "NOT_FOUND",
					message: `asset not found: ${operation.mediaId}`,
					details: {
						mediaId: operation.mediaId,
					},
				});
			}
			const element = buildElementFromMedia({
				mediaId: asset.id,
				mediaType: asset.type,
				name: asset.name,
				duration:
					asset.duration != null
						? mediaTimeFromSeconds({ seconds: asset.duration })
						: DEFAULT_NEW_ELEMENT_DURATION,
				startTime: ticks({ value: operation.startTicks, field: "startTicks" }),
			});
			return new InsertElementCommand({
				element,
				placement: operation.trackId
					? { mode: "explicit", trackId: operation.trackId }
					: { mode: "auto" },
			});
		}
		case "split_clip": {
			const clipId = resolveClipId({
				clipId: operation.clipId,
				clipRef: operation.clipRef,
				refs,
			});
			const { trackId } = requireClip(clipId);
			return new SplitElementsCommand({
				elements: [{ trackId, elementId: clipId }],
				splitTime: ticks({ value: operation.atTicks, field: "atTicks" }),
				retainSide: operation.retainSide ?? "both",
			});
		}
		case "move_clip": {
			const clipId = resolveClipId({
				clipId: operation.clipId,
				clipRef: operation.clipRef,
				refs,
			});
			const { trackId } = requireClip(clipId);
			return new MoveElementCommand({
				moves: [
					{
						sourceTrackId: trackId,
						targetTrackId: operation.targetTrackId ?? trackId,
						elementId: clipId,
						newStartTime: ticks({
							value: operation.newStartTicks,
							field: "newStartTicks",
						}),
					},
				],
			});
		}
		case "trim_clip": {
			const clipId = resolveClipId({
				clipId: operation.clipId,
				clipRef: operation.clipRef,
				refs,
			});
			const { trackId, element } = requireClip(clipId);
			const patch: Partial<TimelineElement> = {
				trimStart:
					operation.trimStartTicks !== undefined
						? ticks({
								value: operation.trimStartTicks,
								field: "trimStartTicks",
							})
						: element.trimStart,
				trimEnd:
					operation.trimEndTicks !== undefined
						? ticks({ value: operation.trimEndTicks, field: "trimEndTicks" })
						: element.trimEnd,
			};
			if (operation.startTicks !== undefined) {
				patch.startTime = ticks({
					value: operation.startTicks,
					field: "startTicks",
				});
			}
			if (operation.durationTicks !== undefined) {
				patch.duration = ticks({
					value: operation.durationTicks,
					field: "durationTicks",
				});
			}
			return new UpdateElementsCommand({
				updates: [{ trackId, elementId: clipId, patch }],
			});
		}
		case "delete_clip": {
			const clipId = resolveClipId({
				clipId: operation.clipId,
				clipRef: operation.clipRef,
				refs,
			});
			const { trackId } = requireClip(clipId);
			return new DeleteElementsCommand({
				elements: [{ trackId, elementId: clipId }],
			});
		}
		case "insert_text": {
			if (typeof operation.text !== "string" || operation.text.length === 0) {
				throw new CommandError({
					code: "INVALID_OPERATION",
					message: "insert_text requires a non-empty text",
					details: { field: "text" },
				});
			}
			const element = buildTextElement({
				raw: {
					duration: ticks({
						value: operation.durationTicks,
						field: "durationTicks",
					}),
					params: { content: operation.text, ...(operation.params ?? {}) },
				},
				startTime: ticks({ value: operation.startTicks, field: "startTicks" }),
			});
			return new InsertElementCommand({
				element,
				placement: operation.trackId
					? { mode: "explicit", trackId: operation.trackId }
					: { mode: "auto" },
			});
		}
		case "set_clip_params": {
			const clipId = resolveClipId({
				clipId: operation.clipId,
				clipRef: operation.clipRef,
				refs,
			});
			const { trackId, element } = requireClip(clipId);
			const known = Object.keys(element.params ?? {});
			const unknown = Object.keys(operation.params).filter(
				(key) => !known.includes(key),
			);
			if (unknown.length > 0) {
				throw new CommandError({
					code: "INVALID_OPERATION",
					message: `unknown param(s) for ${element.type} clip: ${unknown.join(", ")}`,
					details: { unknown, known },
				});
			}
			return new UpdateElementsCommand({
				updates: [
					{
						trackId,
						elementId: clipId,
						patch: { params: { ...element.params, ...operation.params } },
					},
				],
			});
		}
		case "set_clip_enabled": {
			const clipId = resolveClipId({
				clipId: operation.clipId,
				clipRef: operation.clipRef,
				refs,
			});
			const { trackId, element } = requireClip(clipId);
			// Áudio não tem `hidden`: "desabilitar" = mutar (params.muted).
			const isAudio = element.type === "audio";
			const currentlyEnabled = isAudio
				? element.params?.muted !== true
				: !("hidden" in element && element.hidden === true);
			if (currentlyEnabled === operation.enabled) {
				throw new CommandError({
					code: "NO_EFFECT",
					message: `clip ${clipId} is already ${operation.enabled ? "enabled" : "disabled"}`,
					details: { clipId },
				});
			}
			return new UpdateElementsCommand({
				updates: [
					{
						trackId,
						elementId: clipId,
						patch: isAudio
							? { params: { ...element.params, muted: !operation.enabled } }
							: { hidden: !operation.enabled },
					},
				],
			});
		}
	}
}

/**
 * One plan operation as a Command. It resolves lazily (so an operation may target a
 * clip created by an earlier operation of the same plan), delegates to the SAME domain
 * command the UI uses, and fails loudly (NO_EFFECT) instead of silently doing nothing.
 */
export class PlannedOperation extends Command {
	private inner: Command | null = null;

	readonly operation: Operation;
	private readonly refs: PlanRefs;

	constructor({ operation, refs }: { operation: Operation; refs: PlanRefs }) {
		super();
		this.operation = operation;
		this.refs = refs;
	}

	execute(): CommandResult | undefined {
		const editor = EditorCore.getInstance();
		const before = takeFingerprint(editor);
		try {
			const tracksBefore = editor.scenes.getActiveScene().tracks;
			this.inner = buildInner({ operation: this.operation, refs: this.refs });
			const result = this.inner.execute();
			this.registerRefs(tracksBefore);
			if (fingerprintsEqual({ a: before, b: takeFingerprint(editor) })) {
				throw new CommandError({
					code: "NO_EFFECT",
					message: `operation ${this.operation.operationId} (${this.operation.type}) had no effect`,
					details: { operationId: this.operation.operationId },
				});
			}
			return result;
		} catch (error) {
			if (error instanceof CommandError) {
				throw new CommandError({
					code: error.code,
					message: error.message,
					details: {
						...error.details,
						operationId: this.operation.operationId,
					},
				});
			}
			throw error;
		}
	}

	/** Names clips created by this operation (insert → ref, split → rightRef). */
	private registerRefs(before: SceneTracks): void {
		const op = this.operation;
		const name =
			op.type === "insert_clip" || op.type === "insert_text"
				? op.ref
				: op.type === "split_clip"
					? op.rightRef
					: undefined;
		if (name === undefined) return;
		const known = new Set(
			[...before.overlay, before.main, ...before.audio].flatMap((track) =>
				(track.elements as TimelineElement[]).map((element) => element.id),
			),
		);
		const after = EditorCore.getInstance().scenes.getActiveScene().tracks;
		for (const track of [...after.overlay, after.main, ...after.audio]) {
			const created = (track.elements as TimelineElement[]).find(
				(element) => !known.has(element.id),
			);
			if (created) {
				this.refs.set(name, created.id);
				return;
			}
		}
	}

	undo(): void {
		this.inner?.undo();
	}

	redo(): CommandResult | undefined {
		return this.inner ? this.inner.redo() : this.execute();
	}
}
