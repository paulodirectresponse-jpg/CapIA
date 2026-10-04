import type { PlatformServices } from "@capia/editor-ui";

/** Diálogos nativos via plugin oficial do Tauri (carregado sob demanda: não existe no navegador). */
export const tauriPlatform: PlatformServices = {
  native: true,
  async pickFiles({ title, filters, multiple }) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ title, multiple: multiple ?? false, ...(filters ? { filters } : {}) });
    if (r === null) return null;
    return Array.isArray(r) ? r : [r];
  },
  async pickFolder({ title }) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ title, directory: true });
    return typeof r === "string" ? r : null;
  },
  async pickSavePath({ title, defaultName, filters }) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({
      title,
      ...(defaultName ? { defaultPath: defaultName } : {}),
      ...(filters ? { filters } : {}),
    });
  },
};
