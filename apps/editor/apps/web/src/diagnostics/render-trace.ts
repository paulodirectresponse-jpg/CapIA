/**
 * Trace de renderização para diagnosticar paridade preview/snapshot/export (CapIA, 2B.1).
 * Só existe em builds com NEXT_PUBLIC_CAPIA_E2E_HOOKS=1 e só grava com `window.__CAPIA_TRACE__ = true`;
 * fora disso `isRenderTraceEnabled()` é um curto-circuito (custo: uma leitura global).
 */

export type RenderTraceEvent =
	| {
			/** Bootstrap do projeto (2B.1B): clique, criação, gravação e navegação. */
			kind: "boot";
			stage: string;
			pathname: string;
			at: number;
			detail?: string;
	  }
	| {
			kind: "video-frame";
			site: "layer" | "backdrop";
			mediaId: string;
			timelineTimeTicks: number;
			timelineTimeSeconds: number;
			clipLocalTimeTicks: number;
			sourceTimeTicks: number;
			sourceTimeSeconds: number;
			frameTimestamp: number;
			frameDuration: number;
			canvasId: number;
			revision: string;
	  }
	| {
			kind: "texture";
			textureId: string;
			decision: "hit" | "miss";
			reason: "first" | "source" | "revision" | "size" | "same";
			uploaded: boolean;
			sourceId: number;
			revision: string;
	  };

const ids = new WeakMap<object, number>();
let nextId = 1;
const events: RenderTraceEvent[] = [];

export function isRenderTraceEnabled(): boolean {
	return (
		process.env.NEXT_PUBLIC_CAPIA_E2E_HOOKS === "1" &&
		typeof window !== "undefined" &&
		Reflect.get(window, "__CAPIA_TRACE__") === true
	);
}

/** Id estável por objeto (identidade do Canvas), para provar reutilização do mesmo objeto. */
export function objectId({ value }: { value: object }): number {
	const known = ids.get(value);
	if (known !== undefined) return known;
	const id = nextId++;
	ids.set(value, id);
	return id;
}

export function recordTrace({ event }: { event: RenderTraceEvent }): void {
	if (isRenderTraceEnabled()) events.push(event);
}

/** Marco do bootstrap de projeto; só grava com o trace ligado (E2E). */
export function recordBoot({
	stage,
	detail,
}: {
	stage: string;
	detail?: string;
}): void {
	if (!isRenderTraceEnabled()) return;
	events.push({
		kind: "boot",
		stage,
		pathname: window.location.pathname,
		at: Math.round(performance.now()),
		detail,
	});
}

export function readTrace(): RenderTraceEvent[] {
	return events.slice();
}

export function clearTrace(): void {
	events.length = 0;
}

export function installRenderTraceHooks(): void {
	if (
		process.env.NEXT_PUBLIC_CAPIA_E2E_HOOKS !== "1" ||
		typeof window === "undefined"
	) {
		return;
	}
	Object.defineProperty(window, "__capiaTrace", {
		value: { read: readTrace, clear: clearTrace },
		configurable: true,
	});
}
