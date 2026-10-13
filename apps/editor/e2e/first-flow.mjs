// Primeiro milestone da nova arquitetura: humano (UI) + CapIA (EnginePort) na MESMA timeline e
// no MESMO histórico. Uso: EDITOR_URL=http://127.0.0.1:3100 node e2e/first-flow.mjs
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
	call,
	dragAssetToTimeline,
	importFiles,
	launch,
	makeMedia,
	openNewProject,
	portCli,
	startBridge,
} from "./harness.mjs";

const BASE = process.env.EDITOR_URL ?? "http://127.0.0.1:3100";
const OUT = process.env.E2E_OUT ?? join(process.cwd(), "e2e-out");
const L = 705_600_000; // ticks/s do lado CapIA (a ponte converte para 120 000 t/s)
mkdirSync(OUT, { recursive: true });
const steps = [];
const rec = (name, status, detail = "") => {
	steps.push({ name, status, detail: String(detail).slice(0, 300) });
	console.log(`[${status}] ${name} :: ${String(detail).slice(0, 160)}`);
};
const must = (ok, name, detail) => {
	rec(name, ok ? "PASS" : "FAIL", detail);
	if (!ok) process.exitCode = 1;
};

const { browser, page } = await launch();
const bridge = await startBridge(page);
const media = makeMedia();
const clipsUI = async () =>
	(await call(page, "inspect_tracks")).tracks.flatMap((t) => t.clips);
const shot = (n) => page.screenshot({ path: join(OUT, `${n}.png`) });
const cli = (...a) => portCli(bridge.addr, a); // Promise

try {
	await openNewProject(page, BASE);
	must(true, "UI: criar projeto", page.url());
	await importFiles(page, [media.main]);
	must(true, "UI: importar vídeo", "main.webm");
	await dragAssetToTimeline(page, /main\.webm/, 400, 620);
	let clips = await clipsUI();
	must(
		clips.length === 1 && Math.abs(clips[0].durationTicks - 12 * 120_000) < 600,
		"UI: inserir vídeo na timeline",
		JSON.stringify(clips.map((c) => [c.startTicks, c.durationTicks])),
	);
	await shot("1-ui-inserted");

	let r = await cli("inspect");
	must(
		r.ok && r.result.sequence.tracks.flatMap((t) => t.clips).length === 1,
		"CAPIA(EnginePort): inspect vê o clip da UI",
		JSON.stringify(r.result?.revision),
	);
	const clipId = r.result.sequence.tracks.flatMap((t) => t.clips)[0].id;

	r = await cli(
		"apply",
		"split",
		JSON.stringify([
			{ type: "split_clip", clip: clipId, at: 4 * L, operation_id: "ff-split" },
		]),
	);
	must(
		r.ok,
		"CAPIA: split @4s",
		JSON.stringify(r.error ?? r.result?.diff?.added?.length),
	);
	r = await cli("inspect");
	const parts = r.result.sequence.tracks
		.flatMap((t) => t.clips)
		.sort((a, b) => a.startTicks - b.startTicks);
	must(
		parts.length === 2 && parts[1].startTicks === 4 * 120_000,
		"CAPIA: inspect após split",
		JSON.stringify(parts.map((p) => [p.startTicks, p.durationTicks])),
	);

	r = await cli(
		"apply",
		"move",
		JSON.stringify([
			{
				type: "move_clips",
				moves: [{ clip: parts[1].id, start: 8 * L }],
				operation_id: "ff-move",
			},
		]),
	);
	must(r.ok, "CAPIA: move (parte direita → 8s)", JSON.stringify(r.error ?? ""));
	r = await cli(
		"apply",
		"trim",
		JSON.stringify([
			{
				type: "trim_clip",
				clip: parts[0].id,
				edge: "out",
				to: 3 * L,
				operation_id: "ff-trim",
			},
		]),
	);
	must(
		r.ok,
		"CAPIA: trim (borda out da parte esquerda → 3s)",
		JSON.stringify(r.error ?? ""),
	);

	clips = (await clipsUI()).sort((a, b) => a.startTicks - b.startTicks);
	must(
		clips.length === 2 &&
			clips[0].durationTicks === 3 * 120_000 &&
			clips[1].startTicks === 8 * 120_000,
		"UI: vê as mudanças do CAPIA (estado vivo)",
		JSON.stringify(clips.map((c) => [c.startTicks, c.durationTicks])),
	);
	const labels = await page.evaluate(
		() =>
			[...document.querySelectorAll("span,div")].filter(
				(e) =>
					e.children.length === 0 &&
					/main\.webm/.test(e.textContent ?? "") &&
					e.getBoundingClientRect().top > 550,
			).length,
	);
	must(
		labels >= 2,
		"UI: DOM da timeline mostra os 2 clips",
		`labels=${labels}`,
	);
	await shot("2-capia-edits-visible-in-ui");

	await page.mouse.click(1000, 300); // foco fora de inputs
	await page.keyboard.press("Control+z");
	await page.waitForTimeout(500);
	r = await cli("inspect");
	const afterUndo = r.result.sequence.tracks
		.flatMap((t) => t.clips)
		.sort((a, b) => a.startTicks - b.startTicks);
	must(
		afterUndo[0].durationTicks === 4 * 120_000,
		"UI: Ctrl+Z desfaz o trim do CAPIA → CAPIA inspect vê o estado desfeito",
		JSON.stringify(afterUndo.map((c) => [c.startTicks, c.durationTicks])),
	);

	const before = JSON.stringify(
		afterUndo.map((c) => [c.id, c.startTicks, c.durationTicks]),
	);
	const bad = await cli(
		"apply",
		"atomic-bad",
		JSON.stringify([
			{
				type: "split_clip",
				clip: afterUndo[0].id,
				at: 2 * L,
				operation_id: "ff-a1",
			},
			{ type: "delete_clip", clip: "does-not-exist", operation_id: "ff-a2" },
		]),
	);
	const afterBad = (await clipsUI()).sort(
		(a, b) => a.startTicks - b.startTicks,
	);
	must(
		!bad.ok &&
			JSON.stringify(
				afterBad.map((c) => [c.id, c.startTicks, c.durationTicks]),
			) === before,
		"CAPIA: plano atômico inválido não aplica nada",
		JSON.stringify(bad.error),
	);

	r = await cli(
		"apply",
		"atomic-good",
		JSON.stringify([
			{
				type: "split_clip",
				clip: afterUndo[0].id,
				at: 2 * L,
				operation_id: "ff-g1",
			},
			{
				type: "move_clips",
				moves: [{ clip: afterUndo[1].id, start: 5 * L }],
				operation_id: "ff-g2",
			},
		]),
	);
	must(
		r.ok,
		"CAPIA: plano atômico (split + move)",
		JSON.stringify(r.error ?? r.result.apply.entry_id),
	);
	clips = (await clipsUI()).sort((a, b) => a.startTicks - b.startTicks);
	must(
		clips.length === 3 && clips.at(-1).startTicks === 5 * 120_000,
		"UI: vê o resultado do plano atômico",
		JSON.stringify(clips.map((c) => [c.startTicks, c.durationTicks])),
	);
	await shot("3-final");

	const history = (await cli("history")).result;
	const actors = history.map((h) => h.actor ?? "ui");
	must(
		history.some((h) => h.actor === "agent:run:first-flow") &&
			history.length >= 4,
		"histórico único contém entradas do agente (ator/plan)",
		JSON.stringify(actors),
	);
	await page.keyboard.press("Control+z");
	await page.waitForTimeout(400);
	must(
		(await clipsUI()).length === 2,
		"UI: um Ctrl+Z desfaz o plano atômico inteiro (1 entrada)",
		`clips=${(await clipsUI()).length}`,
	);
} catch (e) {
	rec("EXCEÇÃO", "FAIL", e.stack ?? e);
	process.exitCode = 1;
} finally {
	writeFileSync(
		join(OUT, "first-flow-results.json"),
		JSON.stringify({ base: BASE, steps }, null, 1),
	);
	bridge.close();
	await browser.close();
}
console.log(process.exitCode ? "FIRST_FLOW: FAIL" : "FIRST_FLOW: PASS");
