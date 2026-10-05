# @capia/desktop

Shell **fino** (Vite + Tauri 2). Nenhuma lógica de produto aqui (ADR-002).

- IPC fixo: `editor_call` (JSON) e `editor_call_binary` (quadros/miniaturas/PCM, empacotados `u32 LE | JSON | bytes`) → `capia_editor_api::Session` (método desconhecido ⇒ `UNKNOWN_METHOD`). Sem shell, sem `fs` amplo; capacidades: `core:default`, diálogo abrir/salvar.
- Preview P2: `preview_surface_init` / `preview_render_shared` (SharedBuffer do WebView2 via `capia-webview-surface`); sem WebView2 17 o front usa o IPC binário.
- `src/editorTransport.ts` (Tauri × HTTP do devserver), `src/sharedFrames.ts` (fonte de quadros P2), `src/platform.ts` (diálogos nativos).
- Dev/E2E sem Tauri: `cargo run -p capia-devserver` serve `apps/desktop/dist` + `/api/*`.
- Build: `pnpm build` (gera o WASM da timeline se faltar) · `pnpm desktop:build`.

## Fase 6 — instalador, self-test e suporte

`capia-desktop --version` e `--self-test [--require-media] [--out f]` (headless). Instalador NSIS por usuário, atualização assinada, crash report opt-in e diagnóstico: ver `docs/phase6/IMPL_DESKTOP_DISTRIBUTION.md`.
