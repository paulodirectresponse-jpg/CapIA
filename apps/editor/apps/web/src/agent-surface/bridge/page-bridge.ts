import { EditorCore } from "@/core";
import { AgentSurface } from "../surface";
import { createDispatcher, type RpcParams } from "./dispatch";

declare global {
	interface Window {
		/** E2E/benchmark only (NEXT_PUBLIC_CAPIA_E2E_HOOKS=1): the live EditorCore. */
		__capiaEditor?: EditorCore;
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
