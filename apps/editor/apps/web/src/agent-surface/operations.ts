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
import { buildElementFromMedia } from "@/timeline/element-utils";
import { DEFAULT_NEW_ELEMENT_DURATION } from "@/timeline/creation";
import type { TimelineElement } from "@/timeline";
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

function buildInner(operation: Operation): Command {
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
			const { trackId } = requireClip(operation.clipId);
			return new SplitElementsCommand({
				elements: [{ trackId, elementId: operation.clipId }],
				splitTime: ticks({ value: operation.atTicks, field: "atTicks" }),
				retainSide: operation.retainSide ?? "both",
			});
		}
		case "move_clip": {
			const { trackId } = requireClip(operation.clipId);
			return new MoveElementCommand({
				moves: [
					{
						sourceTrackId: trackId,
						targetTrackId: operation.targetTrackId ?? trackId,
						elementId: operation.clipId,
						newStartTime: ticks({
							value: operation.newStartTicks,
							field: "newStartTicks",
						}),
					},
				],
			});
		}
		case "trim_clip": {
			const { trackId, element } = requireClip(operation.clipId);
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
				updates: [{ trackId, elementId: operation.clipId, patch }],
			});
		}
		case "delete_clip": {
			const { trackId } = requireClip(operation.clipId);
			return new DeleteElementsCommand({
				elements: [{ trackId, elementId: operation.clipId }],
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

	constructor(readonly operation: Operation) {
		super();
	}

	execute(): CommandResult | undefined {
		const editor = EditorCore.getInstance();
		const before = takeFingerprint(editor);
		try {
			this.inner = buildInner(this.operation);
			const result = this.inner.execute();
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

	undo(): void {
		this.inner?.undo();
	}

	redo(): CommandResult | undefined {
		return this.inner ? this.inner.redo() : this.execute();
	}
}
