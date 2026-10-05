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
| `docs/OPEN_SOURCE_AUDIT.md` | Due diligence de projetos open-source: licenças, reuso, estratégia recomendada (M02) |
| `docs/PROVENANCE.md` | Política obrigatória de proveniência/licenças de terceiros + registro (ADR-031) |
| `crates/capia-store/README.md` · `crates/capia-cli/README.md` | Formato `.capia`/persistência (ADR-042..044) e CLI `capia` (M06) |
| `crates/capia-media/README.md` · `crates/capia-assets/README.md` | Probe/ffprobe e domínio de assets (ADR-046..049, M07); índice/decode/waveform/proxy, impressão rápida, cache v2, relink em lote (ADR-053..058, M08) |
| `crates/capia-jobs/README.md` | Executor de jobs: prioridades sem starvation, cancelamento real, dedup (ADR-052, M08) |
| `crates/capia-render/README.md` · `crates/capia-decode/README.md` · `crates/capia-preview/README.md` | Render graph + compositor CPU de referência + mixer (ADR-063..065); decode persistente, cache de quadros/PCM, supersession (ADR-059..062); preview headless (ADR-066); export/encoders em `capia-media`/`capia-project`/`capia-cli` (ADR-067/068) |
| `packages/editor-ui/README.md` · `packages/ui-timeline/README.md` · `packages/e2e/README.md` · `apps/desktop/README.md` · `crates/capia-webview-surface/README.md` | Editor da Fase 3: UI, timeline em canvas + WASM, E2E, shell Tauri, superfície P2 (ADR-070..077) |
| `tools/phase3-acceptance/` | Pacote de aceitação humana/hardware da Fase 3 (tarefa UGC, validador, residual de GPU) — **nunca fabricar resultados** |
| `crates/capia-secrets/README.md` · `crates/capia-ai/README.md` · `crates/capia-intelligence/README.md` · `tools/phase4-acceptance/README.md` | Fase 4: segredos/redação, providers/router/tools, pipelines de inteligência e serviço `ai.*` (ADR-078..086); pacote de aceitação (segurança, cenas, DemandSpec) — **nunca fabricar resultados externos** |
| `docs/phase4/` | Especificações da Fase 4 (completion, providers/segurança, inteligência/testes) |
| `docs/spikes/README.md` | Resultados dos spikes S1–S7 (M03) e relatórios individuais |
| `tests/acceptance/` | Suíte de aceitação de comportamento da timeline (critério da Fase 2, ADR-036) |
| `tools/` | `check-architecture.mjs` (fronteiras), `check-licenses.mjs` (licenças JS), `s1-preview-spike/` (medição S1 em Windows) |

## Gates de fase (ADR-037)

- **Fase 2 (motor headless) pode iniciar com OD-1 aberto.** Não construir preview embutido nem UI de editor nela.
- **OD-1 FECHADA (ADR-069): presenter do preview = P2 (SharedBuffer → canvas WebGL); P1 eliminado (airspace medido).** A Fase 3 pode iniciar, mas a primeira entrega de preview deve validar CPU/pacing do P2 em GPU real (`tools/s1-preview-spike`, `run.ps1 -P2Res 1280x720`); gatilho de reabertura na ADR-069. `OUTPUT-H264` (export MP4/H.264 confiável no Windows) deve estar definido antes da entrega do Editor.
- As 8 decisões `D-S7-*` são **definitivas** (ADR-039, PO na M05). Mudá-las exige novo ADR e atualização dos cenários em `tests/acceptance/timeline/`.

## Comandos (workspace)

```
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo check --target wasm32-unknown-unknown -p capia-time -p capia-model -p capia-commands   # núcleo sem IO
pnpm install && pnpm lint && pnpm format:check && pnpm typecheck && pnpm test && pnpm test:tools && pnpm build
pnpm check:arch && pnpm check:licenses && cargo deny check licenses bans sources
pnpm check:parity         # paridade nativo × WASM (precisa do target wasm32-wasip1)
cargo test --release -p capia-commands --test perf -- --ignored --nocapture   # metas de desempenho
cargo test --release -p capia-store --test perf -- --ignored --nocapture      # 10k clips: abrir/commit/reload
CAPIA_PROP_CASES=5000 cargo test --release -p capia-store --test properties   # salvar/reabrir (padrão 1000/gerador)
cargo run -p capia-cli -- create|inspect|validate|apply|undo|redo|history|dump <arq.capia> ...
cargo run -p capia-cli -- asset import|list|inspect|verify|relink|thumbnail <arq.capia> ... ; media probe <arquivo>
cargo test --release -p capia-project --test perf_assets -- --ignored --nocapture   # hash/probe/import, 10k assets
cargo test --release -p capia-jobs --test perf -- --ignored --nocapture; cargo test --release -p capia-assets --test perf_hash -- --ignored --nocapture
cargo test --release -p capia-project --test perf_media -- --ignored --nocapture --test-threads=1   # índice/decode/waveform/proxy/import assíncrono
CAPIA_MEDIA_PROP_CASES=40 cargo test --release -p capia-project --test properties_media   # pipeline ponta a ponta (FFmpeg real)
python3 tools/mutation-m08.py [id…]   # 13 mutações da M08 (árvore limpa; restaura o arquivo sempre)
python3 tools/mutation-phase2.py [id…]   # 16 mutações da Fase 2 (render/decode/áudio/export)
cargo test --release -p capia-render --test perf -- --ignored --nocapture; cargo test --release -p capia-decode --test perf -- --ignored --nocapture --test-threads=1; cargo test --release -p capia-project --test perf_render -- --ignored --nocapture --test-threads=1
CAPIA_RENDER_PROP_CASES=80 cargo test --release -p capia-project --test properties_render   # render × oráculo de decode direto
cargo run -p capia-cli -- media encoders | render frame|audio ... | export intermediate|mp4 ...   # Fase 2
cargo run -p capia-cli -- job list|status|cancel <arq.capia> ... ; media index|frame|waveform|proxy ... ; asset force-relink|relink-folder ... ; cache info|clean <arq.capia>
CAPIA_REQUIRE_FFMPEG=1 cargo test -p capia-media -p capia-assets -p capia-project -p capia-cli   # CI: sem FFmpeg FALHA
tools/gen-media-fixtures.sh   # regenera tests/fixtures/media (versionadas)
pnpm build:wasm           # núcleo da timeline em WASM (gitignored; o CI gera antes dos testes do front)
pnpm --filter @capia/e2e e2e   # E2E Playwright (precisa de pnpm build + cargo build --release -p capia-devserver); ver packages/e2e/README.md
cargo run -p capia-devserver   # servidor HTTP local SÓ dev/E2E (engine real em 127.0.0.1)
node tools/phase4-acceptance/security/run.mjs      # Fase 4: suíte de segurança (summary JSON em target/phase4-acceptance/)
node tools/phase4-acceptance/reference-analyzer/run.mjs | node tools/phase4-acceptance/demand-spec/run.mjs   # cenas ≤5 % / DemandSpec ≥10 briefs (--live/--corpus = externos)
cargo test -p capia-ai -p capia-secrets -p capia-intelligence -p capia-store   # Fase 4 (Replay + servidores HTTP falsos; sem chave)
pnpm desktop:build        # tauri build --no-bundle  (o front precisa estar construído: pnpm --filter @capia/desktop build)
```

Direção de dependência (verificada por `pnpm check:arch`; Fase 4: `capia-secrets` (folha) → `capia-ai` (único HTTP de saída; não conhece projeto/documento) → `capia-intelligence` (cliente do Engine API via `capia-editor-api`) → hospedado por `capia-devserver`/`capia-desktop`; `capia-store` só usa `capia-secrets` para redigir): `capia-time → capia-model → capia-commands`; `{time, model} → capia-render → capia-preview`; `{time, media} → capia-decode`; `capia-time → capia-media`; `{model, media} → capia-assets`; `capia-jobs` (solto) `→ {store, project}`; `{commands, assets, jobs} → capia-store → capia-project (+ decode, render) → {capia-cli, apps/desktop}`. O núcleo puro (time/model/commands) não conhece Tauri, UI, IA, providers, render, FFmpeg nem SQLite (compila para WASM); só `capia-store` fala com SQLite (`rusqlite` bundled); só `capia-media` executa FFmpeg/ffprobe (processo externo, sem shell, com timeout e teto de saída). Novo crate/pacote ⇒ atualizar a matriz em `tools/check-architecture.mjs` de propósito (ADR-038).

## Regras de trabalho

- **Persistência (M06):** o engine persiste **antes** de publicar (`Journal`); uma única transação SQLite por commit; `synchronous=FULL`; migrations só para frente e explícitas; schema mais novo é rejeitado sem tocar o arquivo. Mudar o schema ⇒ nova migration + teste + ADR. Erros do store são sempre `StoreError` estruturado (nunca SQLite bruto).

- **Assets/mídia (M07):** o arquivo de mídia é **entrada hostil** e nunca entra no `.capia`. Identidade = conteúdo (`sha256:` em streaming), nunca o caminho. O catálogo (hash/caminho/metadados/disponibilidade) vive em tabelas do `.capia` (schema 2) e **não** entra no documento nem no undo; import = documento + catálogo na MESMA transação. Relink só aceita o mesmo conteúdo. O cache (`<proj>.capia-cache/`) é descartável — nada essencial vive só nele. Todo uso do ffprobe passa pelo trait `MediaProbe`; nenhum outro crate vê JSON do FFmpeg.
- **Jobs/derivados (M08):** o executor (`capia-jobs`) **só calcula**; documento e catálogo mudam na thread do projeto (`Project::pump`) pelo engine — workers escrevem só **cache**. Estado de jobs/tickets é operacional (schema 3, fora do undo); reabrir marca o que estava em andamento como `interrupted`, **nunca** `completed`. Impressão rápida (`fp1:`) é triagem, **nunca identidade** (só o SHA-256 decide). Derivados (índice `CIDX`, waveform `CWFM`, proxy, miniatura) vão para o cache por `CacheDir::produce` (lock por chave → temp → validar → `rename`); o **proxy nunca é fonte de verdade**. Relink em lote **nunca por nome** (tamanho → impressão → SHA-256); force relink só via `update_asset` (clips dependentes validados, sem trim silencioso). Um único processo é dono do executor (lock do SO). Falhas de crash: feature `failpoints` (só testes).
- **Render/export (Fase 2):** `capia-render` é puro (sem IO/FFmpeg) e o **mesmo** `render_frame` serve preview e export (ADR-063..066); a fonte de mídia é **sempre o original** — o proxy nunca é fonte de decode nem de export. Render/export **nunca escrevem** no documento ou no catálogo. Tempo no render é `Ticks`/`Rational`/amostras inteiras (nunca float de segundos). Export só por `EncoderCapability` **aprovado**: `libx264/libx265` e qualquer encoder fora do catálogo são proibidos, **sem fallback silencioso**; `mpeg4-reference` só por pedido explícito e não é H.264 (ADR-067). Todo export escreve em staging, valida (ffprobe) e publica por `rename` — kill nunca deixa saída parcial como final (ADR-068). Cache de quadros/PCM tem orçamento em **bytes** e chave por conteúdo+namespace de projeto (ADR-060/062). Fases: a Fase 2 está concluída; OD-1 está fechada (ADR-069).
- **Editor (Fase 3):** **toda** mutação do documento a partir da UI é comando/transação do Command Engine — só `store/controller.ts` chama `execute/undo/redo` (verificado por `architecture.test.ts`). Preferências de UI ficam fora do projeto. Sem IPC por quadro de arrasto (snap/grupo/colocação = WASM do core, ADR-070). Preview: render **fora do lock da sessão** (`Session::begin`/`Job`, ADR-073), P2 por SharedBuffer com queda para IPC (ADR-074); `unsafe` só em `capia-webview-surface` e `capia-timeline-wasm`. O export só oferece encoders aprovados (ADR-075) e **não** declara resolvida a decisão jurídica de H.264. A Fase 3 está em `ENGINEERING COMPLETE — HUMAN ACCEPTANCE PENDING`: não marque as caixas humanas/GPU do ROADMAP sem resultados reais (`tools/phase3-acceptance/validate-results.mjs`). **Fase 4: ver regra "IA (Fase 4)" abaixo.**
- **IA (Fase 4):** a IA é só **cliente do Engine API**. Escrita só por `preview → apply_plan` com `Actor::Agent` (`Session::agent_preview/agent_apply` são **só Rust**, nunca em `call`/`begin`); `operation_id` de IA é derivado (tarefa+passo+índice), nunca do modelo; lista **fechada** de comandos e de tools — **sem** shell/filesystem/HTTP genérico/configurações/segredos como tool. Segredos: `SecretString`, cofre do SO, redator central (erros, auditoria, diagnóstico, pânico); a UI envia a chave **uma vez** e só recebe `credential_configured`; referência/host da credencial são decididos no servidor. Fallback de modelo **só** pelo configurado (ADR-080). Conteúdo de documento/transcrição/OCR entra no prompt como `untrusted_data`; o Interpreter não recebe tools. Resultados de IA são derivados (`ai_records`/`ai_usage`, schema 4; fora do documento e do undo). Só o `capia-ai` faz rede; só o `capia-intelligence` orquestra; a UI fala com `ai.*` **apenas** por `store/aiController.ts`. Roteiros Replay (`CAPIA_AI_REPLAY_SCRIPT`) existem só no devserver de E2E. **Fase 4 em `ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`**: não marque avaliação com LLM real, corpus real anotado ou smoke com chaves reais sem resultados reais. **Fase 5 não iniciada** (sem AI Run autônomo, variantes, memória autônoma, Asset Gateway completo).
- Não pule fases do `docs/ROADMAP.md`. Uma missão por vez; atualize `docs/STATUS.md` ao final.
- Mudou uma decisão arquitetural? Registre um ADR novo em `docs/DECISIONS.md` (não reescreva o antigo; marque como *Superseded*).
- Respeite as regras de dependência entre crates (`docs/ARCHITECTURE.md` §4). UI, Timeline, Render, AI, Providers, Assets e Integrations não se acoplam diretamente.
- Testes de propriedade para tempo e Command Engine são obrigatórios (`docs/TEST_STRATEGY.md`).
- Nunca commitar chaves, `.env`, projetos de usuário ou mídia.
- **Todo código/ativo de terceiros segue `docs/PROVENANCE.md`** (registro, cabeçalho, licença permitida; `REFERENCE_ONLY`/`DO_NOT_USE` nunca são copiados).
- **Escritas na timeline:** todo comando tem `operation_id`; atores `Agent`/`Api` só escrevem por `preview → apply_plan` (ADR-029/030).
- **FFmpeg:** build própria LGPL, dinâmica, sem GPL/nonfree (ADR-032). Timing é do engine, não do ffmpeg (ADR-035).
- Idioma da documentação: português; identificadores de código: inglês.
