# @capia/editor-ui

Editor manual do CapIA (React). **Única porta de escrita da UI:** `store/controller.ts` (`EditorController`) — todo comando/undo/redo passa por `EditorClient.execute/undo/redo` e a réplica é atualizada pelos *patches* devolvidos (`@capia/engine-bindings`). Sem IA, sem rede: o editor funciona só com o engine.

- `components/` painéis (Projeto, Mídia, Timeline, Preview, Inspector, rail Texto/Legendas/Transições/Áudio, Exportação, Configurações, Histórico). Só **leem** do cliente: quadros, codificadores, miniaturas.
- `store/` controlador, `edit.ts` (comandos de edição puros e testáveis), `visuals.ts` (miniaturas/waveform), `perf.ts` (amostras para E2E/benchmark).
- `preview/` agendador *latest-wins*, apresentador WebGL/2D, fontes de quadro (`ipc` | `shared-buffer`), monitoração de áudio.
- `lib/` timecode, keymap (conflitos/reset), preferências à prova de corrupção (separadas do projeto), animação.
- `i18n/` `en` (referência) e `ptBR` (mesmas chaves, verificado em teste).
- Testes: `pnpm --filter @capia/editor-ui test` (inclui `architecture.test.ts`: a UI não escreve fora do Command Engine). E2E: `packages/e2e`.
- Ganchos de teste só com `?e2e=1` (`window.__capiaTimeline`, `window.__capiaPerf`).
