# Plano da Fase 2 — adoção do OpenCut classic (`ADOPT_OPENCUT_CLASSIC_WITH_MAJOR_CHANGES`)

> **Não executar ainda.** Evidência e justificativa: [`opencut-classic-validation.md`](opencut-classic-validation.md). Baseline: `OpenCut-app/opencut-classic@cf5e79e9` (MIT, arquivado).
> Cada passo tem KEEP / MODIFY / REMOVE / ADD e um **gate de conclusão** objetivo. "Kill criteria" no fim: se falharem sem correção viável, reavaliar a decisão.

## STEP 1 — Congelar o CapIA legado
- **KEEP:** todas as branches `claude/*` e `research/*` (nada é apagado). **MODIFY:** criar `legacy/core-from-scratch` a partir de `claude/rc3-free-timeline-ux` (tag `legacy-core-rc3`). **REMOVE:** nada. **ADD:** `docs/LEGACY.md` (o que cada crate fazia, o que foi reaproveitado, ponteiros de ADR).
- **Gate:** `git branch -a` mostra `legacy/core-from-scratch`; CI do legado ainda verde; `main` só muda por PR da Fase 2.

## STEP 2 — Estabelecer o upstream OpenCut
- **KEEP:** histórico completo do upstream (remote `upstream` = `OpenCut-app/opencut-classic`, tag `upstream-cf5e79e`). **ADD:** `docs/PROVENANCE.md` (entrada MIT, SHA, data) e `THIRD_PARTY_NOTICES`. **REMOVE:** nada ainda.
- **Gate:** `git merge-base` com o upstream resolve; entrada de proveniência revisada.

## STEP 3 — Criar `integration/opencut-classic`
- **ADD:** branch com `origin` = CapIA; árvore do upstream em `apps/editor` (workspace bun) ao lado de `crates/`; **aplicar** `evidence/opencut-classic/patches/upstream-validation-fixes.patch` (5 remendos) como commits separados e atribuídos; harness de teste Bun (`wasm-preload`) **ou** migrar testes para `vitest`+Node (decisão); CI: install + `tsc` + `bun test` + `next build` (ou build estático) + `build:wasm`. **MODIFY:** `tools/check-architecture.mjs` (nova matriz). **REMOVE:** nada.
- **Gate:** CI verde com 0 falhas conhecidas escondidas — as 4 falhas atuais **corrigidas** (teste obsoleto de `mediaTime(2.5)`, 2 de máscara, 1 de canvas via `OffscreenCanvas`/mock explícito) ou documentadas; `tsc` limpo; build reproduzível em runner limpo.

## STEP 4 — Limpeza de licenciamento
- **REMOVE:** `botid`, `sharp`/otimização de imagem do Next, Freesound, Databuddy, Marble CMS, Upstash, better-auth, Drizzle/Postgres, páginas `blog/changelog/sponsors/contributors/roadmap`. **MODIFY:** `soundtouchjs` (LGPL-2.1) → alternativa permissiva **ou** manter com relink documentado; `mediabunny` (MPL-2.0) → manter, registrar arquivo-a-arquivo; fontes → empacotar famílias auditadas (OFL/Apache) e remover CDN; modelos Whisper → registrar licença por modelo, desligados por padrão. **ADD:** `cargo-deny`/`check-licenses` para o workspace bun; SBOM.
- **Gate:** `check-licenses` sem GPL/AGPL/NC/UNKNOWN; relatório de obrigações MPL/LGPL assinado; nenhum acesso de rede no editor sem ação do usuário.

## STEP 5 — Remover serviços indesejados
- **REMOVE:** rotas `app/api/*`, `env/web.ts` obrigatório, landing, auth, DB. **MODIFY:** `next` → build **estático** (ou Vite) sem servidor; **KEEP:** editor, projeto, export.
- **Gate:** o editor abre e exporta com a rede **desligada** (princípio 2); bundle sem servidor Node.

## STEP 6 — Criar a costura do adapter (`AgentSurface` + `EnginePort`)
- **ADD:** `AgentSurface` (produtizar `capia-spike-index.ts`): `inspect_*` DTO versionado, `validate/preview/apply`, `baseRevision` (CAS), `operationId` derivado, `AtomicBatch`, recibo de undo, ator/planId nos itens do histórico. **MODIFY:** comandos sem `generateUUID` interno (injetar `idFactory`), `execute` com erro estruturado, `BatchCommand` atômico, comando sem efeito **não** entra no histórico, histórico com metadados. **ADD:** trait Rust `EnginePort` (extraída de `capia-editor-api::Session`) + `capia-engine-bridge` (IPC Tauri / sidecar Bun). **KEEP:** `EditorCore`/managers.
- **Gate:** suíte de contrato (stale revision, idempotência, atomicidade, preview sem efeito, 1 plano = 1 undo, paridade humano↔programa) roda em **CI** (Bun headless **e** Playwright); nenhuma regra de timeline duplicada (verificado por teste de arquitetura: o adapter só importa commands/managers).

## STEP 7 — Migrar/rehospedar a inteligência
- **KEEP:** `capia-secrets`, `capia-ai` (providers/router/registry/Brain/Replay/SafeFetcher), `capia-jobs`, `capia-media`/`capia-assets` (análise), `capia-store` tabelas de IA. **MODIFY:** `capia-intelligence` para depender de `EnginePort` + DTOs em vez de `capia-model`/`Session`; Editor Agent (compilador determinístico) emite o catálogo de operações do `AgentSurface`; Critic usa quadros do compositor do editor (snapshot) via bridge. **REMOVE:** dependências de `capia-model/commands/project/render` na inteligência. **ADD:** projeção de projeto OpenCut → `DemandSpec/EditPlan` inputs.
- **Gate:** suítes Replay de autonomia (`autonomy_*`, sem chave) passam sobre o **OpenCut real**; uma AI Run de ponta a ponta (Brief→Plan→Edit→Critic→undo) opera a timeline vista na UI.

## STEP 8 — Projeto e sequências
- **MODIFY:** `Scene → Sequence` (UI criar/duplicar/renomear/ordenar/apagar, abas, view-state por sequência, export por sequência/lote); persistência em **`.capia`** (SQLite no host Tauri; leitura/escrita via IPC; migrações do upstream preservadas; mídia por referência + hash + relink; histórico persistido; journal). **ADD:** `DuplicateSequenceCommand`, migração v32 aditiva, importação/exportação de pacote. **REMOVE:** dependência de OPFS/IndexedDB como fonte de verdade (fica como cache).
- **Gate:** CREATE→EDIT→SAVE→CLOSE→REOPEN→EDIT→SAVE com kill -9 no meio sem perda além do último commit; projeto com `AD1…AD3, HOOK_1, BODY_MASTER, CTA` abre, edita e exporta cada sequência.

## STEP 9 — Sequências aninhadas
- **ADD:** elemento `sequence` (união `TimelineElement`), `SequenceNode` recursivo no `scene-builder`, mixagem de áudio recursiva, revisão por sequência + invalidação transitiva de cache, detecção de ciclo, make-unique, flatten, UI (clip-sequência, abrir, breadcrumb), migração v32/v33. **KEEP:** compositor e pool de texturas.
- **Gate:** `AD1 = HOOK_01 + BODY_MASTER + CTA_01` e `AD2 = HOOK_02 + BODY_MASTER + CTA_01`; editar `BODY_MASTER` atualiza ambos em preview e export (frames amostrados); ciclo rejeitado; flatten ≡ render original (diff de quadros ≤ tolerância).

## STEP 10 — Superfície de comandos da IA
- **ADD:** reapontar `capia-server` (REST/SSE/MCP/webhooks, escopos, rate limit) ao `AgentSurface`; catálogo fechado de operações; ator `Agent("run:<id>")`; aprovações presas ao digest do plano. **KEEP:** auth/escopos/webhooks do `capia-server`. **REMOVE:** operações atreladas ao modelo Rust antigo.
- **Gate:** fluxo externo canônico (criar projeto → importar → planejar → aplicar → exportar) via REST **e** MCP sobre o editor OpenCut; suíte de paridade REST×MCP verde.

## STEP 11 — Empacotamento desktop
- **KEEP:** shell Tauri, NSIS, updater assinado, crash opt-in, diagnóstico (`apps/desktop`, `capia-updater`, `capia-support`). **MODIFY:** hospedar o build estático do editor + sidecar Rust (IA/mídia/`.capia`). **ADD:** FS nativo para mídia; gate de **H.264/AAC no WebView2**. **REMOVE:** GPUI/`apps/desktop` do upstream.
- **Gate:** instalador Windows limpo abre o editor, importa, edita, exporta **MP4 H.264+AAC** (decisão `OUTPUT-H264` fechada ou fallback aprovado) em GPU real.

## STEP 12 — Benchmark de aceitação
- **ADD:** timeline virtualizada (≥ 5 000 clips ≤ 100 ms/comando; 10 000 abre ≤ 3 s); transições (mínimo: crossfade/dissolve), reorder de tracks, grupos; testes de snapping reais; tarefa humana UGC (`tools/phase3-acceptance`) e demandas reais (`tools/phase5-acceptance`) — **nunca fabricar resultados**. **MODIFY:** lint (112 erros → 0).
- **Gate:** pacote de aceitação humana/GPU executado e validado; CI 100% verde no HEAD.

## Kill criteria (reavaliar a decisão se ocorrer)
1. H.264/AAC não exportável no WebView2 e sem alternativa aprovada → reabrir `OUTPUT-H264` e comparar com export via FFmpeg LGPL do CapIA legado (`HYBRID_REUSE_COMPONENTS`).
2. Compositor com divergência não corrigível em GPU real.
3. Virtualização da timeline não atinge a meta sem reescrever o domínio.
4. `EnginePort` exige reescrever os agentes (não só adaptar).

## Riscos a registrar em ADR
Princípio #5 (Rust-first → domínio TS headless-capaz), tick 120 000/s vs 705 600 000/s (áudio sample-exato), upstream arquivado (dono do fork), dependência de codec da plataforma.
