import { describe, expect, test } from "bun:test";
import { wsolaStretch } from "../wsola";

const SAMPLE_RATE = 44100;

function sine({ frequency, seconds }: { frequency: number; seconds: number }) {
	const data = new Float32Array(Math.round(SAMPLE_RATE * seconds));
	for (let i = 0; i < data.length; i++) {
		data[i] = Math.sin((2 * Math.PI * frequency * i) / SAMPLE_RATE) * 0.8;
	}
	return data;
}

function estimateFrequency({ data }: { data: Float32Array }) {
	// Passagens ascendentes por zero, ignorando as bordas do overlap-add.
	const from = Math.floor(data.length * 0.1);
	const to = Math.floor(data.length * 0.9);
	let crossings = 0;
	for (let i = from + 1; i < to; i++) {
		if (data[i - 1] < 0 && data[i] >= 0) crossings++;
	}
	return crossings / ((to - from) / SAMPLE_RATE);
}

describe("wsolaStretch (substitui soundtouchjs)", () => {
	for (const rate of [0.5, 0.75, 1.25, 1.5, 2]) {
		test(`rate ${rate} mantém o tom e o comprimento`, () => {
			const input = sine({ frequency: 440, seconds: 2 });
			const outputLength = Math.ceil(input.length / rate);
			const [out] = wsolaStretch({
				channels: [input],
				rate,
				sampleRate: SAMPLE_RATE,
				outputLength,
			});
			expect(out.length).toBe(outputLength);
			expect(Math.abs(estimateFrequency({ data: out }) - 440)).toBeLessThan(8);
			const peak = out
				.slice(Math.floor(out.length * 0.1), Math.floor(out.length * 0.9))
				.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
			expect(peak).toBeGreaterThan(0.7);
			expect(peak).toBeLessThan(0.95);
		});
	}

	test("estéreo mantém os canais independentes e alinhados em fase", () => {
		const left = sine({ frequency: 330, seconds: 1 });
		const right = sine({ frequency: 660, seconds: 1 });
		const [l, r] = wsolaStretch({
			channels: [left, right],
			rate: 1.5,
			sampleRate: SAMPLE_RATE,
			outputLength: Math.ceil(left.length / 1.5),
		});
		expect(Math.abs(estimateFrequency({ data: l }) - 330)).toBeLessThan(10);
		expect(Math.abs(estimateFrequency({ data: r }) - 660)).toBeLessThan(15);
	});

	test("entrada vazia ou curta não explode", () => {
		const [out] = wsolaStretch({
			channels: [new Float32Array(10)],
			rate: 2,
			sampleRate: SAMPLE_RATE,
			outputLength: 5,
		});
		expect(out.length).toBe(5);
	});
});
