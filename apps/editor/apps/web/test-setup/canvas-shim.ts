/**
 * Test harness only (CapIA). `bun test` has no DOM/canvas. Text-measurement code
 * paths (`text/measure-element.ts`) need a 2D context with `measureText`. This
 * installs a deterministic OffscreenCanvas stand-in (0.6 em per glyph) so tests
 * that depend on *geometry relationships* (snapping to centers) run without a browser.
 * It is NOT a rendering implementation and is never imported by product code.
 */
type TextMetricsLike = {
	width: number;
	actualBoundingBoxLeft: number;
	actualBoundingBoxRight: number;
	actualBoundingBoxAscent: number;
	actualBoundingBoxDescent: number;
	fontBoundingBoxAscent: number;
	fontBoundingBoxDescent: number;
};

function fontPx(font: string): number {
	const match = /(\d+(?:\.\d+)?)px/.exec(font);
	return match ? Number(match[1]) : 16;
}

class StubContext2D {
	font = "16px sans-serif";
	letterSpacing = "0px";
	[key: string]: unknown;

	measureText(text: string): TextMetricsLike {
		const size = fontPx(this.font);
		const width = text.length * size * 0.6;
		return {
			width,
			actualBoundingBoxLeft: 0,
			actualBoundingBoxRight: width,
			actualBoundingBoxAscent: size * 0.8,
			actualBoundingBoxDescent: size * 0.2,
			fontBoundingBoxAscent: size * 0.8,
			fontBoundingBoxDescent: size * 0.2,
		};
	}
	save(): void {}
	restore(): void {}
}

class StubOffscreenCanvas {
	constructor(
		public width: number,
		public height: number,
	) {}
	getContext(kind: string): StubContext2D | null {
		return kind === "2d" ? new StubContext2D() : null;
	}
}

if (typeof globalThis.OffscreenCanvas === "undefined") {
	Object.defineProperty(globalThis, "OffscreenCanvas", {
		value: StubOffscreenCanvas,
		configurable: true,
		writable: true,
	});
}
