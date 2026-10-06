# STATUS

> **RC3 (0.6.0-rc.3) — leia primeiro.** Candidata de correção depois de auditoria externa do RC2: executor de jobs com ordem `Queued → Running → terminal`, **timeline livre** (sem tracks fixas; comandos `rename_track`/`move_track`), **redesign** (rail de 5 itens, inspector contextual, Texto+Legendas, Sequências à vista), **export medido e ~2× mais rápido**, harness do **OpenAI real** (gate externo) e E2E de chat/erros pela interface. Evidências e o que foi de fato executado: [`RC3_TEST_REPORT.md`](RC3_TEST_REPORT.md); decisões: ADR-121..125. **Pendências externas continuam abertas** (OpenAI real com a chave do usuário, assinatura, Windows limpos, beta, H.264/AAC jurídico, pentest independente, endpoint de crash).

**Última atualização:** 2026-10-05 · **Fase atual:** FASE 5 — Autonomia · **Estado: `PHASE 5 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`** (branch `claude/phase5-autonomy`; seção "Fase 5" abaixo). Fase 4 permanece `PHASE 4 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING` e Fase 3 `PHASE 3 ENGINEERING COMPLETE — HUMAN ACCEPTANCE PENDING` (pendências **não marcadas**). **Fase 6 NÃO iniciada.**

> **Fase 3 (engenharia):** editor manual utilizável de ponta a ponta — shell + design system, `ui-timeline` em canvas virtualizado, projeto/sequences/nested, biblioteca com arrastar-e-soltar, edição manual completa, inspector + keyframes, texto/legendas/transições/áudio, preview P2, histórico, relink, export + deliverables, atalhos, pt-BR/en — tudo por **comandos do Command Engine**. Branch `claude/phase3-editor` (sem PR: não solicitado). **Pendências inevitáveis (humanas/hardware):** (1) teste com ≥ 3 usuários reais (`tools/phase3-acceptance/`); (2) residual de CPU/pacing do P2 em GPU real (`tools/phase3-acceptance/gpu-residual.ps1`); (3) decisão de produto/jurídica de `OUTPUT-H264` (patentes, OpenH264, qualidade de produção) — a **engenharia** do caminho H.264 está integrada e testada no CI Windows.

## Fase 5 — Autonomia: o que existe

> Especificações: `docs/phase5/` · decisões: ADR-087..101 · pacote de aceitação: `tools/phase5-acceptance/`. **CI:** run 37356301933 no commit `587947a` — os 6 jobs verdes (Núcleo Rust Linux, Rust+desktop Windows, TypeScript, Arquitetura/licenças/segredos, E2E Linux, E2E Windows, incluindo a autonomia no app Tauri/WebView2 real sem skip). O HEAD final difere de `587947a` apenas por esta atualização de documentação.

| Área | Estado (engenharia) | Evidência (arquivos de teste) |
|---|---|---|
| Schema 5 / `AutonomyStore` | `ai_runs`, `ai_run_stages`, `ai_run_events`, `ai_side_effects`, `ai_provenance`, `ai_memory`, `ai_memory_log`, `ai_budget_ledger`; migração aditiva de projetos v4 | `capia-store/tests/autonomy_store.rs` |
| AI Run (máquina de estados) | `UNDERSTAND→PLAN→VALIDATE_PLAN→ACQUIRE→EDIT→REVIEW→CORRECT→DONE` + `WAITING_USER/PAUSED/FAILED/CANCELLED`; tabela de transição fechada; cursor atômico (CAS por `revision`); `recover()` → `Running→Paused`, nunca auto-resume; ator `run:<id>` | `autonomy_run.rs`, `autonomy_scenarios.rs`, `autonomy_properties.rs`, unitários `autonomy::machine` |
| Idempotência e orçamento | livro de efeitos (`llm:/gw:/imp:/gen:`, primeira tentativa vence) e livro de orçamento (reserva/liquidação/liberação, atômico) | `autonomy_store.rs`, `autonomy_budget.rs`, `autonomy_properties.rs` |
| Producer / Planner / Editor / Critic | papéis de LLM sem tools (`untrusted_data`); Editor = compilador determinístico `EditPlan → comandos`; escrita só por `preview → apply_plan` após plano validado; Critic determinístico + semântico; REVIEW→CORRECT limitado com vocabulário fechado | `autonomy_scenarios.rs`, `autonomy_security.rs`, unitários `autonomy::{plan,critic,roles}` |
| Memória (4 escopos) | System/User/Client/Project; promoção só por `UserApproval`; precedência Project>Client>User>System; Client exige `client_id`; rejeitado não ressuscita | `autonomy_memory.rs`, `autonomy_properties.rs`, `AiRunsPanel.test.tsx` |
| Asset Gateway + geração | adapters `LocalLibrary`/`ApprovedUrl`/`ReplayCatalog`; `SafeFetcher` (única rede além dos providers); veredito de licença; proveniência; mídia em `<projeto>-media/ai/` (durável); geração opt-in (desligada por padrão), com aprovação + orçamento e job consultado antes de novo submit | `capia-ai/tests/fetch.rs`, `autonomy_scenarios.rs`, `autonomy_crash_acquire.rs`, `autonomy_service.rs` |
| Undo seletivo | `selective_undo_report/selective_undo` (nova entrada; modos `safe`/`partial`; conflitos por entidade **e** por dependência); `history.undo_report`/`history.undo_selective` por `entries` ou `actor_id` | `capia-commands/tests/selective_undo.rs`, `autonomy_variants.rs` |
| Variantes | `ai.run.variants` (Runs filhas em grupo; `hook_plus_master`/`shared_master`/`format_variant`; sequences distintas e editáveis) | `autonomy_variants.rs` |
| Crash/kill | failpoints (feature `failpoints`, só testes) em processo + SIGKILL real com retomada em outro processo | `autonomy_crash.rs`, `autonomy_crash_acquire.rs`, `autonomy_kill.rs` |
| Serviço `ai.*` | `ai.run.*`, `ai.memory.*`, `ai.gateway.*`, `ai.generation.set_enabled`; AI Off recusa Runs e o editor segue | `autonomy_service.rs` |
| UI | painéis Runs/Memory/Sources, só por `aiController.ts`; undo de Run por `controller.undoRun`; geração/fontes desligadas por padrão | `AiRunsPanel.test.tsx`, `aiController.test.ts`, `architecture.test.ts`, `packages/e2e/tests/autonomy.spec.ts` |
| Critic com visão | quadros compostos amostrados → Capability Router (`VisionInput`) → achados com `EvidenceRef` de quadro; degradação explícita; cache por digest; cancelamento; orçamento | `autonomy_vision.rs`, `autonomy/vision.rs` |
| E2E Windows/Tauri | app desktop de teste (feature `e2e-testkit` + env) com cérebro Replay; Run completa, CORRECT, 2 variantes, undo seletivo, kill/reabrir, WAITING_USER, cancel, resume, AI Off — **sem skip** | `packages/e2e/tests/autonomy.spec.ts` |
| Pacote de aceitação | `tools/phase5-acceptance/` (`autonomy`, `crash-resume`, `security`, `gateway`, `memory`, `real-demands`, `run-all.mjs`) | `real-demands/validate.test.mjs` |

### Critérios de saída (ROADMAP Fase 5) — evidência
| Critério | Resultado |
|---|---|
| ≥ 10 demandas reais, variações 100 % editáveis, custo antes, média humana ≥ "utilizável com ajustes leves" | **PENDENTE EXTERNO.** A engenharia (custo estimado antes da aprovação, variações editáveis) tem testes em Replay; a avaliação humana **não foi executada** |
| Nenhuma escrita antes de plano validado | testes: `autonomy_properties.rs::random_walks_over_the_state_machine_never_break_the_write_gates`, `autonomy_scenarios.rs::plan_approval_gates_every_write_…` |
| Run interrompida (kill) retoma sem duplicar | testes: `autonomy_kill.rs`, `autonomy_crash.rs`, `autonomy_crash_acquire.rs` |
| Nenhuma promoção a Client/User sem aprovação | testes: `autonomy_memory.rs`, `autonomy_properties.rs::random_memory_operations_…` |
| Adapter do Gateway desligado → app funciona, erro claro, fallback | teste: `autonomy_scenarios.rs::a_disabled_gateway_never_breaks_the_app_…` |

### Pendências (exatas) para `PHASE 5 COMPLETE`
1. **≥ 10 demandas reais avaliadas por um humano**, média ≥ 4,0, validadas por `node tools/phase5-acceptance/real-demands/validate.mjs --file <resultados.json>` (sem arquivo: `pending_external`).
2. **Providers e chaves reais** (Brain real, provider de geração real, fontes reais do Gateway): nenhum teste da Fase 5 usa rede externa; qualidade/custo/latência reais **não medidos**.
3. Checagem **humana** no desktop Windows com um provider real (o E2E Windows/Tauri automatizado já cobre o fluxo completo com o cérebro Replay; ver ADR-101).
4. ~~CI no HEAD final~~ — verde: run 37356301933 / `587947a` (6 jobs); o HEAD final difere só por documentação.
5. Pendências da Fase 4 (LLM real no DemandSpec, corpus real de cenas, STT real) e da Fase 3 (3 usuários, residual de GPU/P2, decisão jurídica de H.264) **continuam abertas**.

### Limitações conhecidas (Fase 5)
- Fontes do Gateway são locais/URL aprovada/catálogo Replay; não há marketplace de stock real integrado.
- `service/demo.rs` (feature `testkit`) é só dev/E2E; o produto sem provider configurado falha com erro claro.
- **Critic com visão (ADR-100):** o modelo de visão real não foi avaliado (o Replay simula olhando os pixels dos quadros); qualidade de visão com provider real é pendência externa.
- Correções encontradas pelos próprios testes da fase (registradas por honestidade): mídia adquirida ficava só no cache descartável (agora `<projeto>-media/ai/`); gramática de referência e transcrições não chegavam ao Planner/Critic (busca por sujeito do registro); undo seletivo não reportava perda por dependência.
- Fase 6 (REST/MCP/Webhooks, instalador) **não iniciada**.


## Fase 4 — Inteligência (IA assistida): o que existe

> Closeout da Fase 3 (Etapa 0): run 75 do CI verde nos 6 jobs no commit `530ebbf` (ver "CI verde" da Fase 3). Fase 4 desenvolvida em `claude/phase4-intelligence` (sem PR).

| Área | Estado |
|---|---|
| `capia-secrets` | ✅ `SecretString` (zeroize), `SecretStore` (Credential Manager no Windows; memória explícita nos demais), redator central (+ gancho de pânico), canário |
| `capia-ai` | ✅ contrato canônico + adapters **OpenAI-compatível, Anthropic, Google, Replay, whisper.cpp**; registry (capabilities com origem), probe real, Brain Profile, Capability Router (privacidade/orçamento/contexto, fallback só configurado), dispatcher (retry/backoff, fallback, cancelamento real, custo em micro-unidades, cache determinístico), Tool System com gate e auditoria |
| Persistência | ✅ schema 4 (`ai_records`, `ai_usage`) + `AppDb` global; nenhum segredo em disco |
| `capia-intelligence` | ✅ transcrição (chunks, cache), **legendas automáticas**, **remoção de silêncio**, **detecção de cenas** (local), **Reference Analyzer** (`ReferenceGrammar`), extração DOCX/PDF/TXT/MD, **Demand Interpreter** (`DemandSpec` com fontes verificadas), **assistente de chat** (tools; `preview → apply_plan`; Ask/Auto; cancelamento; idempotência), serviço `ai.*` |
| Hospedagem | ✅ `capia-devserver` e `capia-desktop` roteiam `ai.*` e mesclam eventos no `events.poll`; sem o serviço o editor é idêntico |
| UI | ✅ painel de IA (chat com streaming/aprovação, ferramentas, referência, briefing), configuração (providers write-only, modelos com origem das capabilities, Brain/privacidade/orçamento, diagnóstico redigido, uso/custo) — pt-BR/en |
| Pacotes de aceitação | ✅ `tools/phase4-acceptance/` (segurança em um comando; cenas; DemandSpec ≥ 10 briefings) |

### Critérios de saída (ROADMAP Fase 4) — evidência
| Critério | Resultado |
|---|---|
| Brain entre ≥ 3 providers só por configuração; probe detecta capabilities | ✅ `service.rs::the_brain_swaps_between_three_provider_families_by_configuration_only` (OpenAI-compatível ↔ Anthropic ↔ Google, header nativo de cada um) + `contract.rs::probe_measures_real_capabilities_on_all_families` (servidores HTTP falsos que falam cada protocolo). **Externo:** smoke com chaves reais (não é gate) |
| Canário: chave nunca em logs/projeto/IPC/crash | ✅ 0 ocorrências em eventos, status, diagnóstico, histórico, snapshot, uso e **todos os arquivos em disco** (projeto, WAL, cache, AppDb); erro 401 que ecoa a chave volta redigido; pânico redigido; UI sem chave no DOM/estado/`localStorage` (E2E + vitest) |
| Reference Analyzer: erro de cortes ≤ 5 % | ✅ **0 %** (26/26) no corpus anotado por construção (cortes, dissolves, fades, planos sem corte) e *held-out* 4/4, reproduzível (geradores determinísticos). **Externo/pendente:** corpus de vídeos **reais** anotados por humanos (`reference-analyzer/run.mjs --corpus`) — o corpus sintético **não** substitui |
| DemandSpec de DOCX+PDF+vídeo com `sources` | ✅ `demand_flow.rs` (DOCX + PDF + transcrição de vídeo; fonte com documento/unidade/página/instante/citação **verificada**; citação inventada vira `unverified`) e `demand_eval.rs` (10 briefings, modo Replay: 30/30 fatos com fonte verificada, 4/4 fabricações descartadas). **Externo:** avaliação com LLM real e ≥ 10 briefs reais (`demand-spec/run.mjs --live`) **não executada** |
| Tudo desligado ⇒ editor 100 % manual | ✅ E2E "AI Off": edição completa com **zero** chamadas `ai.*` e zero requisições fora do devserver; `ai.enabled=false` e ausência do serviço cobertas (`service.rs`, desktop) |

### CI verde (Fase 4 — evidência)
Run 83 do CI, commit `02b8490` (branch `claude/phase4-intelligence`): **os 6 jobs verdes no mesmo commit** — Núcleo Rust (Linux), Rust + desktop (Windows: clippy, testes do workspace incl. `capia-secrets/ai/intelligence/desktop`, encoders, aceitação da Fase 2, build do desktop), TypeScript, E2E Linux (incl. `ai.spec.ts` e o cenário *AI Off*), E2E Windows (WebView2) e Arquitetura/licenças/segredos (gitleaks). Histórico até o verde: run 80 (lint do runner de aceitação — corrigido), runs 81/82 (Windows: dois testes de **relógio de parede** da Fase 2 — `async_import_returns_long_before_the_full_hash_of_a_huge_file` e `different_keys_do_not_block_each_other` — com folga insuficiente em runner carregado; as asserções foram alargadas **sem remover cobertura**, mantendo o poder de detectar a regressão). Commits posteriores a `02b8490` são apenas documentação e foram verificados verdes no CI antes do fechamento.

### Segurança (resumo; detalhes nas ADRs 078–086)
Credencial write-only (UI não define referência/host), host binding, sem credencial em redirect, SSRF, TLS sem `danger_*` (teste de política + handshake), headers maliciosos/CRLF recusados, corpo gigante/stream infinito/JSON malformado, tool inexistente/argumentos inválidos, injeção em transcrição/PDF/DOCX sem permissão extra (o Interpreter não recebe tools), tool entre projetos/lifecycle de projeto inexistentes, escalada de permissão, tool tardia após cancelar, `operation_id` repetido idempotente. Suíte: `node tools/phase4-acceptance/security/run.mjs` → `target/phase4-acceptance/security-summary.json`.

### Desempenho medido (Linux, release, FFmpeg real)
- Varredura de cenas em 1080p: 120 s de vídeo em 7,8 s ⇒ **≈ 15× tempo real** (RGB24 64×36 a 25 fps; memória O(n·200 B)); `scene_corpus.rs::perf_scan_throughput_1080p` (`--ignored`).
- Suíte de cenas anotada: 10 clipes + *held-out* em ≈ 8 s. Transcrição/LLM: dependem do provider (não medidas sem provider real).

### Pendências (exatas) para `PHASE 4 COMPLETE`
1. **Qualidade de LLM real** no DemandSpec e smoke das 3 famílias com chaves reais (`tools/phase4-acceptance/demand-spec/run.mjs --live`; `CAPIA_ACCEPT_API_KEY`).
2. **Corpus real anotado por humanos** para cortes (`reference-analyzer/run.mjs --corpus`).
3. Transcrição **real** (whisper.cpp local/nuvem) medida em áudio de fala real — o CI usa o provider Replay.
4. Pendências humanas/hardware herdadas da Fase 3.

### Limitações conhecidas (Fase 4)
- Detecção de cenas calibrada em material sintético: vídeo real com movimento de câmera/efeitos pesados pode exigir ajuste dos limiares (`SceneParams`).
- `render.frame` não é oferecido ao assistente (sem codificador PNG de RGBA no caminho de visão); amostragem de quadros usa o FFmpeg e só com modelo de visão e privacidade que permita.
- O assistente é pontual (≤ 8 passos); não há AI Run autônomo, variantes, memória autônoma nem Asset Gateway completo (Fase 5).
- Sem cofre seguro fora do Windows, as chaves ficam só em memória (a UI informa o backend em uso).

## Fase 3 — o que existe

| Área | Estado |
|---|---|
| Shell/UX | ✅ painéis redimensionáveis/colapsáveis (persistidos, à prova de corrupção), rail (Projeto, Mídia, Áudio, Texto, Legendas, Transições), menus de contexto, toasts, diagnóstico de erro, tema escuro próprio (`@capia/ui-kit`, ícones próprios) |
| Timeline (`@capia/ui-timeline`) | ✅ canvas 2D virtualizado (busca binária, sem DOM por clip), ghost/snap/grupo/colocação pelo **WASM do core** (ADR-070), seleção/marquee/move/reordenar magnético/trim/blade/split/delete/ripple/copiar-colar/duplicar/grupos, faixas (lock/hide/mute/solo/magnética/altura), marcadores, zoom/fit, scrub, miniaturas, waveform |
| Projeto | ✅ árvore (pastas, sequences, uso por nested), abas, criar/duplicar/renomear/excluir, formatos (vertical/quadrado/4:5/horizontal), nested (abrir, breadcrumb, master compartilhado, tornar único) |
| Biblioteca | ✅ importação assíncrona, miniaturas, filtros/ordenação, arrastar para a timeline (preview do drop), offline/modificado + relink por arquivo/pasta + force relink |
| Inspector | ✅ sequence · clip (nome, ativo, transform, opacidade, transição) · texto (conteúdo + estilo) · áudio (volume/fades/separar áudio) · velocidade · **keyframes** (adicionar/mover valor/interpolação linear-hold-ease/remover) |
| Texto/legendas/transições/áudio | ✅ títulos e terço inferior · legendas manuais (lista, dividir, mesclar, estilos) · dissolve/fade/slide-in com validação de *handles* · volume, fades, detach |
| Preview | ✅ P2 (ADR-074): SharedBuffer→WebGL no app Windows, IPC binário como queda; 540p/720p/Auto, proxy (modo de desempenho), safe areas, caixa de transformação com arrastar/escalar, fullscreen, métricas (fps, latência, *dropped slots*), *latest-wins*. **Áudio de monitoração** (ADR-077): o engine mixa blocos de 0,5 s (`render.audio`), o relógio de áudio manda no playhead a 1×; mudo em outras velocidades |
| Histórico | ✅ lista por transação com atores, ir a qualquer ponto (undo/redo em lote), `Ctrl+Z/Shift+Z` |
| Export | ✅ diálogo (sequence, preset H.264/intermediário, encoder aprovado, tamanho, destino), deliverables em lote, progresso/cancelamento reais, relatório ffprobe, texto de limite legal (ADR-075) |
| Atalhos / i18n | ✅ keymap padrão + edição + conflito + reset; pt-BR/en com chaves tipadas (paridade verificada em teste) |
| Multi-cliente | ✅ `revision_changed`: a UI ressincroniza quando outro cliente altera o documento (ADR-072) |
| Desempenho | ✅ instrumentação (`PerfLog`, amostras de pintura/gesto) + `perf.spec` com 5.000 clips; resultados em "Metas de UX medidas" |

### CI verde (evidência)
**Closeout da Fase 3 (Etapa 0 da Fase 4):** run 75 do CI, commit `530ebbf` (branch `claude/phase3-editor`): **os 6 jobs verdes no mesmo commit** — inclui a instalação robusta do FFmpeg no Windows (`tools/ci/install-ffmpeg-windows.ps1`: cache → choco com *backoff* → zip do GitHub → falha dura), os 17 fluxos E2E Linux e a repetição ×6 do undo/redo longo (causa-raiz do flaky: instantâneo tirado antes do `delete` final assíncrono — a espera agora é pela estabilização do histórico) e a correção do `rapid_scrub` (`Cancelled` por supersessão volta como `Superseded`, 25/25 localmente). Nenhuma cobertura foi reduzida. O run 73 (HEAD `a76937f`) falhou por infraestrutura (choco 503) e por esse flaky — ambos corrigidos na raiz.
Run 72 do CI, commit `72a0600` (branch `claude/phase3-editor`): **todos os 6 jobs verdes** — Núcleo Rust (Linux), Rust + desktop (Windows: fmt, clippy, testes do workspace, detecção de encoders, aceitação da Fase 2, build Tauri), TypeScript, arquitetura/licenças/segredos, **E2E Linux** (17 testes + perf de 5.000 clips) e **E2E Windows do app real** (WebView2/CDP; `CAPIA_REQUIRE_SHARED_BUFFER=1` e `CAPIA_REQUIRE_H264=1` impostos pelo teste ⇒ transporte `shared-buffer` e export H.264 aprovado verificados pelo `ffprobe`). **Ressalvas honestas:** no alvo Windows 3 testes são *skipped* por desenho (os dois `kill -9` e o de vazamentos, que dependem de Chromium/flags do Playwright e já rodam no Linux); o runner Windows é software (sem GPU real, sem NVENC/QSV/AMF) — o residual de GPU continua pendente.

### Metas de `TIMELINE_UX.md` §6 — medidas (5.000 clips, Chromium headless **sem GPU** + engine release, Linux)
| Meta | Medido (p95 salvo nota) | Estado |
|---|---|---|
| Scroll/zoom 60 fps | pintura ≈ 11–13 ms (p95), 1.765 clips visíveis no *fit* | ✅ (CPU raster; GPU só melhora) |
| Arrasto < 16 ms | ≈ 3–4 ms por `pointermove` (ghost local, sem IPC) | ✅ |
| Commit < 30 ms | engine p50 4,3 ms / p95 10,8 ms; percebido na UI p50 31 ms / **p95 34,9 ms** (run 72, headless, núcleos compartilhados com o Chromium) | ⚠️ engine ✅; **percebido na UI NÃO atinge a meta neste ambiente** (`targets` do relatório: `false`) — reavaliar em hardware de referência |
| Undo/redo < 50 ms | engine p95 8,2 ms; UI p50 20,6 ms / p95 23 ms (run 72) | ✅ |
| Scrub do preview < 100 ms | p50 26 ms / p95 37,8 ms / máx 113,8 ms (run 72; 5.000 clips sólidos, 404×720, 0 quadros descartados) | ✅ (p95) |
| Miniaturas < 200 ms | 1ª busca fria medida em `thumbnailLatencyMs` (imagens/vídeos de teste) | ✅ (sem estresse de mídia real) |
| Waveform sem recálculo no zoom | pirâmide `CWFM` (Fase 2) | ✅ |
Relatório bruto: `target/perf/phase3-ui-perf.json` (artefato do CI). **Honestidade:** são números de CI/sandbox sem GPU real; o benchmark estrito é `CAPIA_PERF_STRICT=1` localmente.

### Limitações conhecidas (Fase 3)
- Áudio do preview só a 1× (em J/L ≠ 1× fica mudo) e sem *scrub* sonoro; o ajuste fino A/V do WebAudio **não foi medido** em hardware real.
- Rotação não-múltipla de 90° não é renderizada (limite herdado da Fase 2); o inspector aceita o valor e o render avisa.
- "Proxy" no preview = resolução reduzida (modo de desempenho), **não** decodifica o proxy de mídia (ADR-063).
- Reverso de clip: exibido, sem comando de edição.
- Perf: o commit percebido na UI (p95 34,9 ms) **não** atinge a meta de 30 ms neste ambiente (engine sozinho ✅); undo/redo ✅ (ver tabela).
- Lacunas de teste: E2E do app nativo só roda no CI Windows; o residual de GPU e os 3 usuários dependem de pessoas/hardware.

### Aceitação humana / hardware (pendente — pacote pronto)
`tools/phase3-acceptance/README.md`: tarefa UGC cronometrada, formulário de severidade, validador (`PENDENTE` sem resultados reais), `gpu-residual.ps1` (PASS/FAIL na tela). **Nada disso foi executado nem fabricado.**

## Gates de fase (histórico — decisão do Product Owner, ADR-037)

```
Fase 2 — Motor (headless)     CONCLUÍDA.
Fase 3 — Editor / Preview     OD-1 FECHADA (ADR-069, P2). Pode iniciar; residual: CPU/pacing do P2 em GPU real.
```

## Estado

| Item | Estado |
|---|---|
| Núcleo de tempo (`capia-time`) | ✅ `Ticks` (aritmética *checked*, half-up/floor), `Rational`, `FrameRate`, `TimeRange` — 25 testes |
| Modelo (`capia-model`) | ✅ Document com compartilhamento estrutural, Sequence/Track/Clip/Marker/Asset, keyframes (Bézier exato), **ops primitivas invertíveis**, invariantes (sobreposição, magnética, alinhamento, família, fonte, nested DAG/profundidade 16, limites) — 20 testes |
| Command Engine (`capia-commands`) | ✅ 19 comandos, transações atômicas, histórico/undo/redo, `operation_id` idempotente (ADR-029), `preview → apply_plan` por token HMAC (ADR-030), rebase/`CONFLICT`, refs `$nome`, auditoria — ver "O que existe" |
| Suíte de aceitação (ADR-036) | ✅ **120/120** cenários no engine real (108 + 12 da M05); mutação detecta regressões |
| Persistência (`capia-store`) | ✅ M06: `.capia` = SQLite v1 (STRICT, WAL, `synchronous=FULL`), journal de eventos + snapshots, migrations explícitas, commit atômico, idempotência durável, undo/redo após reabrir, crash tests com processo morto (ADR-042..044) |
| Facade + CLI (`capia-project`, `capia-cli`) | ✅ M06: `capia create/inspect/validate/apply/undo/redo/history/dump` |
| Nested (comandos) | ✅ M06: `delete/rename_sequence`, `insert_nested`, `set_nested_target`, `set_follow_length` (ADR-045) |
| Mídia (`capia-media`) | ✅ M07: probe via `ffprobe` atrás do trait `MediaProbe`, metadados normalizados (Ticks/Rational), localização do backend, processos com timeout e teto de saída, miniatura (ADR-047) |
| Assets (`capia-assets`) | ✅ M07: identidade por conteúdo (`sha256:` em streaming), import, dedup, online/offline/modified, relink por conteúdo, cache derivado descartável (ADR-046/048/049) |
| Catálogo de mídia no `.capia` | ✅ M07: schema 2 (`media_assets`, `asset_events`), migration real v1→v2, import atômico (documento + catálogo na mesma transação) |
| Composição de nested | ✅ M07: `duplicate_sequence`, `make_unique`, `flatten_nested`, `create_nested_from_selection`, `generate_variants`, `delete_asset` (ADR-050) |
| Jobs (`capia-jobs`) | ✅ M08: executor bounded, prioridades por créditos (sem starvation), cancelamento real (mata o FFmpeg), dedup, persistência e recuperação (`interrupted`) — ADR-052 |
| Import não bloqueante | ✅ M08: `ImportTicket` (pending → finalized), impressão rápida × SHA-256 em background, `ASSET_CHANGED_DURING_PROCESSING` — ADR-053 |
| Índice de quadros / decode exato | ✅ M08: `CIDX` (CFR/VFR/B-frames/GOP longo), `RawFrame` RGBA8, decode de intervalo — ADR-054 |
| Áudio PCM / waveform / proxy | ✅ M08: f32 intercalado com intervalos exatos, waveform `CWFM` multirresolução, `ProxyProfileV1` (MJPEG LGPL; H.264 só por hardware) — ADR-055/056 |
| Cache derivado v2 | ✅ M08: produção atômica, lock por chave (em-processo + SO), validação, GC — ADR-057 |
| Force relink / relink em lote | ✅ M08: `update_asset` (clips dependentes validados, sem trim), relink por pasta (tamanho → impressão → SHA-256, nunca por nome) — ADR-058 |
| Schema | ✅ M08: schema 3 (`jobs`, `import_tickets`, `media_assets.fingerprint`), migration real v2→v3 |
| Render (`capia-render`) | ✅ grafo + compositor CPU de referência + mixer, puro/WASM; 10 goldens de vídeo, 8 de áudio (ADR-063..065) |
| Decode persistente (`capia-decode`) | ✅ pool de sessões, cache de quadros por bytes, prefetch, supersession, métricas (ADR-059/060) |
| Áudio: índice + seek + cache PCM | ✅ `CAIX`, seek com pouso verificado, custo independente do início, `PcmCache` (ADR-061/062) |
| Preview headless (`capia-preview`) | ✅ scheduler, `FrameSink`, playhead, cadência, descarte de obsoletos; paridade com o export (ADR-066) |
| Export | ✅ intermediário atômico, MP4 por `EncoderCapability` aprovado, validação ffprobe, CLI `render`/`export`/`media encoders` (ADR-067/068) |
| CI | ✅ Linux (fmt, clippy, testes, propriedades pesadas, render ampliado, perf, WASM + paridade), Windows (fmt, clippy, workspace, encoders, E2E, goldens, desktop build), TypeScript, políticas — verde no commit final |
| Decisões abertas | **OD-1** (gate da Fase 3; aberta) · **OUTPUT-H264** (engenharia provada, decisão de produção/jurídica pendente). **As 8 decisões S7 estão definitivas** (ADR-039) |
| Git | `main` = M01–M04; M05 em `claude/m05-core-engine`; M06 em `claude/m06-persistence-cli`; M07 em `claude/m07-assets-media`; M08 em `claude/m08-media-pipeline`; **conclusão da Fase 2 em `claude/phase2-completion`** (a partir de `98b7117`; sem PR: não solicitado) |

## O que existe (conclusão da Fase 2)

**Render (ADR-063..065):** `capia-render` (puro, WASM): `RenderGraph`, `render_frame`/`render_video_range`/`mix_audio_range`; RGBA8 alfa reto, base preto opaco, *source-over* inteiro, opacidade, scale/position, rotação ×90°, nested, retime, mixer f32 com hard clip; texto ⇒ aviso. `Project::render_frame/render_range/render_audio_range` (fonte = original; nunca escreve no documento).
**Decode (ADR-059/060):** `FrameStream` (sessão persistente) + `DecodeService` (pool, prioridade, supersession por `Lane`, métricas) + `ByteLru` (chave `namespace+conteúdo+stream+PTS+formato+versão`).
**Áudio (ADR-061/062):** `AudioIndex`/`CAIX` + `decode_audio_indexed` (pouso verificado por `ashowinfo`, fallback do início) + `PcmCache` (blocos, read-ahead). Job `audio_index`. `AudioStream.time_base` no probe.
**Preview (ADR-066):** `PreviewScheduler`, `Clock` (`SystemClock`/`ManualClock`), `HeadlessSink`.
**Export (ADR-067/068):** `EncoderCapability` (detecção real), `export_intermediate` (`capia-intermediate-v1`), `export_mp4` (pipe → ffmpeg, ffprobe, `rename`), staging órfão limpo. **CLI:** `media encoders`, `render frame|audio`, `export intermediate|mp4`.

## Matriz de requisitos da conclusão da Fase 2

| Requisito | Estado | Teste que prova |
|---|---|---|
| Decode persistente (pool, reuso, ociosidade, prioridade, cancelamento, supersession, métricas) | ✅ | `capia-decode/tests/service.rs` (13) |
| Cache de quadros por bytes (LRU, orçamento, hit, conteúdo trocado, multi-stream, concorrência, sem contaminação) | ✅ | idem + `cache::tests` (4) |
| Prefetch (frente, limitado para trás, salto), interativo > background, scrub t1→t4 | ✅ | `prefetch_fills…`, `interactive_requests_outrank…`, `rapid_scrub_supersedes…` |
| Índice de áudio + seek rápido (44,1/48 kHz, AAC e PCM, fronteiras, início ≠ 0, curto, além do fim, blocos adjacentes) | ✅ | `capia-media/tests/audio_seek.rs` (5), `capia-decode/tests/audio.rs` (5) |
| 1 s em t = 300 s não escala com o início | ✅ | `seek_cost_does_not_scale_with_the_start` (janela decodificada ≈ 5,3 s em t = 0/60/300) |
| Cache de PCM por bytes | ✅ | `byte_budget_is_respected…`, `repeated_reads_hit_the_cache…` |
| `capia-render`: grafo, compositor, mixer, APIs, aritmética *checked* | ✅ | `capia-render` (unidade + goldens) |
| Goldens: ≥ 10 de vídeo e 8 de áudio | ✅ | `golden_video.rs` (10), `golden_audio.rs` (8, tolerância 1e-4) |
| Determinismo: mesma execução, reabrir, cache frio×quente, proxy presente×ausente, ordem | ✅ | `parity.rs` (3), `properties_render.rs` |
| Export intermediário atômico (temp → validar → publicar; kill nunca publica parcial) | ✅ | `export.rs` (7), `crash_phase2.rs` |
| `EncoderCapability` + `capia media encoders` (NVENC/QSV/AMF/`h264_mf`/OpenH264) | ✅ | `encoder::tests`, `encoder_detection_reports_every_backend…`, CI Windows |
| MP4/H.264 válido por encoder aprovado, ffprobe (contêiner, codec, duração, fps, quadros, tamanho, áudio, sync) | ✅ **Windows** (`h264_mf`) · Linux: contrato (falha estruturada, sem publicar) + `mpeg4-reference` | `phase2_e2e` no CI Windows (`H.264 approved encoder available on this host: true`) |
| Nunca libx264/libx265/GPL; sem fallback silencioso | ✅ | `gpl_encoders_are_refused…`, `there_is_no_silent_fallback…`, mutação #15 |
| Preview headless (scheduler, sink, playhead, cadência, descarte) + paridade com hashes exatos | ✅ | `capia-preview/tests/scheduler.rs` (6), `parity.rs` |
| Corpus A/V (VFR, 23,976, 29,97, 59,94, 44,1/48 kHz, 10 min) com drift ≤ 1 quadro | ✅ | `av_drift.rs` (3) |
| CLI E2E (3 sequences + nested, undo/redo, reabrir, export MP4, ffprobe, validar) | ✅ | `capia-cli/tests/phase2_e2e.rs` |
| Crash real (decode ativo, render range, índice de áudio, export intermediário ×2, MP4 ×2) | ✅ | `crash_phase2.rs` (7) |
| Propriedade de render × oráculo (posição, trim, velocidade, ordem, opacidade, transform, nested, reabrir, frio/quente/minúsculo) | ✅ | `properties_render.rs` (6 casos local; **80 no Linux release do CI**; 2 smoke no Windows) |
| Mutação (16) | ✅ **16/16 detectadas** | `tools/mutation-phase2.py` |
| Medições de desempenho | ✅ | tabela abaixo |
| WASM / fronteiras / licenças | ✅ | `cargo check wasm32` inclui `capia-render`, paridade 150 sequências, `check:arch`, `cargo deny` |

### Desempenho da conclusão (release, Linux VM; só medições — nenhuma meta inventada)

| Medida | Resultado |
|---|---|
| Decode 1080p mpeg4 GOP 30: 1º quadro frio (abre sessão + GOP) · quadro repetido (cache) | 135 ms · **0,3 ms** |
| Decode sequencial 120 quadros 1080p | 28,9 ms/quadro (**34,7 fps**), **1 sessão aberta, 119 reaproveitadas, taxa 0,99** |
| Decode seek aleatório sem cache (24 pedidos) | 166 ms/seek (21 sessões novas, 3 reaproveitadas) |
| Decode VFR (fixture 64×48, 25 quadros) | 92 ms total |
| Áudio 1 s em t = 0 / 60 / 300 s (WAV 44,1 kHz, frio) | 65 / 77 / 76 ms (janela decodificada igual nos três) |
| Áudio 30 blocos sequenciais de 1 s · *hit* quente | **26 ms/bloco** (10 chamadas ao decoder) · 0,02 ms |
| Compositor 1080p, Synth: 1 clip · 2 layers 50% · 2 layers + scale/pos/opacidade · rotação 90° · nested | 26 · 67 · 65 · 44 · 94 ms/quadro (38 · 15 · 15 · 22 · 11 fps) |
| Timeline de 10 s (300 quadros), 2 layers com keyframes | 65 ms/quadro (15,4 fps) |
| Projeto real 1080p, 2 layers (decode + composição), 90 quadros: frio · quente | 57,5 · 58,5 ms/quadro (17 fps); cache de quadros 253 MiB (32 entradas) |
| `render_audio_range` 3 s estéreo 48 kHz | 173 ms |
| `export_intermediate` 1080p, 90 quadros (712 MiB crus) · `export_mp4` (mpeg4) 1080p | 5,1 fps · 12,3 fps (arquivo 6,3 MiB) |

O compositor CPU é a **referência determinística**, não o caminho de tempo real (esse é o wgpu da Fase 3).

### Limitações restantes

- **OD-1 aberta:** nenhuma apresentação de preview na janela; **sem** medição de GPU/WebView2. A Fase 3 não começa.
- **OUTPUT-H264 (decisão pendente):** `h264_mf` provado só no runner Windows (software, sem NVENC/QSV/AMF lá); falta decisão de produto/jurídica (patentes, OpenH264) e qualidade/bitrate de produção. Na VM Linux nenhum H.264 aprovado está disponível (x264/x265 presentes e proibidos).
- Não implementados (escopo): compositor wgpu/RGBA16F, texto (`TEXT_NOT_RENDERED`), transições, efeitos, máscaras, rotação arbitrária, VA-API (declarado indisponível), gerência de cor da **fonte** além do padrão do ffmpeg (ADR-065), rotação de exibição do vídeo (metadado ignorado), EXIF em imagens.
- Seek de áudio em mp4 com vídeo de GOP > 4 s cai no *fallback* do início (correto, mais lento); AAC é exato em posição e igual em valor até ~3·10⁻³ no ponto de seek.
- Mutação e perf não rodam no CI (manuais: `tools/mutation-phase2.py`, testes `--ignored`; só o perf roda como medição no job Linux).
- Sessões de decode só otimizam acesso para frente; scrub para trás longo reabre sessão (prefetch limitado).

## Próxima missão (proposta; **não iniciada**)

S1 concluído (ADR-069, P2). Fase 3 pode iniciar; na primeira entrega de preview, rodar `tools/s1-preview-spike` (`powershell -ExecutionPolicy Bypass -File .\run.ps1 -P2Res 1280x720`) em PC com GPU real como critério de aceitação. Em paralelo (não bloqueia): decisão de produto sobre `OUTPUT-H264`.

## O que existe (M08)

**Jobs (ADR-052):** `capia-jobs` — N workers fixos, fila limitada por categoria (`interactive`/`normal`/`background`, créditos 6/3/1: o background sempre avança), `CancelToken` que chega ao processo filho (o FFmpeg é **morto**; o PID some), `dedup_key`, *panic* contido, `shutdown ⇒ interrupted`. Persistência no `.capia` (schema 3) por `JobStore` (conexão própria, é o `JobSink`); ao reabrir `queued/running ⇒ interrupted` (nunca `completed`); um único processo é dono do executor (lock do SO); cancelamento entre processos pela flag no banco (≤ 250 ms). Workers só calculam e escrevem **cache**; documento/catálogo mudam na thread do projeto (`Project::pump`).
**Identidade assíncrona (ADR-053):** impressão rápida `fp1:` (tamanho + regiões amostradas + versão; ≲ 1 MiB lido; **nunca identidade**) e SHA-256 completo em job cancelável com progresso. `import_asset_async` devolve um **`ImportTicket`** em ~2–8 ms (arquivo de 256 MiB: hash completo leva 1,4 s); o asset só existe após `pump` finalizar o ticket (mesma transação atômica do import síncrono). `ASSET_CHANGED_DURING_PROCESSING` (crescimento, regravação, troca do arquivo) ⇒ nada registrado. Tickets `interrupted` podem ser retomados.
**Índice de quadros (ADR-054):** pacotes reais (`pts/dts/duração/keyframe`) em streaming ⇒ `FrameIndex`; formato binário `CIDX` v1 (cabeçalho 64 B + 24 B/quadro + SHA-256; **toda truncagem e todo byte alterado rejeitados**); lookups `frame_at_or_before/after`, `nearest_frame`, `keyframe_before`, `frame_by_index` por PTS reais (**nunca `N/fps`**: VFR e B-frames provados por pixels); cache por `(hash, stream, produtor+ffprobe)`. **Decode exato:** keyframe → `-seek_timestamp -ss -copyts` → `select=eq(pts,N)`; `RawFrame{width,height,stride,RGBA8,pts,índice,tempo,bytes}`; limites e aritmética *checked* antes do processo. `decode_frame_range`: N quadros com **um** processo (3,5 ms/quadro vs 85 ms).
**Áudio e waveform (ADR-055):** PCM f32 intercalado, intervalos em **amostras inteiras** (44.100/48.000 exatos; início ≠ 0; clipes curtos; além do fim ⇒ só o que existe; stream não-padrão). Waveform `CWFM`: pirâmide `(min,max,rms)` mono, nível 0 = 64 amostras, ×4 por nível, checksum; decode que falha no meio ⇒ **nenhum** waveform.
**Proxy (ADR-056):** `ProxyProfileV1`; padrão **MJPEG** (nativo, LGPL, intra-only) em `.mov` + AAC; `fps=keep` preserva os instantes (teste VFR); H.264 só por encoder de hardware/SO (`libx264/libx265` nunca; `MEDIA_ENCODER_UNAVAILABLE` se ausente); cancelar mata o ffmpeg e apaga o parcial; **o proxy nunca é fonte de decode** (teste).
**Cache v2 (ADR-057):** `<cache>/<op>/<16 hex do conteúdo>/<chave>.<ext>`; `produce`: lock por chave → temp em `.tmp/` → validar → `fsync` → `rename`; parcial nunca publicado, corrompido vira *miss*, 12 produtores da mesma chave geram **uma** vez; `usage`, `remove_unused`, `invalidate_content`, `sweep_temp`, `clear` (projeto segue válido).
**Force relink e lote (ADR-058):** `update_asset` (novo comando) valida todos os clips dependentes (`CONFLICT` estruturado; **sem trim silencioso**); documento + catálogo na mesma transação; derivados antigos invalidados; `--dry-run`. Lote por pasta: varredura segura (profundidade 16, 500 mil arquivos, symlinks/junctions não seguidos, sem laços, erros de permissão não fatais) + tamanho → impressão → **SHA-256**; `matched/unresolved/ambiguous/rejected/errors`; nunca por nome.
**CLI:** `asset import --async|force-relink|relink-folder`, `media index|frame|waveform|proxy`, `job list|status|cancel`, `cache info|clean`.

## Matriz de requisitos da M08

| Requisito | Estado | Teste que prova |
|---|---|---|
| `capia-jobs` (estados, prioridades, filas limitadas, justiça, cancelar, panic, dedup) | ✅ | `capia-jobs/tests/executor.rs` (13) |
| Cancelamento real (hash para; FFmpeg morto — PID some; temp removido; nada publicado) | ✅ | `hashing::cancelling_stops_hashing…`, `streaming_runner_*`, `cancelling_a_proxy_kills_ffmpeg…`, CLI `a_job_cancelled_from_another_process…` |
| Recuperação (running → interrupted; retry) | ✅ | `capia-store/tests/jobs.rs`, `closing_the_project_interrupts…`, `crash_media.rs` |
| Impressão rápida + colisão deliberada onde o SHA-256 separa | ✅ | `hashing::a_deliberate_collision…`, `batch_relink_rejects_a_same_size_same_fingerprint…` |
| SHA-256 em job; import que volta antes do hash; mudança durante o hash | ✅ | `async_import_returns_long_before…` (7,7 ms × 1,38 s), `growing_or_rewriting…`, `a_file_that_changes_while_it_is_hashed…` |
| Índice CFR/VFR/B-frames/GOP longo; formato binário com checksum e limites | ✅ | `index::tests` (12), `pipeline.rs` (cfr/vfr/offset mkv) |
| Decode exato por pixels; RawFrame; limites | ✅ | `cfr_long_gop…`, `vfr_lookup_and_decode…`, `a_container_with_a_nonzero_start_offset…`, `a_frame_range_equals…` |
| Áudio PCM exato (44,1/48 kHz, início ≠ 0, curto, além do fim) | ✅ | `audio_intervals_are_sample_exact…`, `short_clips…`, `a_non_default_audio_stream…` |
| Waveform multirresolução versionado com checksum | ✅ | `waveform::tests` (5), `waveform_from_a_real_file…`, `a_decode_that_dies_midway…` |
| Proxy (perfil, encoder, cancelamento, nunca autoritativo) | ✅ | `proxy::tests`, `proxy_keeps_every_frame…`, `the_proxy_is_never_used_as_the_decode_source` |
| Cache atômico + lock + GC + corrupção | ✅ | `capia-assets/tests/cache.rs` (8) |
| Force relink (conflitos, sem trim, dry-run) / lote por pasta | ✅ | `force_relink.rs` (5), `pipeline.rs` (batch ×3), CLI `force_relink_and_relink_folder_commands` |
| CLI `job`/`media`/`cache`/`asset` novos | ✅ | `capia-cli/tests/media.rs` (4, binário real) |
| Propriedade (importar→…→reabrir→offline→lote→cancelar→repetir) | ✅ | `properties_media.rs` |
| Crash real (índice, waveform, proxy, import pendente) | ✅ | `crash_media.rs` (4 cenários) |
| Mutação (13) | ✅ 13/13 | `tools/mutation-m08.py` |
| Desempenho | ✅ | tabela abaixo |
| WASM / fronteiras | ✅ | `cargo check wasm32-unknown-unknown` (time/model/commands), `check:arch`, paridade 150 sequências |
| Schema 3 + migration real v2→v3 | ✅ | `capia-store/tests/schema.rs` (cadeia 1→2→3→4 sintética) |

## Validação M08 (container Linux, Rust 1.97, ffmpeg/ffprobe 6.1.1-3ubuntu5)

| Verificação | Resultado |
|---|---|
| `cargo fmt --check` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` | ✅ · ✅ · ✅ **371 testes** (M07: 267) + 10 `#[ignore]` de medição |
| Propriedade com IO (release, 5.000) | ✅ store (timeline+nested) 166 s · assets 104 s (inalterado desde a M07) |
| Propriedade do pipeline de mídia (release) | ✅ **200 casos × 14 passos** com FFmpeg real e modelo independente (import assíncrono, índice, decode por pixels, waveform, proxy, cancelar/repetir, offline, lote, reabrir, `cache clean`) |
| Crash real | ✅ kill dentro do índice, do waveform, da codificação do proxy e com import pendente |
| Mutação manual (13) | ✅ **13/13** detectadas — 3 sobreviveram na 1ª rodada e **os testes foram reforçados** (ver achados); a remoção de **uma** camada de defesa em profundidade (lock em-processo **ou** do SO; filtro de impressão **ou** SHA-256) sobrevive por desenho — as duas juntas são detectadas |
| M05–M07 preservados | ✅ aceitação 120/120, golden digest, paridade WASM, crash tests do store, propriedades |
| `pnpm lint/format:check/typecheck/test/test:tools/build`, `check:arch`, `cargo deny` | ✅ |

### Desempenho M08 (release, Linux, VM sem SHA-NI)

| Medida | Resultado |
|---|---|
| Jobs: 1.000 triviais (submit / execução incluindo overhead) | 15 ms / 15 ms (**15 µs por job**) |
| Jobs: cancelar 1.000 na fila · latência de um interativo atrás de 200 de fundo | 0,6 ms · 1,95 ms |
| Impressão rápida de 256 MiB | **5,5 ms** |
| SHA-256 256 MiB (sync / job com progresso) | 1,37 s (**186 MiB/s**) / 1,43 s (+4 % de custo do cancel+progresso) |
| **Import assíncrono: retorno antes do hash completo** (256 MiB) | **mediana 7,7 ms** (máx. 10 ms) × 1,38 s de SHA-256 ⇒ **179× mais rápido** |
| Import de arquivo pequeno real: síncrono / assíncrono (retorno · finalizado) | 63 ms / 1,7 ms · 62 ms |
| Índice: fixture (50 quadros) · sintético 10 min (**18.000 quadros**, 300 keyframes) | 70 ms · **289 ms** (arquivo de 432 KB) |
| Índice: carga do cache (18.000) · lookup `frame_at_or_before` | 5,7 ms · **65 ns** |
| Decode: frio (meio do arquivo) · repetido · perto do fim | 74 ms · 75 ms · 88 ms |
| Decode: 30 consecutivos, um processo por quadro × **um processo para 30** | 2,56 s (85 ms/quadro) × **106 ms (3,5 ms/quadro)** |
| Áudio: 1 s no início · 1 s em t = 300 s (decode exato desde o início) | 75 ms · **1,55 s** (custo O(início): ver limitações) |
| Waveform 10 min: gerar · ler+validar · `query` (2.000 buckets) | 3,1 s · 82 ms · **18 µs** |
| Proxy MJPEG 640×360, 10 min | 31 s ⇒ **19,4× tempo real** (317 MB) |
| Cache: *hit* validado · 8 threads × 500 *hits* via `produce` · *hit* via job | 2 µs · 25,7 µs/hit · 4,3 ms/job |

### Achados da auditoria e dos testes M08 (corrigidos)

- **Lock de cache preso após crash** (achado pelo crash test): a 1ª versão usava um arquivo `create_new` com expiração de 10 min; um kill no meio do índice deixava a chave presa e o retry travava. Agora lock consultivo do SO (`File::try_lock`), liberado com o processo; mantida a exclusão em-processo (ADR-057).
- `submit_frame_index/waveform/proxy` aceitavam asset sem o stream e só falhavam dentro do job → agora **falham no submit** (`MEDIA_UNSUPPORTED_FORMAT`).
- `Project::pump` perdia os itens rastreados se uma finalização falhasse no meio → agora o item volta à fila e é refeito no próximo `pump`.
- Mutação: **#7** (decode falho no meio virando waveform) sobreviveu — não havia teste de ffmpeg que morre com PCM parcial → ffmpeg falso (`exit 3` após 400 KB) cobre waveform e PCM; **#10** (relink em lote fraco) sobreviveu porque o filtro de impressão mascarava o ramo do SHA-256 → teste novo com arquivo de 24 MiB que difere só fora das regiões amostradas (mesma impressão, SHA-256 diferente); **#13** (lock removido) sobrevivia por haver duas camadas — o script remove as duas.
- Falhas só de teste: tempo relativo do índice (`start_pts`) no teste de duplicatas; `nullsrc` com 25 quadros; luma < 16 saturava (fixtures passaram a `20 + passo·N`); modelo da propriedade ignorava alias online e pasta inexistente.
- **Disco cheio no ambiente** (alocação por sessão): `target/` de debug com debuginfo passou de 19 GB; a validação passou a rodar com `CARGO_PROFILE_*_DEBUG=0`.

### Limitações restantes (M08)

- **Decode de áudio é exato mas O(início):** parte do início do stream e descarta até `start` (1 s em t = 300 s de AAC custa 1,5 s; PCM/WAV é bem mais rápido). Próximo passo: índice de pacotes de áudio + seek com margem e conferência de alinhamento.
- **Decode de vídeo = um processo por chamada** (74–88 ms; 3,5 ms/quadro no intervalo). Um pool/daemon de decode e cache de quadros ficam para o render/preview.
- O índice vem de **pacotes**: codecs em que 1 pacote ≠ 1 quadro (raros) não são tratados; PTS duplicado devolve o primeiro quadro no decode por PTS.
- Derivados exigem fonte coerente por **tamanho + impressão amostrada**; troca de conteúdo fora das regiões amostradas e com mesmo tamanho só é pega por `verify` (SHA-256).
- **Proxy:** MJPEG é pesado em disco (317 MB para 10 min a 640×360) e o caminho H.264 por hardware só tem a lógica de escolha testada (a VM não tem NVENC/QSV/VideoToolbox); não há política de remoção automática de proxies.
- Waveform é **mono** (mix); sem waveform por canal.
- *Undo* de `update_asset` reverte só o documento (catálogo fora do undo, ADR-048 §6).
- Jobs reiniciam do zero após `interrupted` (derivados parciais são inválidos por desenho); `known_paths` segue em texto; `<proj>.jobs-lock` e `<proj>.capia-cache/` ficam ao lado do projeto.
- Resultados do CI (Windows) no relatório de entrega; os testes de PID usam `/proc` (Unix) e `tasklist` (Windows).

## O que existe (M07)

**Identidade (ADR-046):** `AssetId = ast_` + 32 hex do `content_hash`; `content_hash = sha256:<64 hex>` do arquivo **inteiro**, em blocos de **1 MiB** (nunca na memória; teste com leitor-espião prova que nenhuma leitura pede mais que um bloco). O caminho nunca identifica nada. **Dedup:** um asset por conteúdo por projeto (índice único no banco): (A) mesmo arquivo/mesmo path → `existing`, sem escrita; (B) mesmo conteúdo/path novo → `aliased` (alias em `known_paths`) ou, se o asset estava offline/modificado, `relinked` automático; (C) conteúdo diferente/mesmo nome → outro asset; (D) arquivo alterado depois do import → `verify` detecta `modified` (por hash; tamanho diferente já aparece sem hash; mtime nunca decide); (E) alias bom + path principal sobrescrito → o **alias vence** (`quick_status`/`verify_content` preferem o candidato com o tamanho/hash conhecido; determinístico, testado).
**Disponibilidade (ADR-048):** `online | offline | modified`, **derivada do disco** (nunca editada); abrir o projeto custa só `metadata` por asset e nunca falha por mídia ausente; os clips continuam no documento. **Relink** só aceita o **mesmo hash** (`ASSET_HASH_MISMATCH` estruturado com os dois hashes; nada muda); **não existe *force* na M07**. Relink/verify não são comandos do engine (não entram no undo) — ficam na trilha `asset_events`.
**Schema 2 / migration v1→v2:** tabelas `media_assets` (hash único, `json_valid`, `CHECK`s) e `asset_events` (append-only); aditiva, com backup `VACUUM INTO`. Testada sobre um projeto schema 1 **real** (histórico, ramo de redo e `operation_id`s intactos; undo/redo funcionam depois), com falha no meio (rollback), cadeia v1→v2→v3 sintética, schema futuro rejeitado sem tocar o arquivo e **queda de processo no meio da migration** (o arquivo continua v1 íntegro).
**Import atômico:** hash → probe → classificação **antes** de qualquer escrita; o efeito no catálogo viaja na MESMA transação SQLite do `register_asset` (fila drenada pelo `Journal`). Probe/arquivo ruins não deixam nada; escritor obsoleto não deixa linha de catálogo; crash real em 5 estágios ⇒ tudo ou nada nas três camadas (documento, log de operações, catálogo).
**Mídia (ADR-047):** descoberta do backend: caminho configurado → `CAPIA_FFPROBE`/`CAPIA_FFMPEG` → diretório *bundled* (`<exe>/ffmpeg`) → `PATH`; caminho explícito inexistente é erro (não cai para o `PATH`); nada achado ⇒ `MEDIA_BACKEND_NOT_FOUND`. Processo sem shell, caminho como UM argumento `file:<abs>` após `-i`, `-protocol_whitelist file`, **timeout 30 s** (filho morto de verdade — teste confere que o pid some), **stdout ≤ 8 MiB e stderr ≤ 64 KiB** (flood ⇒ processo morto, `MEDIA_PROBE_OUTPUT_TOO_LARGE`), demuxers de playlist/rede rejeitados. Normalização: Ticks por aritmética inteira, `Rational`, streams ordenados, stream padrão explícito (primeiro vídeo que não é capa; primeiro áudio), limites de plausibilidade, rotação, alpha só quando declarado; valores hostis ⇒ `None`/erro estruturado, nunca pânico.
**Cache (ADR-049):** `<projeto>.capia-cache/`, `CacheKey` = SHA-256 de (hash de conteúdo, operação, parâmetros ordenados, produtor+versão do ffmpeg), escrita atômica, **descartável** (apagar não altera o `.capia`; `validate` segue ok; a miniatura regenera). Miniatura: `generate_thumbnail(asset, timestamp)` (PNG, ≤ 1024 px); recusa arquivo com hash diferente do catálogo.
**CLI:** `capia asset import|list|inspect|verify|relink|thumbnail` e `capia media probe` (`--json`, `--ffprobe`, `--ffmpeg`, `--timeout-ms`); erros estruturados, saída 0/1/2.
**Composição (ADR-050):** `duplicate_sequence` (raso ou `deep`), `make_unique`, `flatten_nested`, `create_nested_from_selection`, `generate_variants` (estrutural, sem IA) e `delete_asset` (`IN_USE`). Ids derivados (`S'.X` / `R'~D`), atômicos, com undo/redo/idempotência/persistência; ciclo e profundidade guardados em duas camadas.

## Matriz de requisitos da M07

| Requisito | Estado | Teste que prova |
|---|---|---|
| `capia-media` (probe, normalização) | ✅ | `capia-media`: 21 unit + 14 `real.rs` (goldens), CLI `media_probe_*` |
| `capia-assets` | ✅ | `capia-assets`: 8 unit + 10 `assets.rs` + 2 `real_media.rs` |
| ffprobe abstraído (`MediaProbe`) | ✅ | `FfprobeBackend`, `fake_backend` (travado/flood/JSON lixo/backend ausente), `StaticProbe` nos testes de lógica |
| Descoberta do backend / `MEDIA_BACKEND_NOT_FOUND` | ✅ | `toolchain::tests`, CLI `missing_backend_is_a_structured_error_not_a_panic` |
| Timeout, filho morto, teto de stdout/stderr, sem shell | ✅ | `process::tests` (Unix **e** Windows), `the_child_is_really_gone_after_a_timeout`, `arguments_are_never_interpreted_by_a_shell`, `probe_args` |
| Fixtures reais | ✅ | `tests/fixtures/media` (10 arquivos desde a M08: + `cfr_gop.mp4`, `vfr.mp4`, `tone_44k.wav`; geradas por `tools/gen-media-fixtures.sh`, determinísticas) |
| Hash em streaming | ✅ | `never_asks_for_more_than_one_block`, `streaming_across_block_boundaries…`, 48 MiB, mutação M7 |
| Deduplicação | ✅ | `identity_is_content_not_path_or_name`, `dedup_by_content…`, índice único |
| Import atômico | ✅ | `failed_imports_leave_nothing_behind`, `a_stale_writers_import_leaves_no_trace…`, crash `killing_an_import_at_every_stage…`, `a_failing_catalog_effect_rolls_back…` |
| Schema 2 + migration v1→v2 real | ✅ | `schema.rs` (cadeia v1→v2→v3, histórico/redo/`operation_id`/undo pós-migration), crash `…in_the_middle_of_a_migration…` |
| Assets persistidos | ✅ | `identity_survives_close_and_reopen`, propriedade com reabertura |
| Offline | ✅ | `offline_media_does_not_prevent_opening_and_keeps_the_clips` |
| Verify / modified | ✅ | `verify_detects_modified_content_not_just_missing_files`, `a_good_alias_beats_an_overwritten_primary_path` |
| Relink (ok e rejeitado) | ✅ | `relink_by_content_and_structured_rejection_of_other_content` + CLI |
| Cache / thumbnail | ✅ | `cache_keys_are_deterministic…`, `thumbnail_goes_to_the_cache…` (apagar cache → `validate` ok → regenera) |
| Clips → `AssetId`; referência inválida; asset em uso | ✅ | `clips_reference_assets_and_in_use_assets_cannot_be_deleted` |
| CLI de assets | ✅ | `capia-cli/tests/assets.rs` (6) |
| Nested restante (5 comandos) | ✅ | `compose.rs` (24) + propriedade |
| Propriedade (assets + persistência + composição) | ✅ | `properties_assets.rs` (modelo de referência), `properties` do store com composição |
| Performance 10k assets | ✅ | `perf_assets.rs` (números abaixo) |
| Linux / Windows / WASM | ver relatório de entrega (run final) | job Linux inclui WASM e paridade |
| Imagem: width/height/format/alpha | ✅ | `images()` (PNG com alfa, JPEG) |
| *Force-relink* | ⏳ **adiado** (o enunciado o deixa para depois) | — |
| Rotação em fixture real | ⏳ coberta só por JSON sintético (o ffmpeg não grava Display Matrix de forma portável) | `rotation_from_display_matrix_is_clockwise_and_validated` |

## Validação M07 (container Linux, Rust 1.97, ffmpeg/ffprobe 6.1.1-3ubuntu5)

| Verificação | Resultado |
|---|---|
| `cargo fmt --check` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` | ✅ · ✅ · ✅ **267 testes** (+ 5 de desempenho `#[ignore]`; M06 tinha 163) |
| Propriedade com IO (release, 5.000 casos) | ✅ timeline: 70.000 comandos (41.763 aceitos), 21.374 reaberturas · nested+composição: 70.000 (42.791), 21.401 reaberturas · **assets: 5.000 × 40 passos** (7.456 clips, 10.518 imports criados, 4.624 `existing`, 1.187 `aliased`, 356 relinks automáticos, 1.495 reregistros, 1.991 relinks ok × 3.691 rejeitados, 1.274 `modified`, 3.462 composições, 4.851 undo/redo, **5.949 reaberturas**) — 136 s |
| Propriedade nested em memória | ✅ 3.000 casos, ≈ 24.8 mil comandos aceitos, 4.914 edições nested, **4.755 comandos de composição** |
| Mutação manual (22) | ✅ 19 detectadas: relink sem hash, `AssetId` inexistente aceito, offline ignorado, `make_unique` reaproveitando ids, timeout removido, hash em memória, verify só por tamanho, **catálogo fora da transação**, dedup que nunca acha, `flatten` ignorando a janela, `delete_asset` ignorando uso, prefixo `file:` removido, saída sem teto, **probe falho ignorado (asset sem probe)**, **cache ignorando o hash (cache como permanente)**, **modified reportado como online**, **migration parcial / sem `user_version`**, relink que aceita qualquer arquivo. As 3 restantes são a remoção de **uma** camada de proteção de ciclo/profundidade — sobrevivem *por desenho* (defesa em profundidade); removendo as **duas** camadas juntas os testes de `compose`, `nested` e `nested_graph` falham |
| M05/M06 preservados | ✅ aceitação 120/120, golden digest estável, paridade WASM (150 sequências), `cargo check wasm32-unknown-unknown`, crash tests |
| `pnpm lint/format:check/typecheck/test/test:tools`, `check:arch`, `check:parity`, `cargo deny` | ✅ |

### Desempenho (release, `synchronous=FULL`, Linux, VM sem SHA-NI)

| Medida | Resultado |
|---|---|
| Hash SHA-256 streaming, 256 MiB | 2,6 s ≈ **97 MiB/s** (outlier: esta VM não tem SHA-NI; com aceleração de hardware espera-se ≥ 10× — ver limitações) |
| Hash da fixture de 7,5 KiB | 0,06 ms |
| `ffprobe` da fixture (mediana de 20) | **62 ms** (custo dominante do import) |
| Import completo (hash + ffprobe + commit atômico, mediana de 10) | **68 ms** |
| Miniatura (cache frio / quente) | **83 ms** / 0,02 ms |
| 10.000 assets: 10k `register_asset` num commit durável / 10k linhas de catálogo + eventos | 309 ms / 764 ms |
| Arquivo `.capia` com 10.000 assets | 20,4 MiB |
| **Abrir** (documento + catálogo) / reabrir após import | **299 ms** (meta < 2 s) / 308 ms |
| **Listar** 10k (leitura do catálogo + 10k `stat`) | **207 ms** |
| Lookup por `AssetId` / por hash (média de 100) | 0,05 ms / 0,07 ms |
| `verify` de um asset (stat + hash + evento) | 2,4 ms |
| Import num projeto de 10k assets | **6,5 ms** (meta < 200 ms) |
| Persistência de 10k clips (M06, mesma máquina) | abrir 74 ms · commit comum mediana 9 ms · commit com snapshot 135 ms |

### Estratégia de CI e tempo do Windows (ADR-051)

O `cargo test --workspace` do Windows levava ≈ **17,7 min** na M06 (propriedades com fsync no NTFS). Na M07: **propriedades com IO** leem `CAPIA_IO_PROP_CASES` — Windows usa um orçamento *smoke* (100 casos), o Linux roda o orçamento pesado em **release** (5.000 por gerador, ≈ 6 min no total) mais os benchmarks. O Windows continua rodando integralmente: crash tests reais (incl. migration e import), store, schema/migrations, mídia/assets com FFmpeg instalado via Chocolatey (versão impressa no log; `CAPIA_REQUIRE_FFMPEG=1`), CLI, lógica pura e o build desktop. Os tempos do último run estão no relatório de entrega.

### Achados da auditoria M07 (corrigidos)

- **Fixtures de mídia não estavam no Git** (`.gitignore` bloqueava `*.mp4`/`*.wav`): o CI vermelho revelou; exceção deliberada para `tests/fixtures/media/`.
- ETXTBSY ao executar um backend recém-escrito (corrida de `fork`): retry limitado.
- `join` das threads de leitura travava até o neto do processo fechar o pipe: leitores desacoplados com espera curta.
- `quick_status`/`verify_content` olhavam só o 1º candidato: um alias íntegro não valia nada se o principal fosse sobrescrito → agora prefere o candidato com o tamanho/hash conhecido.
- `Project::asset` montava a lista inteira (O(n)) → leitura direta (O(1), 0,05 ms em 10k).
- Testes só existiam para Unix (`/bin/sh`): adicionados equivalentes Windows para processos e caminhos (drive letters, Unicode, separadores).
- `file:`-prefix e `-protocol_whitelist`: protegidos por teste de argumentos (o caminho absoluto já impede `concat:` relativo; é defesa em profundidade).

### Limitações restantes (M07)

- **Hash completo síncrono no import:** 97 MiB/s nesta VM significa ≈ 100 s para um vídeo de 10 GB sem aceleração SHA-NI; o desenho de `ASSET_SYSTEM.md` (fingerprint amostrado rápido + hash completo em job) fica para a missão de jobs.
- Sem *force-relink*, sem relink em lote por pasta, sem estado `Missing`; `known_paths` guarda texto (caminhos não-UTF-8 viram *lossy* nos aliases).
- `asset_events` cresce a cada `verify` (sem poda).
- O cache de miniatura é chaveado pelo hash do **catálogo**: se o arquivo mudar sem `verify`, um acerto de cache devolve a miniatura do conteúdo original (a geração nova recusa hash diferente).
- Rotação/Display Matrix só testada com JSON sintético; golden de metadados compara campos estáveis entre versões do FFmpeg (a versão do CI é registrada).
- Frame index/decode, proxies, waveform, jobs, render, preview e export continuam fora de escopo.

## O que existe (M06)

**Formato `.capia` v1** (ADR-042): um arquivo SQLite (tabelas `STRICT`): `meta`, `schema_migrations`, `history_entries`, `events`, `applied_operations`, `commit_results`, `snapshots`; triggers *append-only*; `application_id = 0x43415049`. O documento **não** é espalhado em tabelas: estado = último snapshot (JSON canônico + digest) + replay dos eventos posteriores (ADR-043: histórico completo persistido, undo/redo funcionam após reabrir).
**Commit** (ADR-044): entrada de histórico + resultado + `operation_id`s + evento (+ snapshot no cadence) numa única `BEGIN IMMEDIATE`; o engine persiste **antes** de publicar (falha ⇒ memória intacta, `PERSISTENCE_FAILED`). Escritor único pelo lock do SQLite + checagem de *stale head* (`STORE_BUSY` / `STORE_CONFLICT`).
**PRAGMAs:** WAL · `synchronous=FULL` (padrão) · `foreign_keys=ON` · `busy_timeout=5000` · `trusted_schema=OFF` · `cell_size_check=ON` · `quick_check` ao abrir, `integrity_check`+`foreign_key_check` em `validate`.
**Migrations:** só para frente, cada uma em transação própria, backup `VACUUM INTO` (`<arq>.v<N>.bak`) antes de migrar; schema mais novo é **rejeitado sem tocar o arquivo**. Testadas com versão 2 sintética (sucesso, falha com rollback, lacuna, downgrade).
**Erros estruturados:** `PROJECT_NOT_FOUND` · `PROJECT_ALREADY_EXISTS` · `NOT_A_CAPIA_PROJECT` · `PROJECT_CORRUPTED` · `UNSUPPORTED_SCHEMA_VERSION` · `MIGRATION_FAILED` · `STORE_IO_ERROR` · `STORE_BUSY` · `STORE_CONFLICT` · `TRANSACTION_FAILED` · `INVALID_ARGUMENT`. Nenhum erro bruto do SQLite escapa.
**CLI** (`capia`): `create` · `inspect` · `validate` · `apply` · `undo` · `redo` · `history` · `dump` (JSON determinístico). Usa o mesmo parser (`capia_project::parse_transaction`) e o mesmo engine; atores `agent`/`api` passam por `preview → apply_plan`. Saída 0 ok · 1 falha (JSON estruturado em stderr) · 2 uso.
**Nested (ADR-045):** `delete_sequence` · `rename_sequence` · `insert_nested` · `set_nested_target` · `set_follow_length`; ciclo (A→A, A→B→A), profundidade máx. 16 e referência inválida rejeitados; `follow_length` reconciliado após cada comando (magnética: ripple; livre: limitado ao espaço com aviso; travada: intocada com aviso).

## Validação M06 (container Linux, Rust 1.97)

| Verificação | Resultado |
|---|---|
| `cargo fmt` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` | ✅ · ✅ · ✅ **163 testes** (time 25 · model 20+5+4 · commands 9+21+1+18+2+2+2 (+1 acc.+1 golden) · store 11 schema + 18 store + 5 crash + 2 properties · cli 12+4 · project 1 …) + 4 de desempenho `#[ignore]` |
| Crash real (`tests/crash.rs`) | ✅ filho morto por `abort` e por `kill` externo em `before_begin` / `in_tx_after_entry` / `in_tx_after_writes` / `before_commit` / `after_commit` (10 combinações): sempre **estado A completo ou B completo**, `integrity_check` ok, log coerente, reenvio idempotente; + 14 rodadas de SIGKILL em instantes aleatórios com snapshots; repetido 15× sem flakiness |
| Propriedade (lockstep persistido × memória, reabrindo aleatoriamente) | ✅ 1.000 sequências por gerador (timeline e nested) por padrão (~90 s em debug por ser I/O com fsync); **CI release: 5.000 por gerador (10.000 no total, ~147 s)**. Rodada medida: 21.000 comandos / 6.395 reaberturas (timeline), 21.000 / 6.368 (nested). Orçamento escolhido por custo de I/O; `CAPIA_PROP_CASES` ajusta |
| Propriedade nested em memória | ✅ 3.000 casos, 25.586 comandos aceitos, 6.103 edições nested |
| Corrupção (`tests/schema.rs`) | ✅ vazio, não-SQLite, SQLite sem schema, schema novo, blob inválido, registro inconsistente, ids inválidos, 300 rodadas de *byte-fuzz*: sempre erro estruturado, **nunca pânico, arquivo intocado** |
| Concorrência | ✅ 2 escritores: `STORE_BUSY`/`STORE_CONFLICT`; threads com retry nunca corrompem e cada escrita entra exatamente uma vez |
| Desempenho (`--release`, 10.000 clips, `synchronous=FULL`, arquivo 2,4 MiB) | ✅ criar 238 ms · **abrir 81 ms** (meta < 2 s) · recarregar 68 ms · **commit comum mediana 7,2 ms, pior 10 ms** (meta < 200 ms) · commit com snapshot de 10 k clips 118 ms · abrir com cauda de 300 eventos 83 ms |
| Mutação manual (11) | ✅ todas detectadas: `operation_id` duplicado ignorado, erro de insert engolido, evento fora da transação, validação de ciclo/profundidade removida, stale-head removido (defesa em profundidade: a PK também barra), digest do snapshot sem checagem, publicar antes do journal, ids não persistidos, cursor de undo não restaurado, `follow_length` desligado, `synchronous` enfraquecido |
| M05 preservado | ✅ aceitação 120/120, golden digest estável, paridade WASM, `cargo check wasm32-unknown-unknown` |
| CI no GitHub (Linux + Windows) | ver relatório final |

**Achados da auditoria (corrigidos):** carga inconsistente (snapshot e eventos de instantes diferentes) → leitura em transação única; *peek* criava sidecars `-wal/-shm` → leitura crua do cabeçalho; `applied_operations` sem resultado não era detectado → cruzamento em `load`; aritmética sem *checked* sobre revisão/seq vindas do arquivo → `checked_add` e *guards*; blobs gigantes de arquivo hostil → `max_blob_bytes`.

### Limitações restantes (M06)

- Histórico completo e todos os `operation_id` são carregados em memória ao abrir (sem retenção/poda).
- O limite de 512 MiB por blob só foi testado com limite configurado pequeno.
- Custo de fsync por commit (`FULL`); `Normal`/`Off` só para testes de volume.
- Se `tx.commit()` falhar *depois* de o banco ter commitado, a memória fica atrasada; a reabertura recupera (`STORE_CONFLICT`).
- Planos (`preview`) não são persistidos (por desenho).
- Mover clips para track magnética continua `UNSUPPORTED_COMMAND`; permissões por ator continuam adiadas.
- Crash tests foram executados em Linux localmente; Windows roda na CI.

## O que existe (M05)

**Comandos (v1):** `register_asset` · `create_sequence` · `add_track` · `set_track_flags` · `delete_track` · `add_marker` · `move_marker` · `delete_marker` · `insert_clip` · `move_clips` (estrito) · `delete_clip` · `trim_clip` · `split_clip` · `set_clip_speed` · `set_property` · `add_keyframe` · `move_keyframe` · `delete_keyframe` · `set_keyframe_interp`.
**Funções puras de UX/engine:** `resolve_placement` · `resolve_snap` · `threshold_ticks` · `resolve_group_move` · `eval_property`.
**Garantias testadas:** toda escrita passa pelo Engine; `Agent`/`Api` só por preview (`PREVIEW_REQUIRED`); erro ⇒ documento idêntico; `apply∘undo = id`, `undo∘redo = id`; replays nunca duplicam edições; token adulterado/expirado/de outro ator/outra chave é rejeitado; plano só aplica se o `diff_digest` recomputado for idêntico.

### Ainda NÃO implementado (escopo declarado fora da M05)

- Comandos: pastas/organização, `duplicate_sequence`, `reorder_*`, `duplicate_clip`, `replace_clip_media`, `freeze_frame`, `detach_audio`, nested restante (`create_nested_from_selection`, `make_unique`, `flatten_nested`, `generate_variants`), efeitos, transições, texto/legendas, grupos de clips, clipboard, deliverables, comandos de assets além de `register_asset`.
- Mover clips **para/de track magnética** (semântica de *reorder*, Fase 3) → `UNSUPPORTED_COMMAND`.
- Caminhos de ref (`$seq.tracks.main`); só `$nome` simples.
- `permissions` por ator (`PERMISSION_DENIED`; Fase 4) e gate humano de aprovação do plano (`plan_id + diff_digest`).
- Coalescing por `gesture_id`; undo seletivo por ator (Fase 5); `Sequence.revision` individual.
- Conflitos em nível de campo (hoje por entidade — ADR-040).

## Missões

| Missão | Resultado |
|---|---|
| M01 — Fundação arquitetural | ✅ docs + `.gitignore` |
| M02 — Auditoria open-source | ✅ `OPEN_SOURCE_AUDIT.md`; estratégia C |
| M03 — Fechamento e spikes S1–S7 | ✅ S2–S7 medidos; ADR-029..036; `PROVENANCE.md`; `tests/acceptance` (108 cenários) |
| M04 — Scaffold, CI e preparação | ✅ workspace Rust+TS, shell Tauri, CI, licenças/arquitetura, pacote S1, ADR-037/038 |
| **M05 — Core Engine Foundation (Fase 2)** | ✅ tempo, modelo, Command Engine, suíte de aceitação 120/120, propriedade 10.000, idempotência/plano, paridade WASM, ADR-039..041 |
| **M06 — Persistência, nested e CLI (Fase 2)** | ✅ `capia-store`, `capia-project`, `capia-cli`, comandos nested, crash tests reais, propriedade com salvar/reabrir, ADR-042..045 |
| **M07 — Assets, mídia e composição (Fase 2)** | ✅ `capia-media`, `capia-assets`, catálogo no `.capia` (schema 2), import atômico, offline/verify/relink, cache + miniatura, CLI de assets, 5 comandos de composição, ADR-046..051 |
| **M08 — Jobs e pipeline de mídia (Fase 2)** | ✅ `capia-jobs`, import não bloqueante, impressão rápida, índice de quadros, decode exato, áudio PCM, waveform, proxy, cache v2, force/lote relink, schema 3, crash/propriedade/mutação, CLI, ADR-052..058 |

## Validação M05 (saída real; container Linux, Rust 1.97.0, Node 22, pnpm 10.28)

| Verificação | Resultado |
|---|---|
| `cargo fmt --all -- --check` · `cargo clippy --workspace --all-targets -- -D warnings` | ✅ · ✅ sem avisos |
| `cargo test --workspace` | ✅ **84 testes** (time 25 · model 20 · commands 36 · project 2 · desktop 1) + 2 de desempenho `#[ignore]` |
| Aceitação (`tests/acceptance.rs`) | ✅ **120 cenários** (placement 19 · snapping 19 · ripple 25 · retime 19 · group move 15 · keyframes 23) |
| Mutação na suíte (3 defeitos injetados no engine: limiar de snap exclusivo, obstáculo inclusivo no group move, arredondamento por truncamento) | ✅ 3/3 detectados (1, 1 e 3 cenários falham) |
| Propriedade (`tests/properties.rs`) | ✅ **10.000 sequências × 8 comandos** (≈ 50 mil comandos aceitos: insert 19,4 k · trim 4,8 k · delete 4,6 k · keyframe 3,7 k · split 2,7 k · speed 2,4 k · move 0,5 k …); determinismo de replay em 300 sequências × 12 comandos |
| Engine (`tests/engine.rs`) | ✅ 21 testes: replay, `OPERATION_ID_REUSED/CONFLICT`, undo não libera ids, `PREVIEW_REQUIRED`, token adulterado/expirado/outro ator/outra chave/store limitado, drift (rebase seguro × `PLAN_STATE_CHANGED`), `CONFLICT` por `base_revision`, refs, `max_ops`, histórico, `RIPPLE_CONFLICT` estruturado, track travada |
| Robustez (`tests/robustness.rs`) | ✅ ~900 comandos com valores extremos (limites de i64/f64, NaN, ±∞, parâmetros de Bézier) e JSON malformado: só erro estruturado ou commit válido e desfazível; **achado e corrigido na revisão:** overflow de `i128` (pânico em debug / wrap em release) em `mul_div_round`, soma/subtração de `Rational`, escala de frames e `threshold_ticks` — agora aritmética *checked* |
| SHA-256/HMAC próprios | ✅ vetores FIPS 180-4 (incl. 1 M de `a`) e RFC 4231 (casos 1, 2, 6) |
| Desempenho (`--release`, modelo em memória) | ✅ transação de **500 ops em 10.000 clips ≈ 16 ms** (meta < 200 ms) · carregar + indexar + validar 10.000 clips ≈ **17 ms** (meta < 2 s) |
| Paridade nativo × WASM (`pnpm check:parity`) | ✅ 150 sequências aleatórias: digests do documento **idênticos** entre nativo e `wasm32-wasip1` (WASI do Node) — cobre floats de Bézier e JSON |
| Golden digest (`tests/golden.rs`) | ✅ trava a semântica do engine (25 sequências) |
| WASM: `cargo check --target wasm32-unknown-unknown -p capia-time -p capia-model -p capia-commands` | ✅ |
| `pnpm lint` · `format:check` · `test:tools` (15) · `check:arch` · `cargo deny check licenses bans sources` | ✅ ✅ ✅ ✅ ✅ |
| CI no GitHub (runner real) | `main`: M04 falhou **uma vez** no `gitleaks-action` (range inválido com commit raiz) → corrigido (CLI sobre o histórico completo) → **verde**. Branch `claude/m05-core-engine`: jobs Linux (núcleo + perf + paridade, TypeScript, políticas/segredos) ✅ — resultado do job Windows no relatório final |

### Limites do que foi validado

- A suíte de aceitação foi **escrita à mão a partir da especificação**, não gerada pelo engine; porém os 12 cenários novos e a atualização de 4 são da M05 e foram escritos com o mesmo método (valores derivados à mão — ver `provenance`).
- Conflitos de plano são detectados por **recomputação + digest**; não há teste de concorrência real (o engine é *single-writer* por construção; o ator de projeto vem com o `capia-store`/`capia-engine`).
- Desempenho medido só no **modelo em memória**; "abrir projeto" com SQLite será medido com o `capia-store`.
- Paridade WASM por **digest do documento**, não por execução no navegador (WebView/V8); o WASI do Node usa o mesmo motor V8.
- Teste de *crash* (kill no meio do commit) **não** existe ainda (não há persistência).

## OUTPUT-H264 — risco técnico explícito (NÃO resolvido)

Antes da **entrega do Editor** (saída da Fase 3) é preciso um caminho **confiável** de exportação **MP4/H.264 no Windows**:

- **Encoder de hardware** quando disponível (NVENC/AMF/QSV);
- **Media Foundation / FFmpeg** quando adequado (`h264_mf`, builds LGPL — ADR-032);
- **Fallback por software legal e de qualidade aceitável**. Sem x264/x265 GPL no build padrão.

*Sabido (S3):* as builds LGPL trazem NVENC/AMF/QSV/MF; o fallback por software disponível é OpenH264 (**só Constrained Baseline**; PSNR 46,9 dB vs 53,0 dB do x264 no mesmo bitrate em conteúdo **sintético**); o binário OpenH264 compilado do fonte não tem a cobertura de patentes da Cisco.
*Desconhecido:* qualidade/velocidade em vídeo real; disponibilidade de encoder de hardware na máquina mínima (iGPU); comportamento do Media Foundation; política de download/licença do binário OpenH264; patentes H.264/HEVC/AAC (decisão jurídica/comercial).
*Responsável:* Product Owner (com apoio jurídico para patentes). *Sugestão:* spike dedicado em Windows (mesma sessão do S1). *Estado:* **ABERTO**.

## Próxima missão da M08 (histórico; superada pela conclusão da Fase 2)

**M09 — Serviço de decode e fundação do render (headless):** (1) **serviço de decode** sobre `FrameSource`: pool de processos/decoder persistente, cache de quadros (LRU) e *prefetch* por intervalo (`decode_frame_range`), metas de latência de scrub; (2) **áudio com seek** (índice de pacotes de áudio, seek com margem e conferência de alinhamento) para tirar o custo O(início); (3) `capia-render` mínimo: *render graph* a partir da timeline (clips de mídia/imagem/texto/sólido), compositor **CPU de referência** headless e *golden frames* (base para o wgpu da Fase 3); (4) **OUTPUT-H264**: spike do caminho de export (encoders de hardware + OpenH264) atrás da abstração, medindo em Windows. Não iniciar a Fase 3 antes de OD-1.

## Blockers

1. **S1 em Windows** — gate da Fase 3 (não bloqueia a Fase 2).
2. **OUTPUT-H264** — antes da entrega do Editor.
3. *(resolvido na M05)* Push ao GitHub (403) e primeira execução do CI.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs da missão.
- Nenhum código de terceiros incorporado (`docs/PROVENANCE.md` §4 vazio). SHA-256/HMAC foram escritos do zero a partir de FIPS 180-4/RFC 2104/4231.
- Ao concluir uma missão, atualize esta página.
