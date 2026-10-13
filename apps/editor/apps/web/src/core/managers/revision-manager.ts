import type { EditorCore } from "@/core";

/**
 * Monotonic document revision (CAS token for agent plans).
 * Bumped by every scene/timeline notification, i.e. by UI edits, agent edits,
 * undo and redo alike. Not persisted: it is an in-session optimistic-lock token.
 */
export class RevisionManager {
	private value = 0;

	constructor(editor: EditorCore) {
		editor.scenes.subscribe(() => this.bump());
		editor.timeline.subscribe(() => this.bump());
	}

	get(): number {
		return this.value;
	}

	bump(): void {
		this.value += 1;
	}

	/** Used by transient preview/validate runs that are rolled back. */
	restore(value: number): void {
		this.value = value;
	}
}
