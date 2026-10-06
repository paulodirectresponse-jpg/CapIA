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
| Jornada humana de 25 passos | `packages/e2e/tests/rc2-journey.spec.ts` — passou localmente (Chromium + devserver release + FFmpeg real) **e** no CI (Linux e Windows/Tauri/WebView2) |
| CI completo no HEAD `a2cd6a9` (branch `claude/rc2-stabilization-ux`) | Run [37421782016](https://github.com/paulodirectresponse-jpg/CapIA/actions/runs/37421782016): todos os jobs verdes (Linux headless, TypeScript, E2E Linux, Rust+desktop Windows, E2E Windows, arquitetura/licenças/segredos). Runs anteriores (133, 134) falharam por causas minhas — clippy, ESLint, gitleaks, dois passos do roteiro no Windows — e foram corrigidas |
| Instalador Windows (NSIS) + smoke (instalar, self-test instalado, primeira execução, desinstalar sem apagar projetos, reinstalar) | Run [37425814693](https://github.com/paulodirectresponse-jpg/CapIA/actions/runs/37425814693) (`workflow_dispatch`, HEAD `a2cd6a9`): sucesso. Artefato `installer-windows-dev-test`, ~72 MB, `sha256:76fe4a4f06c7572b8cff226dfe71a7111176ab0db63fd7f4086433de7afae9d5` — **candidato SEM assinatura de código** |

## Achado pelo próprio teste
O primeiro E2E reproduziu o erro técnico de transição sem handles (que o editor do teste manual relatou). Corrigido pelo ADR-120.

## Não executado / externo (não marcar como aceito)
- **OpenAI real:** sem chave no ambiente e sem rede para `api.openai.com`; o provedor foi exercitado só por servidor falso em loopback.
- Chat com edição (preview → aprovação → aplicar → desfazer): coberto pelos testes Rust do assistente (`crates/capia-intelligence/tests/assistant.rs`); **não** está no E2E de UI.
- Assinatura de código (sem certificado real: o instalador é um candidato **não assinado**), Windows 10/11 limpos, beta humano, H.264/AAC jurídico, pentest independente.
- Trilhas do escopo RC2 **não** concluídas: layout/rail reduzido e inspetor contextual, acessibilidade/resolução, texto/legendas simplificados, sequences aninhadas, medição de gargalo do export.
