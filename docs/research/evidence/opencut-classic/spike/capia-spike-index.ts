// VALIDATION SPIKE (throwaway clone, never shipped): programmatic editing adapter.
// Rule: NO timeline rules duplicated here. Everything goes through the same
// EditorCore managers / Command classes the UI uses.
import { EditorCore } from "@/core";
import { Command, type CommandResult } from "@/commands";
import {
	InsertElementCommand,
	SplitElementsCommand,
	MoveElementCommand,
	DeleteElementsCommand,
	UpdateElementsCommand,
} from "@/commands/timeline";
import { buildElementFromMedia } from "@/timeline/element-utils";
import type { SceneTracks, TimelineElement } from "@/timeline";
import { mediaTimeFromSeconds, TICKS_PER_SECOND } from "@/wasm";

type Op =
	| { op: "insert_clip"; operationId: string; mediaId: string; startSeconds: number }
	| { op: "split_clip"; operationId: string; elementId: string; atSeconds: number }
	| { op: "move_clip"; operationId: string; elementId: string; newStartSeconds: number; targetTrackId?: string }
	| { op: "trim_clip"; operationId: string; elementId: string; trimStartSeconds: number; trimEndSeconds: number; startSeconds?: number; durationSeconds?: number }
	| { op: "delete_clip"; operationId: string; elementId: string };

const ed = () => EditorCore.getInstance();
const sec = (s: number) => mediaTimeFromSeconds({ seconds: s });

function flatTracks(t: SceneTracks) {
	return [
		...t.overlay.map((x, i) => ({ zone: "overlay" as const, index: i, track: x })),
		{ zone: "main" as const, index: 0, track: t.main },
		...t.audio.map((x, i) => ({ zone: "audio" as const, index: i, track: x })),
	];
}
function locate(elementId: string) {
	for (const { track } of flatTracks(ed().scenes.getActiveScene().tracks)) {
		const el = (track.elements as TimelineElement[]).find((e) => e.id === elementId);
		if (el) return { trackId: track.id, element: el };
	}
	return null;
}

let revision = 0;
const appliedOps = new Set<string>();
let wired = false;

class AtomicBatch extends Command {
	private done: Command[] = [];
	constructor(private cmds: Command[]) { super(); }
	execute(): CommandResult | undefined {
		this.done = [];
		try {
			for (const c of this.cmds) { c.execute(); this.done.push(c); }
		} catch (e) {
			for (const c of [...this.done].reverse()) c.undo();
			this.done = [];
			throw e;
		}
		return undefined;
	}
	undo() { for (const c of [...this.done].reverse()) c.undo(); }
	redo() { return this.execute(); }
}

function toCommand(op: Op): Command {
	switch (op.op) {
		case "insert_clip": {
			const a = ed().media.getAssets().find((x) => x.id === op.mediaId);
			if (!a) throw new Error(`asset not found: ${op.mediaId}`);
			const el = buildElementFromMedia({
				mediaId: a.id, mediaType: a.type, name: a.name,
				duration: a.duration != null ? sec(a.duration) : sec(5),
				startTime: sec(op.startSeconds),
			});
			return new InsertElementCommand({ element: el, placement: { mode: "auto" } });
		}
		case "split_clip": {
			const loc = locate(op.elementId); if (!loc) throw new Error(`element not found: ${op.elementId}`);
			return new SplitElementsCommand({ elements: [{ trackId: loc.trackId, elementId: op.elementId }], splitTime: sec(op.atSeconds), retainSide: "both" });
		}
		case "move_clip": {
			const loc = locate(op.elementId); if (!loc) throw new Error(`element not found: ${op.elementId}`);
			return new MoveElementCommand({ moves: [{ sourceTrackId: loc.trackId, targetTrackId: op.targetTrackId ?? loc.trackId, elementId: op.elementId, newStartTime: sec(op.newStartSeconds) }] });
		}
		case "trim_clip": {
			const loc = locate(op.elementId); if (!loc) throw new Error(`element not found: ${op.elementId}`);
			const patch: Partial<TimelineElement> = { trimStart: sec(op.trimStartSeconds), trimEnd: sec(op.trimEndSeconds) } as never;
			if (op.startSeconds !== undefined) (patch as any).startTime = sec(op.startSeconds);
			if (op.durationSeconds !== undefined) (patch as any).duration = sec(op.durationSeconds);
			return new UpdateElementsCommand({ updates: [{ trackId: loc.trackId, elementId: op.elementId, patch }] } as never);
		}
		case "delete_clip": {
			const loc = locate(op.elementId); if (!loc) throw new Error(`element not found: ${op.elementId}`);
			return new DeleteElementsCommand({ elements: [{ trackId: loc.trackId, elementId: op.elementId }] });
		}
	}
}

const summarize = (tracks: SceneTracks) =>
	flatTracks(tracks).map(({ zone, index, track }) => ({
		trackId: track.id, zone, index, type: track.type, name: track.name,
		elements: (track.elements as TimelineElement[]).map((e: any) => ({
			id: e.id, type: e.type, name: e.name, startTicks: e.startTime, durationTicks: e.duration,
			startSec: e.startTime / TICKS_PER_SECOND, durSec: e.duration / TICKS_PER_SECOND,
			trimStartTicks: e.trimStart, trimEndTicks: e.trimEnd, mediaId: e.mediaId, rate: e.retime?.rate,
		})),
	}));

function diffTracks(a: SceneTracks, b: SceneTracks) {
	const fa = new Map<string, any>(), fb = new Map<string, any>();
	for (const t of summarize(a)) for (const e of t.elements) fa.set(e.id, { ...e, trackId: t.trackId });
	for (const t of summarize(b)) for (const e of t.elements) fb.set(e.id, { ...e, trackId: t.trackId });
	const added = [...fb.keys()].filter((k) => !fa.has(k));
	const removed = [...fa.keys()].filter((k) => !fb.has(k));
	const changed = [...fb.keys()].filter((k) => fa.has(k) && JSON.stringify(fa.get(k)) !== JSON.stringify(fb.get(k)));
	return { added: added.map((k) => fb.get(k)), removed: removed.map((k) => fa.get(k)), changed: changed.map((k) => ({ before: fa.get(k), after: fb.get(k) })) };
}

export function installCapiaSpike() {
	const editor = ed();
	if (!wired) {
		wired = true;
		editor.scenes.subscribe(() => { revision++; });
	}
	const api = {
		inspect_project() {
			const p = editor.project.getActive();
			return { id: p.metadata.id, name: p.metadata.name, scenes: editor.scenes.getScenes().map((s) => ({ id: s.id, name: s.name, isMain: s.isMain })), activeSceneId: editor.scenes.getActiveScene().id, revision, canUndo: editor.command.canUndo(), canRedo: editor.command.canRedo(), assets: editor.media.getAssets().map((a) => ({ id: a.id, name: a.name, type: a.type, duration: a.duration })) };
		},
		inspect_scene() { const s = editor.scenes.getActiveScene(); return { id: s.id, name: s.name, revision, totalDurationSec: editor.timeline.getTotalDuration() / TICKS_PER_SECOND, trackCount: flatTracks(s.tracks).length }; },
		inspect_tracks() { return summarize(editor.scenes.getActiveScene().tracks); },
		insert_clip(a: { mediaId: string; startSeconds: number }) { return api.apply_plan({ baseRevision: revision, ops: [{ op: "insert_clip", operationId: crypto.randomUUID(), ...a }] }); },
		split_clip(a: { elementId: string; atSeconds: number }) { return api.apply_plan({ baseRevision: revision, ops: [{ op: "split_clip", operationId: crypto.randomUUID(), ...a }] }); },
		move_clip(a: { elementId: string; newStartSeconds: number; targetTrackId?: string }) { return api.apply_plan({ baseRevision: revision, ops: [{ op: "move_clip", operationId: crypto.randomUUID(), ...a }] }); },
		trim_clip(a: any) { return api.apply_plan({ baseRevision: revision, ops: [{ op: "trim_clip", operationId: crypto.randomUUID(), ...a }] }); },
		delete_clip(a: { elementId: string }) { return api.apply_plan({ baseRevision: revision, ops: [{ op: "delete_clip", operationId: crypto.randomUUID(), ...a }] }); },
		undo() { editor.command.undo(); return { canUndo: editor.command.canUndo(), revision }; },
		redo() { editor.command.redo(); return { canRedo: editor.command.canRedo(), revision }; },
		// preview = execute outside history + diff + rollback (commands mutate the store immediately)
		preview_plan(plan: { baseRevision: number; ops: Op[] }) { return run(plan, false); },
		apply_plan(plan: { baseRevision: number; ops: Op[] }) { return run(plan, true); },
	};
	function run(plan: { baseRevision: number; ops: Op[] }, commit: boolean) {
		if (plan.baseRevision !== revision) return { ok: false, error: { code: "STALE_REVISION", expected: revision, got: plan.baseRevision } };
		const fresh = plan.ops.filter((o) => !appliedOps.has(o.operationId));
		if (fresh.length === 0) return { ok: true, idempotent: true, applied: 0, revision };
		let batch: AtomicBatch;
		try { batch = new AtomicBatch(fresh.map(toCommand)); } catch (e: any) { return { ok: false, error: { code: "VALIDATION", message: String(e.message) } }; }
		const before = editor.scenes.getActiveScene().tracks;
		if (!commit) {
			const rev0 = revision;
			try { batch.execute(); } catch (e: any) { revision = rev0; return { ok: false, error: { code: "EXEC", message: String(e.message) } }; }
			const after = editor.scenes.getActiveScene().tracks; const diff = diffTracks(before, after);
			batch.undo(); revision = rev0;
			return { ok: true, preview: true, diff, revision };
		}
		try { editor.command.execute({ command: batch }); } catch (e: any) { return { ok: false, error: { code: "EXEC", message: String(e.message) }, rolledBack: true } }
		fresh.forEach((o) => appliedOps.add(o.operationId));
		const after = editor.scenes.getActiveScene().tracks;
		return { ok: true, applied: fresh.length, diff: diffTracks(before, after), revision, undoReceipt: { historyDepthHint: "single undo entry" } };
	}
	(window as any).__capia = { editor, api };
	return api;
}
