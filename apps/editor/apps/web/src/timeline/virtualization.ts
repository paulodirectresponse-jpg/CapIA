import { getTimelinePixelsPerSecond } from "@/timeline/pixel-utils";
import { TICKS_PER_SECOND } from "@/wasm";

/**
 * Janela temporal visível da timeline (CapIA, spike de virtualização). A UI renderiza só os clips
 * que intersectam a janela (+ overscan) e os que a interação precisa manter montados (selecionados,
 * arrastados, alvo de drop). O domínio (EditorCore/commands/renderer) não muda.
 */
export interface VisibleTimeWindow {
	/** início da janela (ticks) */
	start: number;
	/** fim da janela (ticks) */
	end: number;
}

/** Granularidade da janela em px: a janela só muda ao cruzar um bloco (menos re-render ao rolar). */
export const WINDOW_CHUNK_PX = 400;
export const WINDOW_OVERSCAN_PX = 800;

export function computeVisibleTimeWindow({
	scrollLeft,
	viewportWidth,
	zoomLevel,
	overscanPx = WINDOW_OVERSCAN_PX,
	chunkPx = WINDOW_CHUNK_PX,
}: {
	scrollLeft: number;
	viewportWidth: number;
	zoomLevel: number;
	overscanPx?: number;
	chunkPx?: number;
}): VisibleTimeWindow {
	const pixelsPerSecond = getTimelinePixelsPerSecond({ zoomLevel });
	const quantizedLeft = Math.floor(Math.max(0, scrollLeft) / chunkPx) * chunkPx;
	const quantizedRight =
		Math.ceil((Math.max(0, scrollLeft) + viewportWidth) / chunkPx) * chunkPx;
	const toTicks = (px: number) => (px / pixelsPerSecond) * TICKS_PER_SECOND;
	return {
		start: Math.floor(toTicks(Math.max(0, quantizedLeft - overscanPx))),
		end: Math.ceil(toTicks(quantizedRight + overscanPx)),
	};
}

export function isInWindow({
	startTime,
	duration,
	window,
}: {
	startTime: number;
	duration: number;
	window: VisibleTimeWindow;
}): boolean {
	return startTime + duration >= window.start && startTime <= window.end;
}

/** Elementos a montar: os que intersectam a janela + os que a interação exige (`keep`). */
export function selectVisibleElements<
	T extends { id: string; startTime: number; duration: number },
>({
	elements,
	window,
	keep,
}: {
	elements: readonly T[];
	window: VisibleTimeWindow | null;
	keep: (id: string) => boolean;
}): readonly T[] {
	if (window === null) return elements;
	return elements.filter(
		(element) =>
			isInWindow({
				startTime: element.startTime,
				duration: element.duration,
				window,
			}) || keep(element.id),
	);
}

/**
 * Virtualização ligada por padrão. Só builds com NEXT_PUBLIC_CAPIA_E2E_HOOKS=1 permitem desligá-la
 * em runtime (`window.__CAPIA_NO_VIRTUALIZATION__ = true`) para medir antes/depois no benchmark.
 */
export function isVirtualizationEnabled(): boolean {
	if (
		process.env.NEXT_PUBLIC_CAPIA_E2E_HOOKS === "1" &&
		typeof window !== "undefined" &&
		Reflect.get(window, "__CAPIA_NO_VIRTUALIZATION__") === true
	) {
		return false;
	}
	return true;
}

/**
 * Virtualização VERTICAL de faixas (2B). Linhas são posicionadas em absoluto (top/height em px), então
 * basta decidir quais montar. A janela é quantizada em blocos (menos re-render ao rolar).
 */
export interface VisibleRowWindow {
	top: number;
	bottom: number;
}

export const ROW_WINDOW_CHUNK_PX = 120;
export const ROW_WINDOW_OVERSCAN_PX = 300;

export function computeVisibleRowWindow({
	scrollTop,
	viewportHeight,
	overscanPx = ROW_WINDOW_OVERSCAN_PX,
	chunkPx = ROW_WINDOW_CHUNK_PX,
}: {
	scrollTop: number;
	viewportHeight: number;
	overscanPx?: number;
	chunkPx?: number;
}): VisibleRowWindow {
	const top = Math.max(0, scrollTop);
	return {
		top: Math.max(0, Math.floor(top / chunkPx) * chunkPx - overscanPx),
		bottom: Math.ceil((top + viewportHeight) / chunkPx) * chunkPx + overscanPx,
	};
}

/** Linhas a montar: as que intersectam a janela + as que a interação exige (`keep`). */
export function selectVisibleRows<
	T extends { id: string; top: number; height: number },
>({
	rows,
	window,
	keep,
}: {
	rows: readonly T[];
	window: VisibleRowWindow | null;
	keep: (id: string) => boolean;
}): readonly T[] {
	if (window === null) return rows;
	return rows.filter(
		(row) =>
			(row.top + row.height >= window.top && row.top <= window.bottom) ||
			keep(row.id),
	);
}
