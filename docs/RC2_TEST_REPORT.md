# RC2 — relatório de testes

Escopo: correção após o teste manual do RC1. Cada linha diz **o que foi executado** e onde; o que depende de terceiros fica na seção final.

## Executado e verificado

| Item | Evidência |
|---|---|
| CI Windows do RC1 (`capia-desktop --lib`, `capia-server --test external_flow`) | Run 119 (HEAD `b183b5a`, `claude/phase6-finalization`): todos os jobs verdes |
| Dissolução entre clipes inteiros (ADR-120) | `crates/capia-render/tests/editor_render.rs::dissolve_between_whole_clips_without_handles_blends_and_holds_edges`; `crates/capia-commands/tests/editor.rs::transitions_need_adjacency_but_not_handles`; E2E passo 8 |
| Keyframes interpolam no render e é determinístico | `editor_render.rs::scale_keyframes_grow_the_picture_and_render_is_deterministic`; E2E passos 11–13 e 17 (persistência) |
| Estabilidade da qualidade automática do preview | `packages/editor-ui/src/preview/scheduler.test.ts` (`AutoQuality`) |
| Adapter OpenAI `/v1/responses` | `crates/capia-ai/src/providers/openai_responses.rs` (unit) e `crates/capia-ai/tests/openai_responses.rs` (servidor falso: stream de texto + uso, function call em stream, 401 sem vazar a chave) |
| Conectar IA / Brain Profile `auto` / whisper-1 só como STT | `crates/capia-intelligence/tests/connect.rs`; `connect.rs` (unit: escolha de modelo) |
| Contexto da UI no chat (ids sanitizados) | `assistant.rs::ui_context_tests` |
| Error Boundary, recentes, estatísticas de export, nomes de track | `packages/editor-ui` (Vitest, 70 testes) |
| Jornada humana de 25 passos | `packages/e2e/tests/rc2-journey.spec.ts` — passou localmente (Chromium + devserver release + FFmpeg real); CI: ver abaixo |

## Achado pelo próprio teste
O primeiro E2E reproduziu o erro técnico de transição sem handles (que o editor do teste manual relatou). Corrigido pelo ADR-120.

## Não executado / externo (não marcar como aceito)
- **OpenAI real:** sem chave no ambiente e sem rede para `api.openai.com`; o provedor foi exercitado só por servidor falso em loopback.
- Chat com edição (preview → aprovação → aplicar → desfazer): coberto pelos testes Rust do assistente (`crates/capia-intelligence/tests/assistant.rs`); **não** está no E2E de UI.
- Instalador RC2, smoke do instalador, Windows 10/11 limpos, assinatura, beta humano, H.264/AAC jurídico, pentest independente.
- Trilhas do escopo RC2 **não** concluídas: layout/rail reduzido e inspetor contextual, acessibilidade/resolução, texto/legendas simplificados, sequences aninhadas, medição de gargalo do export.
