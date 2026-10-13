/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- MediaTime is a branded number: tests pass raw ticks to the UI path */
import { describe, expect, test } from "bun:test";
import { BatchCommand } from "@/commands/batch-command";
import { Command, CommandError } from "@/commands/base-command";
import { insertedClipId, splitRightClipId } from "@/agent-surface";
import { AGENT, T, allClips, createFixture, makePlan } from "./fixture";

function withInsertedClip() {
	const { editor, surface } = createFixture();
	const applied = surface.apply_plan(
		makePlan({
			revision: editor.revision.get(),
			planId: "plan-insert",
			operations: [
				{
					type: "insert_clip",
					operationId: "i1",
					mediaId: "asset-video",
					startTicks: 0,
				},
			],
		}),
	);
	if (!applied.ok) throw new Error(JSON.stringify(applied.error));
	return {
		editor,
		surface,
		clipId: insertedClipId({ planId: "plan-insert", operationId: "i1" }),
	};
}

describe("AgentSurface v1 contract (real domain, headless)", () => {
	test("inspect_* return versioned integer-tick DTOs", () => {
		const { surface, clipId } = withInsertedClip();
		const project = surface.inspect_project();
		expect(project.ok && project.project.schemaVersion).toBe(1);
		expect(project.ok && project.project.ticksPerSecond).toBe(T);
		const sequence = surface.inspect_sequence();
		expect(sequence.ok && sequence.sequence.durationTicks).toBe(10 * T);
		expect(allClips(surface).length).toBe(1);
		const clip = surface.inspect_clip({ clipId });
		expect(clip.ok && Number.isInteger(clip.clip.startTicks)).toBe(true);
		expect(surface.inspect_clip({ clipId: "nope" }).ok).toBe(false);
	});

	test("inserted clip id is deterministic and predictable across editors", () => {
		const a = withInsertedClip();
		const b = withInsertedClip();
		expect(allClips(a.surface)[0].id).toBe(a.clipId);
		expect(allClips(b.surface)[0].id).toBe(a.clipId);
	});

	test("split/move/trim/delete share one history with actor metadata", () => {
		const { editor, surface, clipId } = withInsertedClip();
		expect(
			surface.split_clip({
				operationId: "s1",
				clipId,
				atTicks: 4 * T,
				actor: AGENT,
			}).ok,
		).toBe(true);
		const rightId = splitRightClipId({
			planId: "single:s1",
			operationId: "s1",
		});
		expect(surface.inspect_clip({ clipId: rightId }).ok).toBe(true);
		expect(
			surface.move_clip({
				operationId: "m1",
				clipId: rightId,
				newStartTicks: 8 * T,
				actor: AGENT,
			}).ok,
		).toBe(true);
		expect(
			surface.trim_clip({
				operationId: "t1",
				clipId,
				trimEndTicks: 2 * T,
				durationTicks: 2 * T,
				actor: AGENT,
			}).ok,
		).toBe(true);
		expect(
			surface.delete_clip({ operationId: "d1", clipId: rightId, actor: AGENT })
				.ok,
		).toBe(true);
		const history = editor.command.getHistory();
		expect(history.map((entry) => entry.meta?.actor)).toEqual(
			Array(5).fill("agent:test"),
		);
		expect(history.at(-1)?.meta?.operationIds).toEqual(["d1"]);
	});

	test("stale baseRevision is rejected without touching the document", () => {
		const { editor, surface, clipId } = withInsertedClip();
		const depth = editor.command.getHistory().length;
		const result = surface.apply_plan(
			makePlan({
				revision: editor.revision.get() - 1,
				planId: "stale",
				operations: [{ type: "delete_clip", operationId: "x", clipId }],
			}),
		);
		expect(!result.ok && result.error.code).toBe("STALE_REVISION");
		expect(editor.command.getHistory().length).toBe(depth);
	});

	test("same planId replays the receipt; a reused operationId is rejected", () => {
		const { editor, surface, clipId } = withInsertedClip();
		const plan = makePlan({
			revision: editor.revision.get(),
			planId: "idem",
			operations: [
				{ type: "split_clip", operationId: "sp", clipId, atTicks: 3 * T },
			],
		});
		const first = surface.apply_plan(plan);
		const depth = editor.command.getHistory().length;
		const second = surface.apply_plan(plan);
		expect(first.ok && first.status).toBe("applied");
		expect(second.ok && second.status).toBe("idempotent_replay");
		expect(editor.command.getHistory().length).toBe(depth);
		const reused = surface.apply_plan(
			makePlan({
				revision: editor.revision.get(),
				planId: "other-plan",
				operations: [{ type: "delete_clip", operationId: "sp", clipId }],
			}),
		);
		expect(!reused.ok && reused.error.code).toBe("DUPLICATE_OPERATION_ID");
	});

	test("atomic: an invalid later operation rolls back earlier ones", () => {
		const { editor, surface, clipId } = withInsertedClip();
		const revision = editor.revision.get();
		const depth = editor.command.getHistory().length;
		const result = surface.apply_plan(
			makePlan({
				revision,
				planId: "atomic",
				operations: [
					{ type: "split_clip", operationId: "ok", clipId, atTicks: 2 * T },
					{ type: "delete_clip", operationId: "bad", clipId: "does-not-exist" },
				],
			}),
		);
		expect(!result.ok && result.error.code).toBe("NOT_FOUND");
		expect(!result.ok && result.error.operationId).toBe("bad");
		expect(editor.command.getHistory().length).toBe(depth);
		expect(editor.revision.get()).toBe(revision);
		expect(allClips(surface).length).toBe(1);
	});

	test("a no-op fails with NO_EFFECT and never enters the history (agent and UI paths)", () => {
		const { editor, surface, clipId } = withInsertedClip();
		const depth = editor.command.getHistory().length;
		const result = surface.split_clip({
			operationId: "outside",
			clipId,
			atTicks: 99 * T,
			actor: AGENT,
		});
		expect(!result.ok && result.error.code).toBe("NO_EFFECT");
		expect(editor.command.getHistory().length).toBe(depth);
		const tracks = surface.inspect_tracks();
		if (!tracks.ok) throw new Error("inspect failed");
		editor.timeline.splitElements({
			elements: [{ trackId: tracks.tracks[0].id, elementId: clipId }],
			splitTime: (99 * T) as never,
		});
		expect(editor.command.getHistory().length).toBe(depth);
	});

	test("preview/validate leave document, history and revision untouched", () => {
		const { editor, surface, clipId } = withInsertedClip();
		const revision = editor.revision.get();
		const depth = editor.command.getHistory().length;
		const preview = surface.preview_plan(
			makePlan({
				revision,
				planId: "prev",
				operations: [
					{ type: "split_clip", operationId: "p", clipId, atTicks: 5 * T },
				],
			}),
		);
		expect(preview.valid).toBe(true);
		expect(preview.diff?.added.length).toBe(1);
		expect(editor.command.getHistory().length).toBe(depth);
		expect(editor.revision.get()).toBe(revision);
		expect(allClips(surface).length).toBe(1);
		const invalid = surface.validate_plan(
			makePlan({
				revision,
				planId: "val",
				operations: [{ type: "delete_clip", operationId: "v", clipId: "zzz" }],
			}),
		);
		expect(invalid.valid).toBe(false);
	});

	test("one plan = one undo entry (human undo path); ids survive undo/redo", () => {
		const { editor, surface, clipId } = withInsertedClip();
		const depth = editor.command.getHistory().length;
		const applied = surface.apply_plan(
			makePlan({
				revision: editor.revision.get(),
				planId: "multi",
				operations: [
					{ type: "split_clip", operationId: "a", clipId, atTicks: 3 * T },
					{ type: "split_clip", operationId: "b", clipId, atTicks: 1 * T },
				],
			}),
		);
		expect(applied.ok).toBe(true);
		expect(editor.command.getHistory().length).toBe(depth + 1);
		const idsAfter = allClips(surface)
			.map((clip) => clip.id)
			.sort();
		editor.command.undo(); // the UI's Ctrl+Z path
		expect(allClips(surface).length).toBe(1);
		editor.command.redo();
		expect(
			allClips(surface)
				.map((clip) => clip.id)
				.sort(),
		).toEqual(idsAfter);
		expect(idsAfter).toContain(
			splitRightClipId({ planId: "multi", operationId: "a" }),
		);
	});

	test("refs: a plan can insert, split and move clips it created itself", () => {
		const { editor, surface } = createFixture();
		const plan = makePlan({
			revision: editor.revision.get(),
			planId: "refs",
			operations: [
				{
					type: "insert_clip",
					operationId: "a",
					mediaId: "asset-video",
					startTicks: 0,
					ref: "clip",
				},
				{
					type: "split_clip",
					operationId: "b",
					clipRef: "clip",
					atTicks: 4 * T,
					rightRef: "right",
				},
				{
					type: "move_clip",
					operationId: "c",
					clipRef: "right",
					newStartTicks: 8 * T,
				},
			],
		});
		const applied = surface.apply_plan(plan);
		expect(applied.ok).toBe(true);
		const starts = allClips(surface)
			.map((clip) => clip.startTicks)
			.sort((a, b) => a - b);
		expect(starts).toEqual([0, 8 * T]);
		expect(editor.command.getHistory().length).toBe(1);
		const unknown = surface.preview_plan(
			makePlan({
				revision: editor.revision.get(),
				planId: "refs-bad",
				operations: [
					{ type: "delete_clip", operationId: "z", clipRef: "nope" },
				],
			}),
		);
		expect(unknown.valid).toBe(false);
		expect(unknown.errors[0].code).toBe("NOT_FOUND");
	});

	test("BatchCommand is atomic at domain level", () => {
		const { editor } = createFixture();
		let undone = 0;
		class Ok extends Command {
			execute() {
				return undefined;
			}
			undo() {
				undone += 1;
			}
		}
		class Boom extends Command {
			execute(): never {
				throw new CommandError({ code: "BOOM", message: "fail" });
			}
		}
		const batch = new BatchCommand([new Ok(), new Ok(), new Boom()]);
		expect(() => editor.command.execute({ command: batch })).toThrow();
		expect(undone).toBe(2);
		expect(editor.command.getHistory().length).toBe(0);
	});
});
