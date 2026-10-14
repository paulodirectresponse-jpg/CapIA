import type { EditorCore } from "@/core";
import { BatchCommand } from "@/commands/batch-command";
import { CommandError, runExecute } from "@/commands/base-command";
import type { SceneTracks, TimelineElement, TimelineTrack } from "@/timeline";
import { calculateTotalDuration } from "@/timeline";
import { deriveExecutionIds, withIdSeed } from "@/utils/id";
import { TICKS_PER_SECOND } from "@/wasm";
import { PlannedOperation, type PlanRefs, locateClip } from "./operations";
import {
	AGENT_SURFACE_SCHEMA_VERSION,
	type ApplyReport,
	type ClipDetail,
	type ClipSummary,
	type Diff,
	type HistoryInfo,
	type Operation,
	type Plan,
	type PreviewReport,
	type ProjectInfo,
	type Receipt,
	type Result,
	type SequenceInfo,
	type SurfaceError,
	type TrackSummary,
	type ValidationReport,
} from "./types";

type Failure = { ok: false; error: SurfaceError; rolledBack?: boolean };

const OPERATION_TYPES = new Set([
	"insert_clip",
	"move_clip",
	"trim_clip",
	"split_clip",
	"delete_clip",
	"insert_text",
	"set_clip_params",
	"set_clip_enabled",
]);

function fail({ code, message, ...extra }: SurfaceError): Failure {
	return { ok: false, error: { code, message, ...extra } };
}

function flattenTracks(tracks: SceneTracks): Array<{
	zone: TrackSummary["zone"];
	index: number;
	track: TimelineTrack;
}> {
	return [
		...tracks.overlay.map((track, index) => ({
			zone: "overlay" as const,
			index,
			track,
		})),
		{ zone: "main" as const, index: 0, track: tracks.main },
		...tracks.audio.map((track, index) => ({
			zone: "audio" as const,
			index,
			track,
		})),
	];
}

function summarizeClip({
	element,
	trackId,
}: {
	element: TimelineElement;
	trackId: string;
}): ClipSummary {
	const summary: ClipSummary = {
		id: element.id,
		type: element.type,
		name: element.name,
		trackId,
		startTicks: element.startTime,
		durationTicks: element.duration,
		endTicks: element.startTime + element.duration,
		trimStartTicks: element.trimStart,
		trimEndTicks: element.trimEnd,
	};
	if ("mediaId" in element && typeof element.mediaId === "string") {
		summary.mediaId = element.mediaId;
	}
	if ("retime" in element && element.retime) {
		summary.retimeRate = element.retime.rate;
	}
	return summary;
}

function summarizeTracks(tracks: SceneTracks): TrackSummary[] {
	return flattenTracks(tracks).map(({ zone, index, track }) => {
		const summary: TrackSummary = {
			id: track.id,
			zone,
			index,
			type: track.type,
			name: track.name,
			clips: (track.elements as TimelineElement[]).map((element) =>
				summarizeClip({ element, trackId: track.id }),
			),
		};
		if ("muted" in track) summary.muted = track.muted;
		if ("hidden" in track) summary.hidden = track.hidden;
		return summary;
	});
}

function indexClips(tracks: SceneTracks): Map<string, ClipSummary> {
	const map = new Map<string, ClipSummary>();
	for (const track of summarizeTracks(tracks)) {
		for (const clip of track.clips) map.set(clip.id, clip);
	}
	return map;
}

function diffTracks({
	before,
	after,
}: {
	before: SceneTracks;
	after: SceneTracks;
}): Diff {
	const a = indexClips(before);
	const b = indexClips(after);
	const diff: Diff = { added: [], removed: [], changed: [] };
	for (const [id, clip] of b) {
		const previous = a.get(id);
		if (!previous) diff.added.push(clip);
		else if (JSON.stringify(previous) !== JSON.stringify(clip)) {
			diff.changed.push({ before: previous, after: clip });
		}
	}
	for (const [id, clip] of a) {
		if (!b.has(id)) diff.removed.push(clip);
	}
	return diff;
}

function seedFor({
	plan,
	operation,
}: {
	plan: Plan;
	operation: Operation;
}): string {
	return `${plan.planId}:${operation.operationId}`;
}

/** Deterministic id of the clip an `insert_clip` operation creates. */
export function insertedClipId({
	planId,
	operationId,
}: {
	planId: string;
	operationId: string;
}): string {
	return deriveExecutionIds({ seed: `${planId}:${operationId}`, count: 1 })[0];
}

/** Deterministic id of the right-hand clip a `split_clip` operation creates. */
export function splitRightClipId({
	planId,
	operationId,
}: {
	planId: string;
	operationId: string;
}): string {
	return deriveExecutionIds({ seed: `${planId}:${operationId}`, count: 1 })[0];
}

export class AgentSurface {
	private readonly receipts = new Map<string, Receipt>();
	private readonly appliedOperationIds = new Set<string>();

	constructor(private readonly editor: EditorCore) {}

	// ---- inspect ---------------------------------------------------------

	private history(): HistoryInfo {
		return {
			canUndo: this.editor.command.canUndo(),
			canRedo: this.editor.command.canRedo(),
			entries: this.editor.command.getHistory().map((item) => ({
				index: item.index,
				command: item.command,
				actor: item.meta?.actor,
				planId: item.meta?.planId,
				operationIds: item.meta?.operationIds,
			})),
		};
	}

	inspect_project(): Result<{ project: ProjectInfo }> {
		const project = this.editor.project.getActiveOrNull();
		const active = this.editor.scenes.getActiveSceneOrNull();
		if (!project || !active)
			return fail({ code: "NOT_FOUND", message: "no active project" });
		return {
			ok: true,
			project: {
				schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
				projectId: project.metadata.id,
				name: project.metadata.name,
				revision: this.editor.revision.get(),
				ticksPerSecond: TICKS_PER_SECOND,
				fps: { ...project.settings.fps },
				canvas: { ...project.settings.canvasSize },
				activeSequenceId: active.id,
				sequences: this.editor.scenes.getScenes().map((scene) => ({
					id: scene.id,
					name: scene.name,
					isMain: scene.isMain,
				})),
				assets: this.editor.media.getAssets().map((asset) => ({
					id: asset.id,
					name: asset.name,
					type: asset.type,
					durationSeconds: asset.duration,
				})),
				history: this.history(),
			},
		};
	}

	private sceneOrFailure(sequenceId?: string) {
		const active = this.editor.scenes.getActiveSceneOrNull();
		if (!active)
			return fail({ code: "NOT_FOUND", message: "no active sequence" });
		if (sequenceId === undefined || sequenceId === active.id) return active;
		const other = this.editor.scenes
			.getScenes()
			.find((scene) => scene.id === sequenceId);
		return (
			other ??
			fail({ code: "NOT_FOUND", message: `sequence not found: ${sequenceId}` })
		);
	}

	inspect_sequence(
		args: { sequenceId?: string } = {},
	): Result<{ sequence: SequenceInfo }> {
		const scene = this.sceneOrFailure(args.sequenceId);
		if ("ok" in scene) return scene;
		const flat = flattenTracks(scene.tracks);
		return {
			ok: true,
			sequence: {
				schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
				sequenceId: scene.id,
				name: scene.name,
				isMain: scene.isMain,
				revision: this.editor.revision.get(),
				durationTicks: calculateTotalDuration({ tracks: scene.tracks }),
				trackCount: flat.length,
				clipCount: flat.reduce(
					(sum, item) => sum + item.track.elements.length,
					0,
				),
			},
		};
	}

	inspect_tracks(args: { sequenceId?: string } = {}): Result<{
		revision: number;
		sequenceId: string;
		tracks: TrackSummary[];
	}> {
		const scene = this.sceneOrFailure(args.sequenceId);
		if ("ok" in scene) return scene;
		return {
			ok: true,
			revision: this.editor.revision.get(),
			sequenceId: scene.id,
			tracks: summarizeTracks(scene.tracks),
		};
	}

	inspect_clip(args: {
		clipId: string;
	}): Result<{ revision: number; clip: ClipDetail }> {
		const found = locateClip(args.clipId);
		if (!found)
			return fail({
				code: "NOT_FOUND",
				message: `clip not found: ${args.clipId}`,
			});
		const tracks = this.editor.scenes.getActiveScene().tracks;
		const zone =
			flattenTracks(tracks).find((item) => item.track.id === found.trackId)
				?.zone ?? "overlay";
		const element: Record<string, unknown> = Object.fromEntries(
			Object.entries(found.element).filter(([key]) => key !== "buffer"),
		);
		return {
			ok: true,
			revision: this.editor.revision.get(),
			clip: {
				...summarizeClip({ element: found.element, trackId: found.trackId }),
				zone,
				element,
			},
		};
	}

	// ---- plans -----------------------------------------------------------

	private checkShape(plan: Plan): SurfaceError | null {
		if (plan.schemaVersion !== AGENT_SURFACE_SCHEMA_VERSION) {
			return {
				code: "SCHEMA_VERSION_UNSUPPORTED",
				message: `schemaVersion ${String(plan.schemaVersion)} is not supported`,
			};
		}
		if (typeof plan.planId !== "string" || plan.planId === "") {
			return { code: "INVALID_PLAN", message: "planId is required" };
		}
		if (
			!plan.actor ||
			typeof plan.actor.id !== "string" ||
			plan.actor.id === ""
		) {
			return { code: "INVALID_PLAN", message: "actor {kind,id} is required" };
		}
		if (!Number.isInteger(plan.baseRevision)) {
			return {
				code: "INVALID_PLAN",
				message: "baseRevision (integer) is required",
			};
		}
		if (!Array.isArray(plan.operations) || plan.operations.length === 0) {
			return {
				code: "INVALID_PLAN",
				message: "operations must be a non-empty array",
			};
		}
		const seen = new Set<string>();
		for (const operation of plan.operations) {
			if (
				!operation ||
				!OPERATION_TYPES.has(operation.type) ||
				typeof operation.operationId !== "string" ||
				operation.operationId === ""
			) {
				return {
					code: "INVALID_OPERATION",
					message: "each operation needs a known type and an operationId",
				};
			}
			if (
				seen.has(operation.operationId) ||
				this.appliedOperationIds.has(operation.operationId)
			) {
				return {
					code: "DUPLICATE_OPERATION_ID",
					message: `operationId already used: ${operation.operationId}`,
					operationId: operation.operationId,
				};
			}
			seen.add(operation.operationId);
		}
		const active = this.editor.scenes.getActiveSceneOrNull();
		if (
			plan.sequenceId !== undefined &&
			active &&
			plan.sequenceId !== active.id
		) {
			return {
				code: "SEQUENCE_NOT_ACTIVE",
				message: "v1 operates only on the active sequence",
			};
		}
		return null;
	}

	private precheck(plan: Plan): Failure | null {
		const previous = this.receipts.get(plan.planId);
		if (previous) return null; // idempotent replay is decided by the caller
		const shape = this.checkShape(plan);
		if (shape) return { ok: false, error: shape };
		const revision = this.editor.revision.get();
		if (plan.baseRevision !== revision) {
			return fail({
				code: "STALE_REVISION",
				message: `baseRevision ${plan.baseRevision} != current ${revision}`,
				details: { expected: revision, got: plan.baseRevision },
			});
		}
		return null;
	}

	private buildBatch(plan: Plan): BatchCommand {
		const refs: PlanRefs = new Map();
		return new BatchCommand(
			plan.operations.map((operation) =>
				withIdSeed({
					seed: seedFor({ plan, operation }),
					phase: "c",
					fn: () => new PlannedOperation({ operation, refs }),
				}),
			),
		);
	}

	private toSurfaceError(error: unknown): SurfaceError {
		if (error instanceof CommandError) {
			const operationId =
				typeof error.details.operationId === "string"
					? error.details.operationId
					: undefined;
			return {
				code: error.code,
				message: error.message,
				operationId,
				details: error.details,
			};
		}
		return {
			code: "COMMAND_FAILED",
			message: error instanceof Error ? error.message : String(error),
		};
	}

	/** Runs the plan against the real domain, then rolls it back (nothing persists, no history). */
	private dryRun(plan: Plan): { ok: true; diff: Diff } | Failure {
		const blocked = this.precheck(plan);
		if (blocked) return blocked;
		const { save, revision } = this.editor;
		const revisionBefore = revision.get();
		const before = this.editor.scenes.getActiveScene().tracks;
		save.pause();
		try {
			const batch = this.buildBatch(plan);
			try {
				runExecute(batch);
			} catch (error) {
				return {
					ok: false,
					error: this.toSurfaceError(error),
					rolledBack: true,
				};
			}
			const after = this.editor.scenes.getActiveScene().tracks;
			const diff = diffTracks({ before, after });
			batch.undo();
			return { ok: true, diff };
		} finally {
			revision.restore(revisionBefore);
			save.resume();
		}
	}

	validate_plan(plan: Plan): ValidationReport {
		if (this.receipts.has(plan.planId)) {
			return { valid: true, errors: [], revision: this.editor.revision.get() };
		}
		const outcome = this.dryRun(plan);
		return outcome.ok
			? { valid: true, errors: [], revision: this.editor.revision.get() }
			: {
					valid: false,
					errors: [outcome.error],
					revision: this.editor.revision.get(),
				};
	}

	preview_plan(plan: Plan): PreviewReport {
		const outcome = this.dryRun(plan);
		return outcome.ok
			? {
					valid: true,
					errors: [],
					revision: this.editor.revision.get(),
					diff: outcome.diff,
				}
			: {
					valid: false,
					errors: [outcome.error],
					revision: this.editor.revision.get(),
					diff: null,
				};
	}

	apply_plan(plan: Plan): Result<ApplyReport> {
		const replay = this.receipts.get(plan.planId);
		if (replay) {
			return { ok: true, status: "idempotent_replay", receipt: replay };
		}
		const blocked = this.precheck(plan);
		if (blocked) return blocked;
		const revisionBefore = this.editor.revision.get();
		const before = this.editor.scenes.getActiveScene().tracks;
		const batch = this.buildBatch(plan);
		try {
			this.editor.command.execute({
				command: batch,
				strict: true,
				meta: {
					actor: `${plan.actor.kind}:${plan.actor.id}`,
					planId: plan.planId,
					operationIds: plan.operations.map(
						(operation) => operation.operationId,
					),
				},
			});
		} catch (error) {
			// state is semantically unchanged after rollback: keep the CAS token stable
			this.editor.revision.restore(revisionBefore);
			return { ok: false, error: this.toSurfaceError(error), rolledBack: true };
		}
		const receipt: Receipt = {
			schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
			planId: plan.planId,
			actor: plan.actor,
			appliedOperationIds: plan.operations.map(
				(operation) => operation.operationId,
			),
			revisionBefore,
			revisionAfter: this.editor.revision.get(),
			diff: diffTracks({
				before,
				after: this.editor.scenes.getActiveScene().tracks,
			}),
			undoReceipt: {
				historyIndex: this.editor.command.getHistory().length - 1,
			},
		};
		this.receipts.set(plan.planId, receipt);
		for (const id of receipt.appliedOperationIds)
			this.appliedOperationIds.add(id);
		return { ok: true, status: "applied", receipt };
	}

	// ---- convenience single-operation plans -------------------------------

	private single({
		operation,
		actor,
	}: {
		operation: Operation;
		actor: Plan["actor"];
	}): Result<ApplyReport> {
		return this.apply_plan({
			schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
			planId: `single:${operation.operationId}`,
			actor,
			baseRevision: this.editor.revision.get(),
			operations: [operation],
		});
	}

	insert_clip(
		args: Omit<Extract<Operation, { type: "insert_clip" }>, "type"> & {
			actor: Plan["actor"];
		},
	) {
		const { actor, ...rest } = args;
		return this.single({ operation: { type: "insert_clip", ...rest }, actor });
	}

	move_clip(
		args: Omit<Extract<Operation, { type: "move_clip" }>, "type"> & {
			actor: Plan["actor"];
		},
	) {
		const { actor, ...rest } = args;
		return this.single({ operation: { type: "move_clip", ...rest }, actor });
	}

	trim_clip(
		args: Omit<Extract<Operation, { type: "trim_clip" }>, "type"> & {
			actor: Plan["actor"];
		},
	) {
		const { actor, ...rest } = args;
		return this.single({ operation: { type: "trim_clip", ...rest }, actor });
	}

	split_clip(
		args: Omit<Extract<Operation, { type: "split_clip" }>, "type"> & {
			actor: Plan["actor"];
		},
	) {
		const { actor, ...rest } = args;
		return this.single({ operation: { type: "split_clip", ...rest }, actor });
	}

	delete_clip(
		args: Omit<Extract<Operation, { type: "delete_clip" }>, "type"> & {
			actor: Plan["actor"];
		},
	) {
		const { actor, ...rest } = args;
		return this.single({ operation: { type: "delete_clip", ...rest }, actor });
	}

	// ---- history ---------------------------------------------------------

	undo(): Result<{
		revision: number;
		undone: HistoryInfo["entries"][number] | null;
		history: HistoryInfo;
	}> {
		const top = this.editor.command.peekUndo();
		if (!top)
			return fail({ code: "NOTHING_TO_UNDO", message: "history is empty" });
		this.editor.command.undo();
		return {
			ok: true,
			revision: this.editor.revision.get(),
			undone: {
				index: top.index,
				command: top.command,
				actor: top.meta?.actor,
				planId: top.meta?.planId,
				operationIds: top.meta?.operationIds,
			},
			history: this.history(),
		};
	}

	redo(): Result<{ revision: number; history: HistoryInfo }> {
		if (!this.editor.command.canRedo())
			return fail({ code: "NOTHING_TO_REDO", message: "redo stack is empty" });
		this.editor.command.redo();
		return {
			ok: true,
			revision: this.editor.revision.get(),
			history: this.history(),
		};
	}
}
