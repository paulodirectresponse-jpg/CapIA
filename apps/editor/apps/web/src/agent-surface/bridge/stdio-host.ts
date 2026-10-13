/**
 * Headless AgentSurface host: JSON-lines RPC over stdin/stdout.
 *   -> {"id":1,"method":"host.open","params":{"assets":[...]}}
 *   <- {"id":1,"ok":true,"result":{...}}
 * Run with: bun run apps/web/src/agent-surface/bridge/stdio-host.ts
 */
import { createInterface } from "node:readline";
import type { AgentSurface } from "../surface";
import { createDispatcher, RpcError, type RpcParams } from "./dispatch";
import { openHeadlessProject, type FixtureAsset } from "./headless-fixture";

let surface: AgentSurface | null = null;

function isFixtureAssets(value: unknown): value is FixtureAsset[] {
	return (
		Array.isArray(value) &&
		value.every(
			(asset) =>
				typeof asset === "object" &&
				asset !== null &&
				typeof Reflect.get(asset, "id") === "string" &&
				typeof Reflect.get(asset, "name") === "string",
		)
	);
}

function handle({
	method,
	params,
}: {
	method: string;
	params: RpcParams;
}): unknown {
	if (method === "host.open") {
		const assets = params.assets;
		surface = openHeadlessProject({
			assets: isFixtureAssets(assets) ? assets : [],
		});
		return { opened: true };
	}
	if (method === "host.shutdown") {
		setTimeout(() => process.exit(0), 10);
		return { bye: true };
	}
	if (!surface) {
		throw new RpcError({ code: "NOT_OPEN", message: "call host.open first" });
	}
	return createDispatcher({ surface })({ method, params });
}

const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
	if (line.trim() === "") return;
	let id: unknown = null;
	try {
		const request: unknown = JSON.parse(line);
		if (typeof request !== "object" || request === null) {
			throw new RpcError({
				code: "INVALID_REQUEST",
				message: "request must be an object",
			});
		}
		id = Reflect.get(request, "id");
		const method = Reflect.get(request, "method");
		const rawParams: unknown = Reflect.get(request, "params");
		if (typeof method !== "string") {
			throw new RpcError({
				code: "INVALID_REQUEST",
				message: "method (string) is required",
			});
		}
		const params: RpcParams =
			typeof rawParams === "object" && rawParams !== null
				? Object.fromEntries(Object.entries(rawParams))
				: {};
		const result = handle({ method, params });
		process.stdout.write(`${JSON.stringify({ id, ok: true, result })}\n`);
	} catch (error) {
		const code = error instanceof RpcError ? error.code : "INTERNAL";
		const message = error instanceof Error ? error.message : String(error);
		process.stdout.write(
			`${JSON.stringify({ id, ok: false, error: { code, message } })}\n`,
		);
	}
});
