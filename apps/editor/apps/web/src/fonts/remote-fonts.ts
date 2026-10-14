// Fontes do Google são baixadas de fonts.googleapis.com — rede e privacidade (princípio local-first do CapIA).
// Por padrão NENHUMA requisição é feita: o editor usa fontes do sistema. O usuário habilita explicitamente.
const STORAGE_KEY = "capia.remoteFonts";

export function isRemoteFontsEnabled({
	storage = typeof localStorage === "undefined" ? undefined : localStorage,
	envFlag = process.env.NEXT_PUBLIC_CAPIA_REMOTE_FONTS,
}: {
	storage?: Pick<Storage, "getItem"> | undefined;
	envFlag?: string | undefined;
} = {}): boolean {
	if (envFlag === "1") return true;
	try {
		return storage?.getItem(STORAGE_KEY) === "1";
	} catch {
		return false;
	}
}

export function setRemoteFontsEnabled({
	enabled,
	storage = typeof localStorage === "undefined" ? undefined : localStorage,
}: {
	enabled: boolean;
	storage?: Pick<Storage, "setItem" | "removeItem"> | undefined;
}): void {
	try {
		if (enabled) storage?.setItem(STORAGE_KEY, "1");
		else storage?.removeItem(STORAGE_KEY);
	} catch {
		// armazenamento indisponível: permanece desabilitado
	}
}
