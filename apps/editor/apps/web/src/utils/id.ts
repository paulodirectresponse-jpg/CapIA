// CapIA: deterministic id scope. Inside `withIdSeed(seed, phase, fn)` every
// generateUUID() call returns a UUID-shaped hash of `${seed}/${phase}#${counter}`,
// so agent plans can derive the same ids on every (re)execution.
let ambientScope: string | null = null;
let ambientCounter = 0;
let ambientSeed: string | undefined;

function hash128(input: string): number[] {
	let h1 = 1779033703;
	let h2 = 3144134277;
	let h3 = 1013904242;
	let h4 = 2773480762;
	for (let i = 0; i < input.length; i++) {
		const k = input.charCodeAt(i);
		h1 = h2 ^ Math.imul(h1 ^ k, 597399067);
		h2 = h3 ^ Math.imul(h2 ^ k, 2869860233);
		h3 = h4 ^ Math.imul(h3 ^ k, 951274213);
		h4 = h1 ^ Math.imul(h4 ^ k, 2716044179);
	}
	h1 = Math.imul(h3 ^ (h1 >>> 18), 597399067);
	h2 = Math.imul(h4 ^ (h2 >>> 22), 2869860233);
	h3 = Math.imul(h1 ^ (h3 >>> 17), 951274213);
	h4 = Math.imul(h2 ^ (h4 >>> 19), 2716044179);
	return [
		(h1 ^ h2 ^ h3 ^ h4) >>> 0,
		(h2 ^ h1) >>> 0,
		(h3 ^ h1) >>> 0,
		(h4 ^ h1) >>> 0,
	];
}

export function deterministicUuid(input: string): string {
	const bytes: number[] = [];
	for (const word of hash128(input)) {
		bytes.push(
			(word >>> 24) & 255,
			(word >>> 16) & 255,
			(word >>> 8) & 255,
			word & 255,
		);
	}
	bytes[6] = (bytes[6] & 0x0f) | 0x40;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;
	const hex = bytes.map((b) => b.toString(16).padStart(2, "0"));
	return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex.slice(6, 8).join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10, 16).join("")}`;
}

/** Seed of the ambient scope (commands capture it at construction time). */
export function currentIdSeed(): string | undefined {
	return ambientSeed;
}

export function withIdSeed<T>({
	seed,
	phase,
	fn,
}: {
	seed: string | undefined;
	phase: string;
	fn: () => T;
}): T {
	if (seed === undefined) {
		return fn();
	}
	const previous = {
		scope: ambientScope,
		counter: ambientCounter,
		seed: ambientSeed,
	};
	ambientScope = `${seed}/${phase}`;
	ambientCounter = 0;
	ambientSeed = seed;
	try {
		return fn();
	} finally {
		ambientScope = previous.scope;
		ambientCounter = previous.counter;
		ambientSeed = previous.seed;
	}
}

/** The ids a command created under `seed` will receive during execution, in order. */
export function deriveExecutionIds({
	seed,
	count,
}: {
	seed: string;
	count: number;
}): string[] {
	return Array.from({ length: count }, (_, i) =>
		deterministicUuid(`${seed}/x#${i}`),
	);
}

export function generateUUID(): string {
	if (ambientScope !== null) {
		return deterministicUuid(`${ambientScope}#${ambientCounter++}`);
	}
	if (
		typeof crypto !== "undefined" &&
		typeof crypto.randomUUID === "function"
	) {
		return crypto.randomUUID();
	}

	const bytes = new Uint8Array(16);
	crypto.getRandomValues(bytes);

	bytes[6] = (bytes[6] & 0x0f) | 0x40;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;

	const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0"));

	return (
		hex.slice(0, 4).join("") +
		"-" +
		hex.slice(4, 6).join("") +
		"-" +
		hex.slice(6, 8).join("") +
		"-" +
		hex.slice(8, 10).join("") +
		"-" +
		hex.slice(10, 16).join("")
	);
}
