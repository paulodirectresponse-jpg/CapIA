import type { AgentSurface } from "../surface";
import type { Plan } from "../types";
import { AGENT_SURFACE_NAME, AGENT_SURFACE_SCHEMA_VERSION } from "../types";

/**
 * Transport-neutral RPC table over AgentSurface. The same table serves the headless stdio
 * host (Bun/Node sidecar), the in-page bridge (`window.__capiaAgent`, driven by the Tauri
 * host or Playwright) and, later, REST/MCP. Params/results are plain JSON.
 */
export type RpcParams = Record<string, unknown>;

export class RpcError extends Error {
	readonly code: string;

	constructor({ code, message }: { code: string; message: string }) {
		super(message);
		this.code = code;
	}
}

const asPlan = (params: RpcParams): Plan => {
	const plan = params.plan;
	if (typeof plan !== "object" || plan === null) {
		throw new RpcError({
			code: "INVALID_PARAMS",
			message: "params.plan (object) is required",
		});
	}
	// Shape and schemaVersion are validated by AgentSurface itself (structured errors).
	return Object.assign(
		{
			schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
			planId: "",
			actor: { kind: "agent", id: "" },
			baseRevision: -1,
			operations: [],
		} satisfies Plan,
		plan,
	);
};

export function createDispatcher({ surface }: { surface: AgentSurface }) {
	return function call({
		method,
		params = {},
	}: {
		method: string;
		params?: RpcParams;
	}): unknown {
		switch (method) {
			case "surface.info":
				return {
					name: AGENT_SURFACE_NAME,
					schemaVersion: AGENT_SURFACE_SCHEMA_VERSION,
					methods: [
						"inspect_project",
						"inspect_sequence",
						"inspect_tracks",
						"inspect_clip",
						"validate_plan",
						"preview_plan",
						"apply_plan",
						"undo",
						"redo",
					],
				};
			case "inspect_project":
				return surface.inspect_project();
			case "inspect_sequence":
				return surface.inspect_sequence({
					sequenceId:
						typeof params.sequenceId === "string"
							? params.sequenceId
							: undefined,
				});
			case "inspect_tracks":
				return surface.inspect_tracks({
					sequenceId:
						typeof params.sequenceId === "string"
							? params.sequenceId
							: undefined,
				});
			case "inspect_clip":
				return surface.inspect_clip({ clipId: String(params.clipId) });
			case "validate_plan":
				return surface.validate_plan(asPlan(params));
			case "preview_plan":
				return surface.preview_plan(asPlan(params));
			case "apply_plan":
				return surface.apply_plan(asPlan(params));
			case "undo":
				return surface.undo();
			case "redo":
				return surface.redo();
			default:
				throw new RpcError({
					code: "METHOD_NOT_FOUND",
					message: `unknown method: ${method}`,
				});
		}
	};
}
