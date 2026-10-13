import type { EditorCore } from "@/core";
import { CommandError, runExecute, runRedo } from "@/commands/base-command";
import {
	fingerprintsEqual,
	takeFingerprint,
	type DocumentFingerprint,
} from "./document-fingerprint";
import type { Command, CommandResult } from "@/commands";
import type { EditorSelectionSnapshot } from "@/selection/editor-selection";
import { applyRippleAdjustments, computeRippleAdjustments } from "@/ripple";
import type { SceneTracks } from "@/timeline/types";

/** Who/what produced a history entry (CapIA: shared human + agent history). */
export interface HistoryMeta {
	actor: string;
	planId?: string;
	operationIds?: string[];
}

interface CommandHistoryEntry {
	command: Command;
	previousSelection: EditorSelectionSnapshot;
	selectionOverride?: EditorSelectionSnapshot;
	meta?: HistoryMeta;
}

export interface HistoryItem {
	index: number;
	command: string;
	meta?: HistoryMeta;
}

export class CommandManager {
	public isRippleEnabled = false;
	private history: CommandHistoryEntry[] = [];
	private redoStack: CommandHistoryEntry[] = [];
	private reactors: Array<() => void> = [];

	constructor(private editor: EditorCore) {}

	private fingerprint(): DocumentFingerprint {
		return takeFingerprint(this.editor);
	}

	private static sameFingerprint({
		a,
		b,
	}: {
		a: DocumentFingerprint;
		b: DocumentFingerprint;
	}): boolean {
		return fingerprintsEqual({ a, b });
	}

	/**
	 * Executes a command and records it in the shared history.
	 * - A command that changed nothing is NOT recorded (and throws NO_EFFECT when `strict`).
	 * - A command that throws is rolled back (best effort) and the error is rethrown.
	 */
	execute({
		command,
		meta,
		strict = false,
	}: {
		command: Command;
		meta?: HistoryMeta;
		strict?: boolean;
	}): Command {
		const beforeTracks = this.isRippleEnabled
			? (this.editor.scenes.getActiveSceneOrNull()?.tracks ?? null)
			: null;
		const previousSelection = this.getSelectionSnapshot();
		const stateBefore = this.fingerprint();
		let result: CommandResult | undefined;
		try {
			result = runExecute(command);
		} catch (error) {
			if (
				!CommandManager.sameFingerprint({
					a: stateBefore,
					b: this.fingerprint(),
				})
			) {
				try {
					command.undo();
				} catch {
					// best effort: keep the original error
				}
			}
			throw error;
		}
		const hadEffect = !CommandManager.sameFingerprint({
			a: stateBefore,
			b: this.fingerprint(),
		});
		this.applyRippleIfEnabled({ beforeTracks });
		const selectionOverride = this.applySelectionOverride(result);
		this.runReactors();
		if (!hadEffect) {
			if (strict) {
				throw new CommandError({
					code: "NO_EFFECT",
					message: "command completed without changing the document",
					details: { command: command.constructor.name },
				});
			}
			return command;
		}
		this.history.push({
			command,
			previousSelection,
			selectionOverride,
			meta,
		});
		this.redoStack = [];
		return command;
	}

	/** Read-only view of the shared history (oldest first). */
	getHistory(): HistoryItem[] {
		return this.history.map((entry, index) => ({
			index,
			command: entry.command.constructor.name,
			meta: entry.meta,
		}));
	}

	peekUndo(): HistoryItem | null {
		const entry = this.history[this.history.length - 1];
		return entry
			? {
					index: this.history.length - 1,
					command: entry.command.constructor.name,
					meta: entry.meta,
				}
			: null;
	}

	push({ command, meta }: { command: Command; meta?: HistoryMeta }): void {
		this.history.push({
			command,
			previousSelection: this.getSelectionSnapshot(),
			meta,
		});
		this.redoStack = [];
	}

	registerReactor(reactor: () => void): void {
		this.reactors.push(reactor);
	}

	undo(): void {
		if (this.history.length === 0) return;
		const entry = this.history.pop();
		entry?.command.undo();
		if (entry) {
			// Only restore selection for commands that explicitly changed it.
			// Commands without selection intent leave selection untouched,
			// preserving any UI-driven selection changes (clicks, box select)
			// that happened between commands. Commands that remove editor-owned
			// selection targets must declare a selection override to clear stale refs.
			if (entry.selectionOverride !== undefined) {
				this.editor.selection.restoreSnapshot({
					snapshot: entry.previousSelection,
				});
			}
			this.redoStack.push(entry);
		}
	}

	redo(): void {
		if (this.redoStack.length === 0) return;
		const entry = this.redoStack.pop();
		if (!entry) {
			return;
		}

		const beforeTracks = this.isRippleEnabled
			? (this.editor.scenes.getActiveSceneOrNull()?.tracks ?? null)
			: null;
		const previousSelection = this.getSelectionSnapshot();
		const result = runRedo(entry.command);
		this.applyRippleIfEnabled({ beforeTracks });
		const selectionOverride = this.applySelectionOverride(result);
		this.runReactors();

		this.history.push({
			command: entry.command,
			previousSelection,
			selectionOverride,
			meta: entry.meta,
		});
	}

	canUndo(): boolean {
		return this.history.length > 0;
	}

	canRedo(): boolean {
		return this.redoStack.length > 0;
	}

	clear(): void {
		this.history = [];
		this.redoStack = [];
	}

	private getSelectionSnapshot(): EditorSelectionSnapshot {
		return this.editor.selection.getSnapshot();
	}

	private applySelectionOverride(
		result: CommandResult | undefined,
	): EditorSelectionSnapshot | undefined {
		if (!result?.selection) {
			return undefined;
		}
		return this.editor.selection.applySelectionPatch({
			patch: result.selection,
		});
	}

	private runReactors(): void {
		for (const reactor of this.reactors) {
			reactor();
		}
	}

	private applyRippleIfEnabled({
		beforeTracks,
	}: {
		beforeTracks: SceneTracks | null;
	}): void {
		if (!this.isRippleEnabled || !beforeTracks) {
			return;
		}

		const afterTracks = this.editor.scenes.getActiveSceneOrNull()?.tracks;
		if (!afterTracks) {
			return;
		}
		const adjustments = computeRippleAdjustments({
			beforeTracks,
			afterTracks,
		});
		if (adjustments.length === 0) {
			return;
		}

		const tracksWithRipple = applyRippleAdjustments({
			tracks: afterTracks,
			adjustments,
		});
		this.editor.timeline.updateTracks(tracksWithRipple);
	}
}
