/* eslint-disable @typescript-eslint/no-unsafe-type-assertion -- testes leem elementos do domínio sem tipar todos os params */
import { describe, expect, test } from "bun:test";
import { EditorCore } from "@/core";
import { insertedClipId } from "@/agent-surface";
import { T, createFixture, makePlan } from "./fixture";
import type { Operation } from "@/agent-surface";

function apply({
	editor,
	surface,
	planId,
	operations,
}: {
	editor: ReturnType<typeof createFixture>["editor"];
	surface: ReturnType<typeof createFixture>["surface"];
	planId: string;
	operations: Operation[];
}) {
	return surface.apply_plan(
		makePlan({ revision: editor.revision.get(), planId, operations }),
	);
}

function element(id: string) {
	const tracks = EditorCore.getInstance().scenes.getActiveScene().tracks;
	for (const track of [...tracks.overlay, tracks.main, ...tracks.audio]) {
		const found = (
			track.elements as unknown as Array<Record<string, unknown>>
		).find((e) => e.id === id);
		if (found)
			return found as {
				id: string;
				type: string;
				hidden?: boolean;
				duration: number;
				startTime: number;
				params: Record<string, unknown>;
			};
	}
	return undefined;
}

describe("AgentSurface v1.1 — texto, parâmetros, habilitar", () => {
	test("insert_text cria texto com conteúdo/duração e id determinístico", () => {
		const { editor, surface } = createFixture();
		const r = apply({
			editor,
			surface,
			planId: "p-text",
			operations: [
				{
					type: "insert_text",
					operationId: "t1",
					text: "COMPRE AGORA",
					startTicks: 2 * T,
					durationTicks: 3 * T,
				},
			],
		});
		expect(r.ok).toBe(true);
		const id = insertedClipId({ planId: "p-text", operationId: "t1" });
		const el = element(id);
		expect(el?.type).toBe("text");
		expect(el?.params.content).toBe("COMPRE AGORA");
		expect(el?.duration).toBe(3 * T);
		expect(el?.startTime).toBe(2 * T);
	});

	test("insert_text vazio é rejeitado sem alterar o documento", () => {
		const { editor, surface } = createFixture();
		const before = editor.revision.get();
		const r = apply({
			editor,
			surface,
			planId: "p-bad",
			operations: [
				{
					type: "insert_text",
					operationId: "t1",
					text: "",
					startTicks: 0,
					durationTicks: T,
				},
			],
		});
		expect(r.ok).toBe(false);
		expect(editor.revision.get()).toBe(before);
	});

	test("set_clip_params altera opacidade/posição por ref; param desconhecido é INVALID_OPERATION", () => {
		const { editor, surface } = createFixture();
		const ok = apply({
			editor,
			surface,
			planId: "p-params",
			operations: [
				{
					type: "insert_clip",
					operationId: "i1",
					mediaId: "asset-video",
					startTicks: 0,
					ref: "v",
				},
				{
					type: "set_clip_params",
					operationId: "s1",
					clipRef: "v",
					params: { opacity: 0.4, "transform.positionX": 120 },
				},
			],
		});
		expect(ok.ok).toBe(true);
		const el = element(
			insertedClipId({ planId: "p-params", operationId: "i1" }),
		);
		expect(el?.params.opacity).toBe(0.4);
		expect(el?.params["transform.positionX"]).toBe(120);

		const bad = apply({
			editor,
			surface,
			planId: "p-params-bad",
			operations: [
				{
					type: "set_clip_params",
					operationId: "b1",
					clipId: insertedClipId({ planId: "p-params", operationId: "i1" }),
					params: { nonexistent: 1 },
				},
			],
		});
		expect(bad.ok).toBe(false);
		expect(!bad.ok && bad.error.code).toBe("INVALID_OPERATION");
	});

	test("set_clip_enabled alterna hidden e um plano inteiro desfaz com um undo", () => {
		const { editor, surface } = createFixture();
		apply({
			editor,
			surface,
			planId: "p-en",
			operations: [
				{
					type: "insert_clip",
					operationId: "i1",
					mediaId: "asset-video",
					startTicks: 0,
				},
			],
		});
		const id = insertedClipId({ planId: "p-en", operationId: "i1" });
		const off = apply({
			editor,
			surface,
			planId: "p-en2",
			operations: [
				{
					type: "set_clip_enabled",
					operationId: "e1",
					clipId: id,
					enabled: false,
				},
				{
					type: "set_clip_params",
					operationId: "e2",
					clipId: id,
					params: { opacity: 0.1 },
				},
			],
		});
		expect(off.ok).toBe(true);
		expect(element(id)?.hidden).toBe(true);
		editor.command.undo();
		expect(element(id)?.hidden).not.toBe(true);
		expect(element(id)?.params.opacity).toBe(1);
	});

	test("mesmo efeito já aplicado vira NO_EFFECT (nunca entra no histórico)", () => {
		const { editor, surface } = createFixture();
		apply({
			editor,
			surface,
			planId: "p-ne",
			operations: [
				{
					type: "insert_clip",
					operationId: "i1",
					mediaId: "asset-video",
					startTicks: 0,
				},
			],
		});
		const id = insertedClipId({ planId: "p-ne", operationId: "i1" });
		const r = apply({
			editor,
			surface,
			planId: "p-ne2",
			operations: [
				{
					type: "set_clip_enabled",
					operationId: "e1",
					clipId: id,
					enabled: true,
				},
			],
		});
		expect(r.ok).toBe(false);
		expect(!r.ok && r.error.code).toBe("NO_EFFECT");
	});
});
