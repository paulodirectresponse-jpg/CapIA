# ARCHITECTURE — Visão geral do sistema

> Documento-mestre. Detalhes de cada subsistema estão nos documentos específicos; este documento define **fronteiras, contratos e regras de dependência**.

## 1. Forma geral

```
┌──────────────────────────── Desktop App (processo único + sidecars) ─────────────────────────────┐
│                                                                                                   │
│  WebView2 (React + TS)                       Rust Core (headless, independente de Tauri)          │
│  ┌──────────────────────────┐   IPC tipado   ┌──────────────────────────────────────────────────┐ │
│  │ Shell UI / painéis       │◄──────────────►│ Engine API (fachada única)                       │ │
│  │ Timeline (canvas)        │  comandos ↓     │   ├─ Command Engine ── Document (snapshots)     │ │
│  │ Inspector / Library      │  patches  ↑     │   ├─ Query Service                              │ │
│  │ Read-replica do documento│                 │   ├─ Store (SQLite, autosave, recovery)         │ │
│  │ (+ core WASM p/ ghost)   │                 │   ├─ Assets + Asset Gateway                     │ │
│  └──────────────────────────┘                 │   ├─ Media (FFmpeg/libav, frame index)          │ │
│                ▲                              │   ├─ Render Graph → Compositor (wgpu) → Encoder │ │
│                │ frames                       │   ├─ Preview Engine → Presenter ────────────────┼─┼─► superfície nativa
│                └──────────────────────────────┤   ├─ Job System                                 │ │
│                                               │   ├─ AI (Orchestrator, Router, Providers, Tools)│ │
│                                               │   └─ Secrets (Credential Manager)               │ │
│                                               └──────────────────────────────────────────────────┘ │
│  Sidecars (processos filhos, opcionais): ffmpeg CLI p/ jobs batch, adapters do Asset Gateway,      │
│  whisper.cpp local, etc.                                                                          │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
Futuro (Fase 6): capia-server (REST / MCP / Webhooks) usa a MESMA Engine API, sem Tauri.
```

Conceito central:

```
USER (UI) ─────┐
CLI / API / MCP├──► ENGINE API ──► COMMAND ENGINE ──► DOCUMENT (timeline) ──► Preview / Render / Store
AI BRAIN ──────┘        (tools)        (validação, transações, undo)
```

## 2. Decisão de stack (resumo — justificativa completa em `DECISIONS.md` ADR-001..005)

A stack candidata **Tauri + React + TypeScript + Rust + SQLite + FFmpeg** é **aceita com cinco condições obrigatórias**:

| # | Condição | Por quê |
|---|---|---|
| C1 | **Rust core headless** (workspace de crates sem dependência de Tauri) | REST/MCP/CLI/testes usam o mesmo motor; Tauri vira só um shell substituível |
| C2 | **Preview e export renderizados em Rust** (FFmpeg/libav + wgpu), **não** em WebGL/WebCodecs na WebView | Paridade preview/export, cobertura de codecs, VFR, aceleração HW, portabilidade (WebKit no macOS/Linux não tem as mesmas APIs) |
| C3 | **Timeline desenhada em canvas** (não DOM por clip) com virtualização | Milhares de clips a 60 fps; DOM não escala |
| C4 | **Toda IA e todo acesso a credenciais em Rust** | Chaves nunca tocam JS/WebView; orquestração disponível headless |
| C5 | **Modelo/Comandos compiláveis para WASM** (crates sem IO) | UI calcula "ghost previews" de arrastos com a mesma lógica do core, sem divergência |

Alternativas avaliadas e rejeitadas: Electron (sem ganho real — o trabalho pesado é nativo de qualquer forma; mais memória), Qt/QML (bom para vídeo, mas iteração de UI mais lenta, C++/licenciamento), UI 100% Rust (egui/iced/slint: imaturas para UI profissional densa), WebCodecs+WebGPU como engine de preview (ver `PREVIEW_RENDER.md` §2).

**Risco principal da stack:** apresentar frames nativos dentro de uma janela Tauri/WebView2 (o "problema de airspace"). Mitigação: abstração `PreviewPresenter` com duas implementações candidatas, validadas em spike na Fase 1 (ver `PREVIEW_RENDER.md` §5 e `DECISIONS.md` OD-1).

## 3. Estrutura do repositório (alvo — criada a partir da Fase 1/2)

> **Estado real (M05, ADR-040/041):** `capia-time`, `capia-model` e `capia-commands` estão implementados (tempo, documento, ops primitivas, Command Engine com idempotência e plano por token, suíte de aceitação, paridade nativo × WASM); `serde` (e `serde_json` em `capia-commands`) é a única dependência externa do núcleo. Persistência/assets/mídia/render ainda não existem.
>
> **Estado real (M04, ADR-038):** existem hoje `crates/{capia-time,capia-model,capia-commands,capia-project}`, `apps/desktop` (Vite + Tauri; crate `capia-desktop`), `packages/{engine-bindings,editor-ui}`, `tests/acceptance`, `tools/`, `spikes/`. `capia-project` cumpre inicialmente o papel de `capia-store` + `capia-engine`; `engine-bindings` é o `engine-client`. Os demais itens abaixo nascem quando sua fase começa.

```
/crates                      Rust workspace
  capia-time                 Ticks, Rational, FrameRate, TimeRange, conversões (no_std-friendly, WASM)
  capia-model                Entidades do documento, IDs, invariantes (sem IO, WASM)
  capia-commands             Command Engine: comandos, primitive ops, transações, histórico (sem IO, WASM)
  capia-store                SQLite, formato .capia, migrations, autosave, snapshots, recovery
  capia-jobs                 Executor genérico de jobs (prioridades por créditos, cancelamento real, dedup) — M08
  capia-media                Probe, demux/decode (libav), frame index, VFR, HW decode, thumbnails, waveforms
  capia-assets               Bibliotecas, fingerprint, dedup, relink, offline, versões, proveniência
  capia-gateway              Asset Gateway: trait de adapter, registro, sandbox de sidecars
  capia-render               Render Graph, compiler, compositor wgpu, text engine, audio mixer, encoder
  capia-preview              Playback scheduler, frame cache, PreviewPresenter
  capia-jobs                 Job system persistente
  capia-secrets              SecretStore (Windows Credential Manager / Keychain / Secret Service)
  capia-ai                   Providers, Model Registry, Brain Profile, Capability Router, Tools, Orchestrator, Agents, Memory
  capia-engine               Engine API (fachada): sessões de projeto, actor do documento, eventos
  capia-cli                  CLI headless (testes, scripts, automação) — Fase 2
  capia-desktop              Shell Tauri: janela, IPC adapter, presenter nativo
  capia-server               REST / MCP / Webhooks — Fase 6
/apps/desktop-ui             React + TS (shell, painéis)
/packages
  ui-timeline                Timeline em canvas (render, hit-test, gestos)
  design-system              Tokens, componentes próprios
  engine-client              Cliente IPC tipado (tipos gerados do Rust via specta/ts-rs)
  core-wasm                  Build WASM de capia-time/model/commands
/docs                        Esta documentação
/spikes                      Experimentos descartáveis da Fase 1 (relatórios em docs/spikes/)
/testdata                    Corpus de mídia de teste (via Git LFS ou download script, nunca mídia de cliente)
```

## 4. Regras de dependência (obrigatórias)

```
capia-time ◄── capia-model ◄── capia-commands
                    ▲                ▲
capia-media ◄── capia-assets         │
     ▲              ▲                │
capia-render ◄── capia-preview       │
     ▲              ▲                │
     └──── capia-engine ─────────────┘──► capia-store, capia-jobs, capia-gateway, capia-secrets
                    ▲
                capia-ai  (depende SOMENTE de capia-engine (API pública) + capia-secrets + capia-model (tipos))
                    ▲
     capia-desktop / capia-cli / capia-server (adapters de transporte)
```

Regras:

1. `capia-time`, `capia-model`, `capia-commands` **não fazem IO** e compilam para WASM.
2. `capia-render` não conhece IA, UI, providers nem Tauri. Recebe um snapshot do documento + resolvedor de mídia.
3. `capia-ai` **não** importa `capia-store`, `capia-render` ou `capia-model` internals mutáveis: só fala com a Engine API (via tools) — é um cliente como a UI.
4. Providers de IA e adapters do Asset Gateway são plugins atrás de traits; nenhum outro crate conhece um provider concreto.
5. A UI não contém regra de negócio de edição além da usada para ghost preview (que vem do WASM do core).
6. Transportes (Tauri IPC, CLI, HTTP, MCP) são adapters finos sobre a Engine API; nunca implementam lógica.

## 5. Engine API (contrato central)

Fachada única, assíncrona, serializável (todos os tipos com schema JSON gerado). Grupos:

| Grupo | Exemplos | Doc |
|---|---|---|
| `project.*` | open, create, close, save_snapshot, list_snapshots, restore_snapshot, package | DATA_MODEL |
| `command.*` | execute(tx) (User/System), preview(tx) → plan_token, apply_plan(plan_token) (obrigatório para Agent/Api), begin_tx, apply_in_tx, validate_tx, commit_tx, rollback_tx, undo, redo — todo comando carrega `operation_id` | COMMAND_SYSTEM §4 |
| `query.*` | get_document(rev), get_sequence, find_clips(range/filter), timeline_digest, history | COMMAND_SYSTEM |
| `assets.*` | import, search, relink, get_representations, generate_version | ASSET_SYSTEM |
| `gateway.*` | fetch(url), search(query), fetch_comments(url) → jobs | ASSET_SYSTEM |
| `jobs.*` | list, subscribe, cancel, retry | §8 |
| `render.*` | export(deliverable), render_frame(seq, t), render_range_preview | PREVIEW_RENDER |
| `preview.*` | attach_presenter, play, pause, seek, set_quality | PREVIEW_RENDER |
| `ai.*` | start_run, run_status, approve_plan, cancel_run, provider/model/profile CRUD | AI_SYSTEM / AI_PROVIDERS |
| `events` | stream: DocumentChanged{rev, patches}, JobProgress, RunProgress, MediaStatus | — |

Toda chamada carrega um **`Actor`** (`User`, `Agent{run_id, role}`, `Api{client_id}`, `System`) usado para permissões, histórico semântico e auditoria.

## 6. Modelo de concorrência

- **Um actor por projeto aberto** possui o documento. Todas as mutações são serializadas na fila desse actor (single-writer).
- O documento é **imutável com compartilhamento estrutural** (ex.: crates `im`/`rpds`); cada commit produz `Arc<DocumentSnapshot>` com `revision: u64`. Leitores (preview, render, AI, UI sync) seguram snapshots sem bloquear o writer.
- Preview e export renderizam um snapshot fixo; export longo nunca vê estado "meio editado".
- Trabalho pesado (decode, render, análise, IA) roda fora do actor: thread pools / tokio runtime / GPU queue.

## 7. Sincronização UI ↔ Core

1. UI abre projeto → recebe snapshot completo serializado (uma vez).
2. Cada commit emite `DocumentChanged { revision, patches[] }`; a UI aplica patches na sua read-replica.
3. Gestos contínuos (arrastar, trim, slider) são calculados localmente pelo WASM do core sobre a réplica ("ghost"); no *drop*, a UI envia **um** comando. Sliders usam `gesture_id` para coalescer em uma entrada de undo.
4. Se o core rejeitar o comando, a UI descarta o ghost e mostra o erro.
5. Payloads grandes (thumbnails, waveforms) **não** passam pelo IPC JSON: são servidos por protocolo customizado (`capia://`) com cache HTTP-like.

## 8. Job System

Detalhes de uso por subsistema estão nos docs específicos; o contrato é único:

```rust
struct Job { id, kind: JobKind, project_id: Option<ProjectId>, state: JobState, progress: Progress,
             priority, attempts: u32, max_attempts: u32, idempotency_key: String,
             depends_on: Vec<JobId>, payload: Json, result: Option<Json>, error: Option<JobError>,
             created_at, started_at, finished_at, cancel_requested: bool }
enum JobState { Queued, Running, RetryWait{until}, Succeeded, Failed, Cancelled, Interrupted }
enum JobKind  { Probe, Fingerprint, Proxy, Thumbnails, Waveform, Transcription, MediaAnalysis,
                ReferenceAnalysis, Download, Generation, Render, Package }
```

- **Persistência:** tabela `jobs` (projeto ou app.db conforme escopo). Ao reiniciar: `Running` → `Queued` se idempotente, senão `Interrupted` (usuário decide).
- **Pools por recurso** com limites: `cpu`, `gpu_decode`, `gpu_render`, `network`, `ai` (por provider, respeitando rate limit), `io`.
- **Cancelamento** cooperativo via token; processos filhos ficam num Windows Job Object (matar a árvore toda).
- **Retry** só para erros classificados como transitórios (rede, 429, 5xx), backoff exponencial com jitter; erros de validação/permissão não fazem retry.
- **Dependências** (DAG): `Download → Fingerprint → Proxy → Transcription`.
- **Progresso** throttled (≤ 10 eventos/s por job) via stream de eventos.
- Jobs nunca mutam o documento diretamente: ao concluir, emitem resultado; se precisam alterar a timeline (raro), submetem um comando com `Actor::System`.

## 9. Local-first e fronteira com a nuvem

| Local (sempre) | Nuvem (somente quando configurado) |
|---|---|
| Projeto, timeline, histórico, assets, proxies, cache, preview, metadados, render final | Inferência LLM, visão, transcrição cloud, geração de imagem/vídeo, serviços externos do Gateway |

Regras de minimização: para transcrição envia-se **áudio extraído e comprimido** (ex.: Opus mono 16 kHz), não o vídeo; para visão, **frames amostrados** em resolução reduzida; nunca upload de mídia original sem ação explícita.

## 10. Observabilidade

- Logging estruturado (`tracing`) com **camada de redação de segredos obrigatória** (`SECURITY.md` §4).
- Logs locais rotativos; nenhum envio de telemetria sem opt-in.
- Crash reporting (Fase 6) com opt-in e redação.
- Cada AI Run registra chamadas a modelos (tokens, custo, latência) e tool calls — sem chaves, com prompts armazenados localmente.

## 11. Mapa de documentos por subsistema

Time/tracks/clips/nested → `TIMELINE_ENGINE.md` · UX → `TIMELINE_UX.md` · Comandos → `COMMAND_SYSTEM.md` · Entidades/persistência → `DATA_MODEL.md` · Preview/render → `PREVIEW_RENDER.md` · IA → `AI_SYSTEM.md`, `AI_PROVIDERS.md` · Assets → `ASSET_SYSTEM.md` · Segurança → `SECURITY.md`.

## Persistência, facade e CLI (M06)

`capia-store` (SQLite, journal) → `capia-project` (facade: `Project`, `parse_transaction`) → `capia-cli`. Matriz de dependências em `tools/check-architecture.mjs`.

## Jobs e pipeline de mídia (M08)

`capia-jobs` (genérico: nada de SQLite/FFmpeg/documento) é executor bounded com prioridades por créditos, cancelamento cooperativo (que **mata** o FFmpeg) e deduplicação (ADR-052). `capia-media` ganha índice de quadros `CIDX`, decode exato, PCM f32, waveform `CWFM` e proxy (ADR-054..056); `capia-assets` ganha impressão rápida, hash de job, cache v2 com produção atômica e relink em lote (ADR-053/057/058); `capia-store` ganha o schema 3 (`jobs`, `import_tickets`) e o `JobStore` (conexão própria; é o `JobSink` do executor); `capia-project` orquestra: `start_pipeline`, `import_asset_async` (ticket), `pump` (a thread do projeto grava), `submit_*`, `force_relink_asset`, `batch_relink_folder`, `cache_*`. Dependências novas: `capia-time → capia-media` (já existia), `capia-jobs` solto (sem dependências internas) → `{capia-store, capia-project}`. O núcleo puro segue sem IO: `capia-jobs`, FFmpeg e sistema de arquivos **não** vão para WASM (`cargo check --target wasm32-unknown-unknown` só de `time/model/commands`).

**Regra de escrita:** workers só calculam e escrevem **cache**; documento e catálogo só mudam na thread do projeto (`pump`), pelo engine/catálogo. Um único processo é dono do executor de um projeto (lock do SO).

## Mídia e assets (M07)

`capia-media` (probe/ffprobe atrás de `MediaProbe`, processos limitados) e `capia-assets` (identidade por conteúdo, hash em streaming, import, verify, relink, cache) ficam **abaixo** de `capia-store`, que persiste o catálogo (schema 2); `capia-project` orquestra o import atômico (catálogo + `register_asset` na mesma transação) e a CLI traduz argumentos. Dependências: `capia-time → capia-media`; `{capia-model, capia-media} → capia-assets`; `{capia-commands, capia-assets} → capia-store`. O núcleo puro (`time/model/commands`) segue sem IO e sem FFmpeg (ADR-046..049; matriz em `tools/check-architecture.mjs`).

## Render, decode, preview e export (Fase 2 — conclusão)

```
capia-time → capia-model → capia-commands            (núcleo puro, WASM)
capia-time/model → capia-render      (grafo + compositor CPU + mixer; puro, WASM; ADR-063..065)
capia-time/model/render → capia-preview   (scheduler headless + FrameSink + Clock; ADR-066)
capia-time → capia-media             (probe, índices CIDX/CAIX, decode, FrameStream, encoders, EncodeSession)
{time, media} → capia-decode         (DecodeService + ByteLru + PcmCache; ADR-059..062)
{commands, assets, jobs, decode, render} → capia-store → capia-project → {capia-cli, apps/desktop}
```

`capia-render` recebe a mídia por um trait (`MediaSource`) e nunca faz IO. `capia-project` fornece o adaptador (`ProjectSource`): índice de quadros e de áudio vindos do cache derivado, decode pelo `DecodeService` e PCM pelo `PcmCache`, **sempre do arquivo original** (o proxy nunca é fonte de decode nem de export). `Project::render_frame/render_range/render_audio_range` e `export_intermediate/export_mp4` são a fachada que a CLI usa. `capia-preview` não conhece projeto: opera sobre `Arc<RenderGraph>` + `Arc<dyn MediaSource>` e é testado contra o export. Matriz em `tools/check-architecture.mjs` (novos crates: `capia-render`, `capia-decode`, `capia-preview`).

## Editor (Fase 3)

**Pacotes JS** (matriz verificada por `pnpm check:arch`): `engine-bindings` (contrato tipado, read-model por patches) · `ui-kit` (design system) · `ui-timeline` (canvas + ponte WASM) · `editor-ui` (store/controlador, painéis, i18n, keymap) · `apps/desktop` (adaptadores: Tauri IPC, diálogos, SharedBuffer) · `e2e` (Playwright, sem importar código do produto).
**Crates novas:** `capia-editor-api` (fachada JSON `Session::call/begin`) · `capia-devserver` (HTTP local só dev/E2E) · `capia-timeline-wasm` (snap/grupo/colocação do core em WASM; ADR-070) · `capia-webview-surface` (SharedBuffer do WebView2; `unsafe` isolado; ADR-074).

```
UI (React) ──▶ EditorController (única porta de escrita da UI)
                 │  execute / undo / redo  ──▶ EditorClient ──▶ transporte ─┬─ Tauri IPC (editor_call / editor_call_binary)
                 │  réplica imutável ◀── patches (ChangeSet)               └─ HTTP 127.0.0.1 (devserver: dev/E2E)
                 │                                                              │
                 └─ ghost/snap: ui-timeline ──▶ WASM (capia-timeline-wasm)     capia-editor-api::Session ──▶ capia-project ──▶ Command Engine
Preview: render.frame ─ preparado sob o lock, renderizado fora (Job) ─▶ SharedBuffer (WebView2) | bytes por IPC ─▶ WebGL canvas
Áudio: render.audio (mixer do export) ─▶ WebAudio; relógio de áudio mestre a 1×
```
Regras: a UI nunca escreve fora do Command Engine (teste de fronteira); preferências de UI (`localStorage`, validadas campo a campo) são separadas do projeto; eventos do engine (`events.poll`) e inscrições são liberados no `dispose`; sem IPC por quadro de interação (arrasto = WASM local, IPC só no drop); outros clientes são notificados por `revision_changed` (ADR-072). Decisões: ADR-070..077.

## Fase 4 — camada de IA (implementada)

`capia-secrets` (folha) → `capia-ai` (providers, registry, router, dispatcher, tools; único HTTP de saída) → `capia-intelligence` (pipelines + assistente + serviço `ai.*`; cliente do Engine API) → hospedado pelos composition roots `capia-devserver`/`capia-desktop`. A matriz está em `tools/check-architecture.mjs`; o núcleo puro (time/model/commands) continua sem rede/IA. Decisões: ADR-078..086. A UI só fala `ai.*` por `store/aiController.ts`.

## Fase 5 — autonomia (implementada; ADR-087..099)

Sem crate novo: a autonomia é o módulo `autonomy/` **dentro de `capia-intelligence`** (cliente do Engine API; também usa `capia-store` para `AutonomyStore`/`AppDb`). Mapa:

| Onde | Módulo | Papel |
|---|---|---|
| `capia-intelligence/src/autonomy/` | `machine`, `model`, `orchestrator`, `stages`, `plan`, `roles`, `critic`, `memory`, `gateway`, `generation`, `failpoint` | máquina de estados e cursor, Run/política/orçamento, handlers de stage, compilador `EditPlan → comandos`, papéis de LLM sem tools, Critic, memória de 4 escopos, Asset Gateway, geração, failpoints (feature, só testes) |
| `capia-intelligence/src/service/` | `autonomy_api.rs`, `demo.rs` | `ai.run.*`/`ai.memory.*`/`ai.gateway.*`/`ai.generation.*`; "cérebro" Replay só com feature `testkit` |
| `capia-store/src/` | `autonomy.rs`, `schema.rs` (`m005_autonomy`) | `AutonomyStore`: Runs com CAS, stages, eventos, efeitos, proveniência, memória de projeto, ledger |
| `capia-ai/src/` | `fetch.rs` (`SafeFetcher`) | **único** código de rede fora dos providers de IA, usado pelos adapters do Gateway |
| `capia-commands/src/engine.rs` | `selective_undo_report/selective_undo` | undo seletivo por entradas/ator (nova entrada) |
| `capia-editor-api/src/lib.rs` | `history.undo_report/undo_selective`; `agent_import_*` (só Rust) | undo seletivo na Engine API; import de asset do Gateway |
| `packages/editor-ui`, `packages/engine-bindings` | `AiRunsPanel`, `aiController` (runs/memory/gateway), `controller.undoRun`, `ai.ts` | UI Runs/Memory/Sources; só via controladores |

Regras de dependência (inalteradas, verificadas por `pnpm check:arch`): `capia-secrets → capia-ai (HTTP: providers + SafeFetcher) → capia-intelligence (orquestra) → composition roots`; `capia-ai` **não** conhece projeto/documento/Gateway; o `capia-intelligence` não faz rede por conta própria; `unsafe` continua só em `capia-webview-surface`/`capia-timeline-wasm`. A feature `failpoints` nunca é habilitada no build de produto.
