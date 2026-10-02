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

Missões:
1. ✅ **M01 — Arquitetura e documentação** (este conjunto de documentos).
2. **M02 — Decisões abertas + spikes técnicos** (código descartável em `/spikes`, relatórios em `docs/spikes/`):
   - S1: Preview presenter no Tauri 2/WebView2 (P1 superfície nativa vs P2 shared buffer) — `PREVIEW_RENDER.md` §5.
   - S2: Decode frame-exato com libav no Windows: seek exato, VFR de celular (iPhone/Android), HW decode D3D11VA, rotação, HDR→SDR.
   - S3: Build FFmpeg conforme licenciamento escolhido (OD-2) e encoders disponíveis (NVENC/QSV/AMF/OpenH264).
   - S4: `capia-time`/modelo mínimo compilado para WASM rodando na WebView; throughput de patches pelo IPC.
   - S5: Timeline canvas com 10.000 clips sintéticos (scroll/zoom/drag).
3. **M03 — Scaffold do repositório:** workspace Cargo + pnpm, crates vazios com regras de dependência verificadas (ex.: `cargo-deny`/check customizado), lint/format, CI no GitHub Actions (Windows), secret scanning, licença definida.

**Critérios de conclusão:**
- [ ] Todas as decisões abertas (OD-*) de `DECISIONS.md` resolvidas e registradas como ADR aceitos.
- [ ] Relatórios de S1–S5 com medições contra as metas declaradas; presenter escolhido.
- [ ] CI verde em Windows para workspace vazio (build, lint, testes vazios, secret scan).
- [ ] Nenhum ADR "Proposed" bloqueando a Fase 2.

---

## FASE 2 — Motor (headless, sem UI de produto)

**Objetivo:** todo o núcleo funcionando e testado via CLI/testes, sem interface.

Escopo: `capia-time`, `capia-model`, `capia-commands` (undo/redo, transações, dry-run, refs simbólicas, conflitos), `capia-store` (formato `.capia`, autosave, snapshots, backups, migrations, recovery), `capia-assets` (import, fingerprint, dedup, relink, offline, versões), `capia-media` (probe, frame index, decode, thumbnails, waveforms, proxies), `capia-jobs`, `capia-render` (render graph, compositor wgpu, texto, transições básicas, mixer, export H.264), `capia-preview` (scheduler + presenter escolhido, demo mínima), `capia-engine`, `capia-cli`.

**Critérios de conclusão:**
- [ ] Script via `capia-cli` cria projeto com 3 sequences (2 hooks + `BODY_MASTER` nested), transações, undo/redo, e exporta MP4 corretos (duração/fps/sync verificados por probe).
- [ ] Testes de propriedade do Command Engine (≥ 10.000 sequências aleatórias) sem violação de invariantes; `apply∘undo = id`.
- [ ] Teste de paridade preview×export bit-idêntico (sem proxy) no corpus de teste.
- [ ] Corpus VFR/29,97/23,976/59,94/44,1 kHz: drift A/V ≤ 1 frame em 10 min.
- [ ] Kill -9 durante transações e jobs: projeto reabre íntegro em 100% dos testes de crash.
- [ ] Benchmarks: transação de 500 ops < 200 ms em projeto de 10.000 clips; abrir projeto de 10.000 clips < 2 s.
- [ ] Migration de fixture v1→v2 sintética testada.

---

## FASE 3 — Editor (manual, sem IA)

**Objetivo:** editor utilizável de ponta a ponta sem IA, com fluidez próxima ao CapCut Desktop.

Escopo: shell UI + design system, `ui-timeline` (canvas), painel Project (árvore) + abas de sequence + `+`, biblioteca (projeto/global), drag-and-drop, trim/split/snapping/zoom/ripple/grupos/copy-paste, tracks (lock/mute/solo/hide/magnetic), nested (abrir, make unique, flatten, follow length), inspector, keyframes, texto, legendas manuais + estilos, transições, áudio (volume/fades/detach), preview com proxies, export de deliverables em lote, histórico visível, relink UI, atalhos configuráveis, pt-BR/en.

**Critérios de conclusão:**
- [ ] Editor experiente produz um UGC ad de 30–45 s (talking head + B-roll + texto + legendas + música + SFX) em ≤ 15 min, sem bugs bloqueantes (teste com ≥ 3 usuários).
- [ ] Metas de `TIMELINE_UX.md` §6 atendidas em hardware de referência (definido em OD-3).
- [ ] Todas as interações da UI produzem comandos (verificado: nenhuma escrita fora do Command Engine).
- [ ] Testes E2E dos fluxos principais verdes no CI Windows.

---

## FASE 4 — Inteligência (IA assistida, um passo por vez)

**Objetivo:** camada de IA configurável e segura; IA executa tarefas pontuais como transações.

Escopo: `capia-secrets` (Credential Manager), Provider abstraction (OpenAI-compatible, Anthropic, Google, local), Model Registry, probe, Brain Profile, Capability Router, Tool System com permissões/auditoria, transcrição (local + cloud), legendas automáticas, análise de mídia, **Reference Analyzer**, Demand Interpreter (DemandSpec), assistente de chat que executa pedidos pontuais ("adicione legendas", "corte silêncios") via transações, contabilidade de custo, provider de replay para testes.

**Critérios de conclusão:**
- [ ] Trocar o Brain entre ≥ 3 providers diferentes sem mudança de código; probe detecta capabilities.
- [ ] Teste canário: chave nunca aparece em logs/projeto/IPC/crash dumps.
- [ ] Reference Analyzer produz `ReferenceGrammar` em corpus de referência com erro de detecção de cortes ≤ 5% (vs. anotação manual).
- [ ] DemandSpec gerada de DOCX+PDF+vídeo com `sources` rastreáveis; avaliação manual em ≥ 10 briefs reais.
- [ ] Com todos os providers desligados, 100% das funções manuais funcionam.

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
