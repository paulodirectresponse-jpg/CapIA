/**
 * Serviços da plataforma que a UI pede ao shell (diálogos nativos). No navegador/E2E não há
 * seletor de arquivos nativo: os métodos devolvem `null` e a UI mostra campos de caminho.
 */
export interface PlatformServices {
  pickFiles(opts: {
    title: string;
    filters?: { name: string; extensions: string[] }[];
    multiple?: boolean;
  }): Promise<string[] | null>;
  pickFolder(opts: { title: string }): Promise<string | null>;
  pickSavePath(opts: {
    title: string;
    defaultName?: string;
    filters?: { name: string; extensions: string[] }[];
  }): Promise<string | null>;
  /** O shell tem diálogos nativos? (muda o rótulo dos botões). */
  readonly native: boolean;
}

export const noPlatform: PlatformServices = {
  native: false,
  pickFiles: () => Promise.resolve(null),
  pickFolder: () => Promise.resolve(null),
  pickSavePath: () => Promise.resolve(null),
};
