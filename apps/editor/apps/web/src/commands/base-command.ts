import type { EditorSelectionPatch } from "@/selection/editor-selection";
import type { ElementRef } from "@/timeline/types";
import { currentIdSeed, withIdSeed } from "@/utils/id";

export interface CommandResult {
	selection?: EditorSelectionPatch;
}

export function createElementSelectionResult(
	selectedElements: ElementRef[],
): CommandResult {
	return {
		selection: {
			selectedElements,
			selectedKeyframes: [],
			keyframeSelectionAnchor: null,
			selectedMaskPoints: null,
		},
	};
}

/** Structured, machine-readable command failure (CapIA). */
export class CommandError extends Error {
	public readonly code: string;
	public readonly details: Record<string, unknown>;

	constructor({
		code,
		message,
		details = {},
	}: {
		code: string;
		message: string;
		details?: Record<string, unknown>;
	}) {
		super(message);
		this.name = "CommandError";
		this.code = code;
		this.details = details;
	}
}

export abstract class Command {
	/** Seed captured at construction: deterministic ids across execute/redo (agent plans). */
	readonly idSeed: string | undefined = currentIdSeed();

	abstract execute(): CommandResult | undefined;

	undo(): void {
		throw new Error("Undo not implemented for this command");
	}

	redo(): CommandResult | undefined {
		return this.execute();
	}
}

/** Execute/redo through these so ids stay deterministic when the command carries a seed. */
export function runExecute(command: Command): CommandResult | undefined {
	return withIdSeed({
		seed: command.idSeed,
		phase: "x",
		fn: () => command.execute(),
	});
}

export function runRedo(command: Command): CommandResult | undefined {
	return withIdSeed({
		seed: command.idSeed,
		phase: "x",
		fn: () => command.redo(),
	});
}
