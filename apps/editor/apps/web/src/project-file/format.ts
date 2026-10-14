/**
 * Formato de projeto em arquivo do CapIA Editor (ADR-132). O projeto do OpenCut passa a ser salvo como
 * um documento JSON versionado, íntegro (SHA-256) e portátil; o IndexedDB/OPFS vira só cache/autosave.
 * Escrita em disco é responsabilidade do host Rust (temp + fsync + rename); este módulo é puro.
 *
 * Regras: sem segredos, sem bytes de mídia (só referências por caminho + SHA-256 para relink),
 * tempo sempre inteiro (ticks já vêm inteiros no documento).
 */
import type { TProject } from "@/project/types";

export const PROJECT_FILE_FORMAT = "capia.project" as const;
export const PROJECT_FILE_VERSION = 1 as const;

export interface MediaReference {
	id: string;
	name: string;
	type: string;
	/** relativo à pasta do projeto, quando a mídia está dentro dela */
	relativePath: string | null;
	absolutePath: string | null;
	sizeBytes: number;
	/** `sha256:<hex>` — identidade por conteúdo (relink só aceita o mesmo conteúdo) */
	sha256: string;
}

export interface ProjectFile {
	format: typeof PROJECT_FILE_FORMAT;
	formatVersion: typeof PROJECT_FILE_VERSION;
	/** versão do esquema do documento do editor (TProject.version) */
	editorSchemaVersion: number;
	/** revisão do documento no momento do salvamento (CAS da AgentSurface) */
	revision: number;
	savedAt: string;
	project: unknown;
	media: MediaReference[];
	/** SHA-256 do conteúdo canônico de project + media */
	contentSha256: string;
}

export type ProjectFileErrorCode =
	| "NOT_JSON"
	| "WRONG_FORMAT"
	| "UNSUPPORTED_VERSION"
	| "INTEGRITY"
	| "INVALID";

export class ProjectFileError extends Error {
	readonly code: ProjectFileErrorCode;
	constructor({
		code,
		message,
	}: {
		code: ProjectFileErrorCode;
		message: string;
	}) {
		super(message);
		this.name = "ProjectFileError";
		this.code = code;
	}
}

/** JSON com chaves ordenadas: o mesmo conteúdo sempre gera os mesmos bytes (hash estável). */
export function canonicalJson(value: unknown): string {
	if (value === null || typeof value !== "object") {
		return JSON.stringify(value) ?? "null";
	}
	if (Array.isArray(value)) {
		return `[${value.map((item) => canonicalJson(item)).join(",")}]`;
	}
	const entries = Object.entries(value)
		.filter(([, v]) => v !== undefined)
		.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
	return `{${entries
		.map(([k, v]) => `${JSON.stringify(k)}:${canonicalJson(v)}`)
		.join(",")}}`;
}

export async function sha256Hex(text: string): Promise<string> {
	const bytes = new TextEncoder().encode(text);
	const digest = await crypto.subtle.digest("SHA-256", bytes);
	return [...new Uint8Array(digest)]
		.map((b) => b.toString(16).padStart(2, "0"))
		.join("");
}

async function contentHash({
	project,
	media,
}: {
	project: unknown;
	media: MediaReference[];
}) {
	return `sha256:${await sha256Hex(canonicalJson({ project, media }))}`;
}

export async function serializeProjectFile({
	project,
	media,
	revision,
	savedAt,
}: {
	project: TProject;
	media: MediaReference[];
	revision: number;
	savedAt: string;
}): Promise<string> {
	// passa por JSON para fixar o que realmente é persistido (Date → ISO, undefined removido)
	const plain: unknown = JSON.parse(JSON.stringify(project));
	const file: ProjectFile = {
		format: PROJECT_FILE_FORMAT,
		formatVersion: PROJECT_FILE_VERSION,
		editorSchemaVersion: project.version,
		revision,
		savedAt,
		project: plain,
		media,
		contentSha256: await contentHash({ project: plain, media }),
	};
	return `${canonicalJson(file)}\n`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

export async function parseProjectFile({
	text,
	maxSupportedEditorSchema,
}: {
	text: string;
	/** esquema mais novo do editor que este build entende; arquivo mais novo é rejeitado intacto */
	maxSupportedEditorSchema: number;
}): Promise<ProjectFile> {
	let raw: unknown;
	try {
		raw = JSON.parse(text);
	} catch {
		throw new ProjectFileError({
			code: "NOT_JSON",
			message: "o arquivo não é JSON válido",
		});
	}
	if (!isRecord(raw) || raw.format !== PROJECT_FILE_FORMAT) {
		throw new ProjectFileError({
			code: "WRONG_FORMAT",
			message: "não é um projeto do CapIA",
		});
	}
	if (raw.formatVersion !== PROJECT_FILE_VERSION) {
		throw new ProjectFileError({
			code: "UNSUPPORTED_VERSION",
			message: `formatVersion ${String(raw.formatVersion)} não suportada (esperada ${PROJECT_FILE_VERSION})`,
		});
	}
	const editorSchema = raw.editorSchemaVersion;
	if (typeof editorSchema !== "number" || !Number.isInteger(editorSchema)) {
		throw new ProjectFileError({
			code: "INVALID",
			message: "editorSchemaVersion ausente",
		});
	}
	if (editorSchema > maxSupportedEditorSchema) {
		throw new ProjectFileError({
			code: "UNSUPPORTED_VERSION",
			message: `projeto salvo por um editor mais novo (esquema ${editorSchema} > ${maxSupportedEditorSchema})`,
		});
	}
	if (
		!isRecord(raw.project) ||
		!Array.isArray(raw.media) ||
		typeof raw.revision !== "number" ||
		typeof raw.savedAt !== "string" ||
		typeof raw.contentSha256 !== "string"
	) {
		throw new ProjectFileError({
			code: "INVALID",
			message: "estrutura do projeto incompleta",
		});
	}
	// sem `as` de domínio: o conteúdo é revalidado pelas migrations do editor ao abrir
	const media: MediaReference[] = raw.media.map((item: unknown) => {
		if (
			!isRecord(item) ||
			typeof item.id !== "string" ||
			typeof item.name !== "string" ||
			typeof item.type !== "string" ||
			typeof item.sizeBytes !== "number" ||
			typeof item.sha256 !== "string" ||
			!(item.relativePath === null || typeof item.relativePath === "string") ||
			!(item.absolutePath === null || typeof item.absolutePath === "string")
		) {
			throw new ProjectFileError({
				code: "INVALID",
				message: "referência de mídia inválida",
			});
		}
		return {
			id: item.id,
			name: item.name,
			type: item.type,
			relativePath: item.relativePath,
			absolutePath: item.absolutePath,
			sizeBytes: item.sizeBytes,
			sha256: item.sha256,
		};
	});
	const expected = await contentHash({ project: raw.project, media });
	if (expected !== raw.contentSha256) {
		throw new ProjectFileError({
			code: "INTEGRITY",
			message:
				"o conteúdo não confere com o SHA-256 gravado (arquivo corrompido ou editado)",
		});
	}
	return {
		format: PROJECT_FILE_FORMAT,
		formatVersion: PROJECT_FILE_VERSION,
		editorSchemaVersion: editorSchema,
		revision: raw.revision,
		savedAt: raw.savedAt,
		project: raw.project,
		media,
		contentSha256: raw.contentSha256,
	};
}

/** Relink: aceita o arquivo candidato somente se o conteúdo for o mesmo (nunca por nome). */
export function mediaMatches({
	reference,
	sizeBytes,
	sha256,
}: {
	reference: MediaReference;
	sizeBytes: number;
	sha256: string;
}): boolean {
	return reference.sizeBytes === sizeBytes && reference.sha256 === sha256;
}
