import {
	Command,
	type CommandResult,
	runExecute,
	runRedo,
} from "./base-command";

/**
 * Atomic batch: if any child throws, children already applied are undone in
 * reverse order and the error is rethrown (all-or-nothing).
 */
export class BatchCommand extends Command {
	private applied: Command[] = [];
	private failed = false;

	constructor(private commands: Command[]) {
		super();
	}

	private run(
		step: (command: Command) => CommandResult | undefined,
	): CommandResult | undefined {
		let latestSelectionResult: CommandResult | undefined;
		this.applied = [];
		this.failed = false;

		for (const command of this.commands) {
			try {
				const result = step(command);
				this.applied.push(command);
				if (result?.selection !== undefined) {
					latestSelectionResult = result;
				}
			} catch (error) {
				for (const done of [...this.applied].reverse()) {
					done.undo();
				}
				this.applied = [];
				this.failed = true;
				throw error;
			}
		}

		return latestSelectionResult;
	}

	execute(): CommandResult | undefined {
		return this.run(runExecute);
	}

	undo(): void {
		// After a rolled-back failure nothing is applied: undo is a no-op.
		const toUndo = this.failed
			? []
			: this.applied.length > 0
				? this.applied
				: this.commands;
		for (const command of [...toUndo].reverse()) {
			command.undo();
		}
	}

	redo(): CommandResult | undefined {
		return this.run(runRedo);
	}
}
