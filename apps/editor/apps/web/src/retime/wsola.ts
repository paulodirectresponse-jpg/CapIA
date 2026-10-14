// Time-stretch com preservação de tom (WSOLA — waveform similarity overlap-add).
// Implementação original do CapIA (MIT); substitui `soundtouchjs` (LGPL-2.1) — ver docs/adoption/PHASE_2B.md.
// Algoritmo clássico: quadros Hann com 50% de sobreposição na síntese; o ponto de leitura de cada quadro
// é ajustado dentro de uma janela de busca para maximizar a correlação com a continuação natural do anterior.

const FRAME_SECONDS = 0.04;
const SEARCH_FRACTION = 0.5; // da meia janela
const CORRELATION_STRIDE = 4;
const SEARCH_STRIDE = 2;

function evenAtLeast({ value, minimum }: { value: number; minimum: number }) {
	const rounded = Math.max(minimum, Math.round(value));
	return rounded % 2 === 0 ? rounded : rounded + 1;
}

function sampleAt({ data, index }: { data: Float32Array; index: number }) {
	return index >= 0 && index < data.length ? data[index] : 0;
}

export function wsolaStretch({
	channels,
	rate,
	sampleRate,
	outputLength,
}: {
	channels: Float32Array[];
	rate: number;
	sampleRate: number;
	outputLength: number;
}): Float32Array<ArrayBuffer>[] {
	const frame = evenAtLeast({ value: sampleRate * FRAME_SECONDS, minimum: 64 });
	const hop = frame / 2;
	const search = Math.max(1, Math.floor(hop * SEARCH_FRACTION));
	const window = new Float32Array(frame);
	for (let i = 0; i < frame; i++) {
		// Hann periódica: duas cópias deslocadas de meia janela somam exatamente 1.
		window[i] = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / frame);
	}

	const inputLength = channels[0]?.length ?? 0;
	const mono = new Float32Array(inputLength);
	for (const channel of channels) {
		for (let i = 0; i < inputLength; i++) {
			mono[i] += channel[i] / channels.length;
		}
	}

	const outputs = channels.map(() => new Float32Array(outputLength + frame));
	let previousStart = 0;
	for (let k = 0; k * hop < outputLength; k++) {
		let start = 0;
		if (k > 0) {
			const ideal = Math.round(k * hop * rate);
			let bestScore = Number.NEGATIVE_INFINITY;
			start = ideal;
			for (let delta = -search; delta <= search; delta += SEARCH_STRIDE) {
				const candidate = ideal + delta;
				let score = 0;
				for (let i = 0; i < hop; i += CORRELATION_STRIDE) {
					score +=
						sampleAt({ data: mono, index: candidate + i }) *
						sampleAt({ data: mono, index: previousStart + hop + i });
				}
				if (score > bestScore) {
					bestScore = score;
					start = candidate;
				}
			}
		}
		const outStart = k * hop;
		for (let c = 0; c < channels.length; c++) {
			const out = outputs[c];
			for (let i = 0; i < frame; i++) {
				out[outStart + i] +=
					window[i] * sampleAt({ data: channels[c], index: start + i });
			}
		}
		previousStart = start;
	}
	return outputs.map((out) => out.slice(0, outputLength));
}
