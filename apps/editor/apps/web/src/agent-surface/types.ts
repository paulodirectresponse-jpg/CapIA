/**
 * AgentSurface v1 contract (CapIA). The ONLY structured write door of the editor
 * besides the UI: every operation runs the same Command classes the UI uses.
 * Time is always integer MediaTime ticks (never float seconds).
 */
export const AGENT_SURFACE_SCHEMA_VERSION = 1 as const;
export const AGENT_SURFACE_NAME = "capia.agent-surface" as const;

export type ActorKind = "human" | "agent" | "api" | "cli";

export interface Actor {
	kind: ActorKind;
	id: string;
}

export type Operation =
	| {
			type: "insert_clip";
			operationId: string;
			mediaId: string;
			startTicks: number;
			trackId?: string;
			/** Names the created clip so later operations of the same plan can target it. */
			ref?: string;
	  }
	| {
			type: "move_clip";
			operationId: string;
			clipId?: string;
			/** Alternative to clipId: a ref created earlier in the same plan. */
			clipRef?: string;
			newStartTicks: number;
			targetTrackId?: string;
	  }
	| {
			type: "trim_clip";
			operationId: string;
			clipId?: string;
			/** Alternative to clipId: a ref created earlier in the same plan. */
			clipRef?: string;
			trimStartTicks?: number;
			trimEndTicks?: number;
			startTicks?: number;
			durationTicks?: number;
	  }
	| {
			type: "split_clip";
			operationId: string;
			clipId?: string;
			/** Alternative to clipId: a ref created earlier in the same plan. */
			clipRef?: string;
			atTicks: number;
			retainSide?: "both" | "left" | "right";
			/** Names the right-hand clip created by the split. */
			rightRef?: string;
	  }
	| {
			type: "delete_clip";
			operationId: string;
			clipId?: string;
			clipRef?: string;
	  };

export type OperationType = Operation["type"];

export interface Plan {
	schemaVersion: typeof AGENT_SURFACE_SCHEMA_VERSION;
	planId: string;
	actor: Actor;
	baseRevision: number;
	/** Optional: v1 operates on the active sequence; a different id is rejected. */
	sequenceId?: string;
	operations: Operation[];
}

export type ErrorCode =
	| "SCHEMA_VERSION_UNSUPPORTED"
	| "INVALID_PLAN"
	| "INVALID_OPERATION"
	| "DUPLICATE_OPERATION_ID"
	| "STALE_REVISION"
	| "SEQUENCE_NOT_ACTIVE"
	| "NOT_FOUND"
	| "NO_EFFECT"
	| "COMMAND_FAILED"
	| "NOTHING_TO_UNDO"
	| "NOTHING_TO_REDO";

export interface SurfaceError {
	code: ErrorCode | string;
	message: string;
	operationId?: string;
	details?: Record<string, unknown>;
}

export interface ClipSummary {
	id: string;
	type: string;
	name: string;
	trackId: string;
	startTicks: number;
	durationTicks: number;
	endTicks: number;
	trimStartTicks: number;
	trimEndTicks: number;
	mediaId?: string;
	retimeRate?: number;
}

export interface TrackSummary {
	id: string;
	zone: "overlay" | "main" | "audio";
	index: number;
	type: string;
	name: string;
	muted?: boolean;
	hidden?: boolean;
	clips: ClipSummary[];
}

export interface Diff {
	added: ClipSummary[];
	removed: ClipSummary[];
	changed: Array<{ before: ClipSummary; after: ClipSummary }>;
}

export interface HistoryInfo {
	canUndo: boolean;
	canRedo: boolean;
	entries: Array<{
		index: number;
		command: string;
		actor?: string;
		planId?: string;
		operationIds?: string[];
	}>;
}

export interface ProjectInfo {
	schemaVersion: typeof AGENT_SURFACE_SCHEMA_VERSION;
	projectId: string;
	name: string;
	revision: number;
	ticksPerSecond: number;
	fps: { numerator: number; denominator: number };
	canvas: { width: number; height: number };
	activeSequenceId: string;
	sequences: Array<{ id: string; name: string; isMain: boolean }>;
	assets: Array<{
		id: string;
		name: string;
		type: string;
		durationSeconds?: number;
	}>;
	history: HistoryInfo;
}

export interface SequenceInfo {
	schemaVersion: typeof AGENT_SURFACE_SCHEMA_VERSION;
	sequenceId: string;
	name: string;
	isMain: boolean;
	revision: number;
	durationTicks: number;
	trackCount: number;
	clipCount: number;
}

export interface ClipDetail extends ClipSummary {
	zone: TrackSummary["zone"];
	element: Record<string, unknown>;
}

export interface Receipt {
	schemaVersion: typeof AGENT_SURFACE_SCHEMA_VERSION;
	planId: string;
	actor: Actor;
	appliedOperationIds: string[];
	revisionBefore: number;
	revisionAfter: number;
	diff: Diff;
	/** The single shared-history entry this plan produced (undo with `undo()`). */
	undoReceipt: { historyIndex: number };
}

export type Result<T> =
	| ({ ok: true } & T)
	| { ok: false; error: SurfaceError; rolledBack?: boolean };

export interface ValidationReport {
	valid: boolean;
	errors: SurfaceError[];
	revision: number;
}

export interface PreviewReport extends ValidationReport {
	diff: Diff | null;
}

export interface ApplyReport {
	status: "applied" | "idempotent_replay";
	receipt: Receipt;
}
