import { EditorCore } from "@/core";
import { buildDefaultMaskInstance } from "@/masks";
import { buildTextElement } from "@/timeline/element-utils";
import { AgentSurface } from "../surface";
import { createDispatcher, type RpcParams } from "./dispatch";

declare global {
	interface Window {
		/** E2E/benchmark only (NEXT_PUBLIC_CAPIA_E2E_HOOKS=1): the live EditorCore. */
		__capiaEditor?: EditorCore;
		/** E2E/benchmark only: domain builders used to compose rich scenes (text, masks). */
		__capiaE2E?: {
			buildTextElement: typeof buildTextElement;
			buildDefaultMaskInstance: typeof buildDefaultMaskInstance;
		};
		/** JSON-in/JSON-out bridge to the SAME editor instance the UI drives. */
		__capiaAgent?: {
			call: (request: { method: string; params?: RpcParams }) => unknown;
		};
	}
}

/**
 * Installs `window.__capiaAgent` on the live editor page. Gated by
 * NEXT_PUBLIC_CAPIA_AGENT_BRIDGE=1 (dev/E2E builds and the desktop host); disabled by default.
 */
export function installPageBridge(): void {
	if (
		process.env.NEXT_PUBLIC_CAPIA_E2E_HOOKS === "1" &&
		typeof window !== "undefined"
	) {
		window.__capiaEditor = EditorCore.getInstance();
		window.__capiaE2E = { buildTextElement, buildDefaultMaskInstance };
	}
	if (
		process.env.NEXT_PUBLIC_CAPIA_AGENT_BRIDGE !== "1" ||
		typeof window === "undefined"
	) {
		return;
	}
	const surface = new AgentSurface(EditorCore.getInstance());
	const dispatch = createDispatcher({ surface });
	window.__capiaAgent = {
		call: ({ method, params }) => dispatch({ method, params }),
	};
}
