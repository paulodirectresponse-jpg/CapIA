/**
 * Identidade de CONTEÚDO de um quadro de vídeo decodificado (2B.1).
 * O `CanvasSink` do mediabunny reutiliza os mesmos objetos Canvas (pool) e só reescreve os pixels;
 * identidade do objeto ≠ identidade do conteúdo temporal. Dois quadros distintos do mesmo arquivo nunca
 * compartilham (timestamp, duração), então esse par identifica o conteúdo.
 */
export function videoFrameRevision({
	timestamp,
	duration,
}: {
	timestamp: number;
	duration: number;
}): string {
	return `${timestamp}:${duration}`;
}

/** Fontes estáticas (imagem, sticker, gráfico): o conteúdo só muda quando o objeto muda. */
export const STATIC_SOURCE_REVISION = "static";
