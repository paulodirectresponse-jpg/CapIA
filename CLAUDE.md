# CLAUDE.md — Contexto para sessões futuras

**CapIA** (nome de trabalho) — editor de vídeo desktop, AI-first, focado em Direct Response, UGC, Ads e VSLs.
Estado atual e próxima missão: **leia `docs/STATUS.md` primeiro.**

## Princípios inegociáveis

1. **A timeline é a fonte de verdade.** Tudo que a IA produz é clip/propriedade comum, 100% editável manualmente.
2. **O editor funciona sem IA.** Nenhum caminho de edição, preview ou export depende de rede ou LLM.
3. **Uma única porta de escrita:** usuário, IA, CLI e futura API/MCP alteram o documento **somente** via Command Engine (comandos estruturados, validados, transacionais, com undo). A IA nunca usa computer-use nem manipula a UI.
4. **Tempo é inteiro.** Nunca use float de segundos no modelo. Use `Ticks` (i64, 705.600.000/s) e `Rational` — ver `docs/TIMELINE_ENGINE.md`.
5. **Core headless.** O núcleo Rust não depende de Tauri/UI. A UI é um cliente.
6. **Segredos nunca saem do core Rust**, nunca vão para logs, projetos, Git ou para a WebView.
7. **Brief ≠ Research ≠ Edit Plan ≠ Timeline.** Estados separados, persistidos separadamente.
8. **Local-first.** Mídia fica local; só sobe para a nuvem o mínimo necessário (áudio comprimido, frames amostrados).

## Mapa da documentação

| Documento | Conteúdo |
|---|---|
| `docs/STATUS.md` | Estado atual, missões concluídas, próximo passo |
| `docs/PRODUCT.md` | Visão, público, escopo, fora de escopo, glossário |
| `docs/ARCHITECTURE.md` | Subsistemas, crates/pacotes, processos, regras de dependência, stack |
| `docs/DATA_MODEL.md` | Entidades, invariantes, onde cada dado é persistido, formato de projeto |
| `docs/TIMELINE_ENGINE.md` | Tempo, frame accuracy, VFR, tracks, clips, nested sequences, operações |
| `docs/TIMELINE_UX.md` | Layout, interações, multi-sequence, metas de fluidez |
| `docs/COMMAND_SYSTEM.md` | Comandos, transações, validação, undo/redo, histórico, concorrência |
| `docs/PREVIEW_RENDER.md` | Render graph, compositor, preview, export, caches, proxies |
| `docs/AI_SYSTEM.md` | Pipeline, agentes, tools, Demand Interpreter, Reference Analyzer, memória |
| `docs/AI_PROVIDERS.md` | Providers, model registry, Brain Profile, Capability Router |
| `docs/ASSET_SYSTEM.md` | Bibliotecas, IDs, dedup, relink, offline, Asset Gateway, mídia gerada |
| `docs/SECURITY.md` | Credenciais, permissões de tools, prompt injection, superfície de ataque |
| `docs/ROADMAP.md` | 6 fases com critérios objetivos de conclusão |
| `docs/TEST_STRATEGY.md` | Estratégia de testes por subsistema |
| `docs/DECISIONS.md` | ADRs: decisões, alternativas, requisitos reformulados |

## Regras de trabalho

- Não pule fases do `docs/ROADMAP.md`. Uma missão por vez; atualize `docs/STATUS.md` ao final.
- Mudou uma decisão arquitetural? Registre um ADR novo em `docs/DECISIONS.md` (não reescreva o antigo; marque como *Superseded*).
- Respeite as regras de dependência entre crates (`docs/ARCHITECTURE.md` §4). UI, Timeline, Render, AI, Providers, Assets e Integrations não se acoplam diretamente.
- Testes de propriedade para tempo e Command Engine são obrigatórios (`docs/TEST_STRATEGY.md`).
- Nunca commitar chaves, `.env`, projetos de usuário ou mídia.
- Idioma da documentação: português; identificadores de código: inglês.
