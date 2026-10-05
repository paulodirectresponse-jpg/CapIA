# ROADMAP

```
FASE 1 — Fundação
FASE 2 — Motor
FASE 3 — Editor
FASE 4 — Inteligência
FASE 5 — Autonomia
FASE 6 — Integração e Finalização
```

Cada fase só começa quando os critérios de conclusão da anterior forem atendidos (registrados em `STATUS.md`). Fases são decompostas em **missões**; uma sessão = uma missão.

---

## FASE 1 — Fundação

**Objetivo:** arquitetura documentada, decisões bloqueantes fechadas, riscos técnicos maiores validados por spikes, repositório pronto para código.

Missões (numeração real das sessões):
1. ✅ **M01 — Arquitetura e documentação.**
2. ✅ **M02 — Auditoria de bases open-source e estratégia de reuso** (`OPEN_SOURCE_AUDIT.md`; estratégia C aprovada).
3. ✅ **M03 — Fechamento da fundação e spikes de risco** (`docs/spikes/`; ADR-029..036). Resultado: S2, S3, S4, S5, S6 e S7 **medidos/concluídos**; **S1 não mensurável no ambiente de nuvem** (sem Windows/GPU).
4. ✅ **M04 — Scaffold, CI e preparação** (workspace Rust+pnpm, shell Tauri, CI, verificação de licenças e de fronteiras, pacote S1 para Windows; ADR-037/038). Pendências remanescentes:
   - **Primeira execução do CI em runner real** (workflow escrito e validado com `actionlint`, ainda não executado: depende do push);
   - ✅ **S1 medido** (runner Windows; `tools/s1-preview-spike`) → **OD-1 fechada: P2** (ADR-069); residual: CPU/pacing em GPU real.

Spikes da Fase 1 (relatórios em `docs/spikes/`): S1 preview surface · S2 decode frame-exato/VFR · S3 FFmpeg LGPL · S4 core em WASM · S5 timeline em canvas · S6 compositor OpenCut (`REIMPLEMENT_WITH_REFERENCE`) · S7 suíte de aceitação de timeline.

**Critérios de conclusão:**
- [x] Decisões bloqueantes da **Fase 2** fechadas (OD-2, OD-3). **OD-1** é gate da **Fase 3** (ADR-037); **OUTPUT-H264** é gate de saída da Fase 3.
- [x] Relatórios dos spikes S2–S7; pacote do S1 pronto para execução em Windows (S1 medido = gate da Fase 3).
- [x] CI verde em Windows para o workspace (build, lint, testes, secret scan) — *verde na Fase 2 e novamente na Fase 3 (run 72, commit `72a0600`).*
- [x] Workspace compila, testa, passa lint/format; fronteiras e licenças verificadas por ferramenta.
- [x] Nenhum ADR "Proposed" bloqueando a Fase 2 (ADR-016 aceito na M03).

---

## FASE 2 — Motor (headless, sem UI de produto)

**Objetivo:** todo o núcleo funcionando e testado via CLI/testes, sem interface.

**Gate de entrada:** nenhum bloqueio por OD-1 — a Fase 2 **pode iniciar com OD-1 aberto** (ADR-037). **Fora do escopo da Fase 2:** preview embutido na janela e UI de editor (só o trait `PreviewPresenter` e um *frame sink* headless). O export H.264 da Fase 2 usa encoders atrás de abstração (ADR-032); a decisão final do caminho de produção é `OUTPUT-H264`.

**Estado da Fase 2: CONCLUÍDA** (branch `claude/phase2-completion`; CI Linux + Windows + TypeScript + políticas verde no mesmo commit — ver `docs/STATUS.md`). **Fase 3 permanece BLOQUEADA por OD-1** (S1 não foi medido: exige Windows 11 + GPU reais). `OUTPUT-H264`: capacidade de engenharia **provada** (`h264_mf` no CI Windows), decisão de produção/jurídica **pendente**.

**Escopo entregue vs. escopo original desta seção:** render graph, compositor **CPU de referência** determinístico (RGBA8, ADR-063..065), mixer, decode persistente + caches, índice/seek de áudio, preview headless, export intermediário e MP4 por `EncoderCapability`. **Não** fazem parte da Fase 2 (decisão do PO na missão de conclusão): compositor wgpu/RGBA16F (Fase 3; a referência CPU é o oráculo dele), texto, transições, efeitos, máscaras, rotação arbitrária. `capia-engine` segue sendo o papel de `capia-project`.

**Progresso da Fase 2 (M08; histórico — as pendências citadas aqui foram fechadas na conclusão):** além da M07, existem `capia-jobs` (executor com prioridades sem starvation, cancelamento real, dedup), import **não bloqueante** (`ImportTicket`), impressão rápida × SHA-256, **índice de quadros** (CFR/VFR/B-frames/GOP longo) e **decode exato** (RGBA8), áudio PCM f32 com intervalos exatos em amostras, waveform multirresolução, **proxy** (MJPEG LGPL; H.264 só por hardware), cache derivado com produção atômica/lock/GC, *force relink* (`update_asset`) e relink em lote por pasta, jobs e tickets persistidos (schema 3) com recuperação (`interrupted`), crash tests reais com kill durante índice/waveform/proxy/hash e CLI `job`/`cache`/`media index|frame|waveform|proxy`. Falta: `capia-render` (compositor), preview headless, export, decode de áudio com seek sem custo O(início), índice de pacotes de áudio.

**Progresso da Fase 2 (M07; histórico):** `capia-media` (probe/ffprobe normalizado, processos limitados), `capia-assets` (identidade por conteúdo, import, verify, relink, cache) e o catálogo de mídia no `.capia` (schema 2) existem; `capia-cli` ganhou `asset …`/`media probe`; os cinco comandos de composição de nested (ADR-050) estão prontos. Falta: frame index/decode, jobs, render, preview headless, export.

Escopo: `capia-time`, `capia-model`, `capia-commands` (undo/redo, transações, `preview`/`apply_plan` com plan token, `operation_id`/idempotência, refs simbólicas, conflitos), `capia-store` (formato `.capia`, autosave, snapshots, backups, migrations, recovery), `capia-assets` (import, fingerprint, dedup, relink, offline, versões), `capia-media` (probe, frame index, decode, thumbnails, waveforms, proxies), `capia-jobs`, `capia-render` (render graph, compositor wgpu **próprio** — ADR-034, RGBA16F linear —, texto, transições básicas, mixer, export H.264 via encoders atrás de abstração — ADR-032), `capia-preview` (scheduler + presenter escolhido, demo mínima), `capia-engine`, `capia-cli`.

**Critérios de conclusão:**
- [x] Script via `capia-cli` cria projeto com 3 sequences (2 hooks + `BODY_MASTER` nested), transações, undo/redo, e exporta MP4 corretos (duração/fps/sync verificados por probe). *(`capia-cli/tests/phase2_e2e.rs`, binário real: importa mídia real, `HOOK_A`/`HOOK_B`/`BODY_MASTER`, undo/redo, render, export e ffprobe. Windows CI: **H.264 via `h264_mf`**; Linux (sem encoder aprovado): H.264 falha de forma estruturada e o fluxo é provado com `mpeg4-reference`, pedido explícito.)*
- [x] Testes de propriedade do Command Engine (≥ 10.000 sequências aleatórias) sem violação de invariantes; `apply∘undo = id` *(M05: `crates/capia-commands/tests/properties.rs`, 10.000 sequências × 8 comandos por execução)*.
- [x] **Suíte de aceitação `tests/acceptance/timeline` (120 cenários: 108 + 12 da M05, ADR-036/039) 100% verde em `capia-commands`**, com exatidão em Ticks e atomicidade nos erros *(M05; mutação detecta regressões)*.
- [x] **Idempotência e plan token (ADR-029/030):** testes de replay, de kill no meio do commit e de rejeição de token adulterado/expirado/consumido/de outro ator. *(M05: replay, conflitos de id, token adulterado/expirado/outro ator/consumido/drift ✅; **kill no meio do commit** ✅ M06: `capia-store/tests/crash.rs`, processo real morto em 5 estágios.)*
- [x] **Paridade nativo × WASM** por hash sobre o modelo real, em CI (ADR-016) *(M05: `tools/check-wasm-parity.mjs`, 150 sequências por digest, nativo × `wasm32-wasip1`)*.
- [x] Teste de paridade preview×export bit-idêntico (sem proxy) no corpus de teste; golden frames. *(`capia-project/tests/parity.rs`: digests SHA-256 iguais quadro a quadro tocando e em scrub, após reabrir, cache frio/quente/minúsculo e com proxy presente; goldens de vídeo/áudio no compositor CPU de referência. **Desvio:** não há GPU de software porque não há compositor GPU na Fase 2 — os goldens da Fase 3 (wgpu em WARP/llvmpipe) serão comparados com esta referência.)*
- [x] Conformidade de mídia (ADR-035): seek por índice 100% exato e conformação de cadência = `frame_at` em corpus CFR/VFR sintético. *(M08: fixtures com quadros identificáveis — CFR com GOP longo e B-frames, VFR com buracos de timestamp, mkv `ffv1` com offset de 3 s — decodificadas por **pixels**; `frame_at_or_before` usa PTS reais, nunca `N/fps`; `capia-media/tests/pipeline.rs`.)*
- [x] Corpus VFR/29,97/23,976/59,94/44,1 kHz: drift A/V ≤ 1 frame em 10 min. *(`capia-project/tests/av_drift.rs`: flash×beep medidos no MP4 exportado em 23,976/29,97/59,94/25/30 fps × 44,1/48 kHz; VFR por tempo; timeline de 600,6 s (150 clips) com áudio exato por amostra e quadros certos nas fronteiras até o penúltimo clip, ~593 s.)*
- [x] Kill -9 durante transações e jobs: projeto reabre íntegro em 100% dos testes de crash. *(M06: transações ✅ 10 combinações + 14 rodadas aleatórias; M08: jobs ✅ — filho morto **dentro** do índice, do waveform, da codificação do proxy e com import pendente: projeto válido, job `interrupted` (nunca `completed`), nenhum derivado parcial publicado, retry funciona; `capia-project/tests/crash_media.rs`.)*
- [x] Benchmarks: transação de 500 ops < 200 ms em projeto de 10.000 clips; abrir projeto de 10.000 clips < 2 s. *(M05, release, só o modelo em memória: tx de 500 ops ≈ 16 ms; carregar+indexar+validar 10.000 clips ≈ 17 ms; M06, release, com SQLite e `synchronous=FULL`: abrir 10.000 clips ≈ 81 ms; commit comum ≈ 7 ms.)*
- [x] Migration de fixture v1→v2 sintética testada. *(M06: `capia-store/tests/schema.rs`; M07: migration **real** v1→v2 (catálogo de mídia) sobre projeto schema 1 de verdade, com backup e cadeia v1→v2→v3 sintética.)*

---

## FASE 3 — Editor (manual, sem IA)

**Objetivo:** editor utilizável de ponta a ponta sem IA, com fluidez próxima ao CapCut Desktop.

**Gates de entrada (hard):** **OD-1 fechado ✅** — S1 executado em Windows (`tools/s1-preview-spike`), relatório em `docs/spikes/S1-preview-surface.md` §6, presenter decidido pela ADR-069 (**P2**) segundo a regra de §4. Residual: validar CPU/pacing do P2 em GPU real na primeira entrega de preview (gatilho de reabertura na ADR-069).
**Gate de saída:** `OUTPUT-H264` — caminho confiável de exportação MP4/H.264 no Windows (hardware quando disponível; Media Foundation/FFmpeg quando adequado; fallback por software legal e de qualidade aceitável; sem x264/x265 GPL) definido e validado **antes da entrega do Editor**.

Escopo: shell UI + design system, `ui-timeline` (canvas), painel Project (árvore) + abas de sequence + `+`, biblioteca (projeto/global), drag-and-drop, trim/split/snapping/zoom/ripple/grupos/copy-paste, tracks (lock/mute/solo/hide/magnetic), nested (abrir, make unique, flatten, follow length), inspector, keyframes, texto, legendas manuais + estilos, transições, áudio (volume/fades/detach), preview com proxies, export de deliverables em lote, histórico visível, relink UI, atalhos configuráveis, pt-BR/en.

**Estado da Fase 3: `ENGINEERING COMPLETE — HUMAN ACCEPTANCE PENDING`** (branch `claude/phase3-editor`; ver `docs/STATUS.md`).

**Critérios de conclusão:**
- [ ] Editor experiente produz um UGC ad de 30–45 s (talking head + B-roll + texto + legendas + música + SFX) em ≤ 15 min, sem bugs bloqueantes (teste com ≥ 3 usuários). **PENDENTE — exige pessoas reais; pacote pronto em `tools/phase3-acceptance/` (nenhum resultado foi fabricado).**
- [ ] Metas de `TIMELINE_UX.md` §6 atendidas em hardware de referência (definido em OD-3). **Parcial:** medidas em CI/sandbox sem GPU (pintura, arrasto, scrub ✅; commit/undo ✅ no engine, na margem pela UI); benchmark estrito em hardware de referência pendente (`CAPIA_PERF_STRICT=1`).
- [x] Todas as interações da UI produzem comandos (verificado: nenhuma escrita fora do Command Engine). *(`packages/editor-ui/src/architecture.test.ts` varre o fonte; `controller.test.ts` confere que as ações só chamam `execute/undo/redo`.)*
- [x] Testes E2E dos fluxos principais verdes no CI Windows. *(job `e2e-windows`: app Tauri real por WebView2/CDP, 17 fluxos + edição + visual, H.264 real via `h264_mf`, preview por SharedBuffer — ver `docs/STATUS.md` para o commit verde.)*
- [ ] Residual ADR-069 em GPU real (CPU/pacing do P2 a 720p). **PENDENTE — hardware:** `tools/phase3-acceptance/gpu-residual.ps1`.
- [x] `OUTPUT-H264` — **engenharia** integrada (UI + export atômico + ffprobe + CI Windows); a decisão **jurídica/de produto** (patentes H.264/AAC, OpenH264, qualidade de produção) **continua pendente e não é declarada resolvida** (ADR-075).

---

## FASE 4 — Inteligência (IA assistida, um passo por vez)

**Estado da Fase 4: `ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`** (branch `claude/phase4-intelligence`; ver `docs/STATUS.md`; CI: run 83, commit `02b8490`, 6/6 jobs verdes). Fase 5 **não** iniciada.

**Objetivo:** camada de IA configurável e segura; IA executa tarefas pontuais como transações.

Escopo: `capia-secrets` (Credential Manager), Provider abstraction (OpenAI-compatible, Anthropic, Google, local), Model Registry, probe, Brain Profile, Capability Router, Tool System com permissões/auditoria, transcrição (local + cloud), legendas automáticas, análise de mídia, **Reference Analyzer**, Demand Interpreter (DemandSpec), assistente de chat que executa pedidos pontuais ("adicione legendas", "corte silêncios") via transações, contabilidade de custo, provider de replay para testes.

**Critérios de conclusão:**
- [x] Trocar o Brain entre ≥ 3 providers diferentes sem mudança de código; probe detecta capabilities. *(OpenAI-compatível/Anthropic/Google por servidores HTTP falsos nativos + Replay; smoke com chaves reais pendente, externo)*
- [x] Teste canário: chave nunca aparece em logs/projeto/IPC/crash dumps. *(`service.rs`, `security.rs`, panic hook, UI)*
- [x] Reference Analyzer produz `ReferenceGrammar` com erro de detecção de cortes ≤ 5% — **no corpus anotado por construção** (0 %; held-out 0 %).
  - [ ] **PENDENTE EXTERNO:** corpus de vídeos reais anotados manualmente (`tools/phase4-acceptance/reference-analyzer --corpus`).
- [x] DemandSpec gerada de DOCX+PDF+vídeo com `sources` rastreáveis e **verificadas** (10 briefings em modo Replay: integridade do pipeline).
  - [ ] **PENDENTE EXTERNO:** avaliação manual com LLM real em ≥ 10 briefs reais (`tools/phase4-acceptance/demand-spec --live`).
- [x] Com todos os providers desligados, 100% das funções manuais funcionam. *(E2E "AI Off": zero chamadas `ai.*`, zero rede externa)*

---

## FASE 5 — Autonomia (pipeline completo)

**Objetivo:** brief + bruto + referência → variações editáveis, com plano, revisão e correção.

Escopo: AI Orchestrator (state machine, AI Run persistente/retomável), Producer, Planner (EditPlan), VALIDATE_PLAN via dry-run, Editor (transações), Critic (frames + digest), loop de correção, checkpoints humanos, orçamentos, Memory (4 escopos, propostas, aprovação), Asset Gateway + adapters iniciais, geração de mídia com proveniência/versões, `generate_variants`, undo seletivo por ator.

**Critérios de conclusão:**
- [ ] Em ≥ 10 demandas reais de teste: produz as variações pedidas, 100% editáveis, com custo exibido antes da execução; avaliação humana média ≥ "utilizável com ajustes leves".
- [ ] Nenhuma escrita na timeline antes de plano validado (verificado por auditoria de Runs).
- [ ] Run interrompido (kill) retoma do último stage sem duplicar edições.
- [ ] Memória: nenhuma promoção a Client/User sem aprovação (teste automatizado).
- [ ] Adapter do Gateway desligado → app funciona, erro claro, fallback quando configurado.

---

## FASE 6 — Integração e Finalização

**Objetivo:** automação externa e produto distribuível.

Escopo: `capia-server` (REST, MCP Server, Webhooks) sobre a Engine API, autenticação local por token/escopos, fluxo "copy + raw video + reference → edição automática", instalador assinado, auto-update, crash reporting opt-in, hardening de desempenho, documentação de usuário, beta.

**Critérios de conclusão:**
- [ ] Ferramenta externa inicia uma edição via REST e via MCP, recebe webhook de conclusão; resultado idêntico ao fluxo pela UI.
- [ ] Instalador assinado, update assinado, testado em Windows 10 22H2 e 11 limpos.
- [ ] Zero vazamentos de segredo nos testes de segurança; pentest básico da API local.
- [ ] Metas de desempenho mantidas em projeto real grande (≥ 30 sequences, ≥ 5.000 clips).
