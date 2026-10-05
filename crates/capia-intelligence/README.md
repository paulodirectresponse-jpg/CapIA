# capia-intelligence

Pipelines e assistente da Fase 4 (ADR-082..086). Cliente do Engine API: lê por métodos de leitura (lista fechada) e escreve **só** por `preview → apply_plan` (ator `Agent`).

| Módulo | Entrega |
|---|---|
| `transcript` | STT por trechos (FLAC mono 16 kHz), costura em µs inteiros, cache por conteúdo+modelo |
| `captions` | legendas = clips de texto comuns em track `captions`; plano determinístico |
| `silence` | silêncio local (RMS 20 ms) → cortes alinhados ao frame **para dentro** do silêncio |
| `scenes` | detecção de cenas local; métrica `(FP+FN)/N ≤ 5 %`; corpus em `tests/scene_corpus.rs` |
| `reference` | `ReferenceGrammar` determinística (ritmo, transições, áudio, fala, hook/corpo/CTA) |
| `docs`, `demand` | extração segura DOCX/PDF/TXT/MD; `DemandSpec` com fontes **verificadas**; sem tools |
| `assistant`, `tools_exec` | chat pontual com tools, gate, aprovação (Ask/Auto), cancelamento, idempotência |
| `service` | superfície `ai.*` (config write-only, tarefas, eventos), hospedada por devserver/desktop |

Testes: `cargo test -p capia-intelligence` (FFmpeg real; `CAPIA_REQUIRE_FFMPEG=1` no CI). Aceitação: `tools/phase4-acceptance/`.

## Autonomia (Fase 5, ADR-087..101)

Módulo `autonomy/` (mesmo crate; o crate continua **cliente** do Engine API via `capia-editor-api` e só escreve por `preview → apply_plan` com o ator `run:<id>`). Rede só pelo `capia-ai` (providers e `SafeFetcher`); este crate orquestra.

| Módulo | Entrega |
|---|---|
| `machine` | `RunStage`/`RunStatus`/`Outcome` fortes; `transition()` é a **única** tabela (fechada); `RunErrorKind` |
| `model` | `AiRun`, `RunPolicy` (padrões seguros), orçamento/uso, decisões pendentes, aprovações presas a digest |
| `orchestrator` | cursor com CAS por `revision` (1 transação por transição), `drive/pause/resume/cancel/decide`, `recover()` (`Running → Paused`, nunca auto-resume), livro de efeitos e de orçamento, pasta de mídia durável `<projeto>-media/ai/` |
| `stages` | handlers UNDERSTAND · PLAN · VALIDATE_PLAN · ACQUIRE · EDIT · REVIEW · CORRECT (devolvem `Outcome`; quem decide é a tabela) |
| `plan` | `ProductionPlan`/`EditPlan` versionados, validação semântica, **compilador determinístico** `EditPlan → comandos` (tempo em ms inteiros → `Ticks`), `ValidationReport`, estratégias de variantes |
| `roles` | Producer, Planner e Critic semântico: sem tools, `untrusted_data`, JSON Schema, prompts versionados |
| `critic` | checagens determinísticas, achados com evidência, `CorrectionPlan` de vocabulário fechado (`FixAction`) |
| `memory` | 4 escopos (System/User/Client/Project), propostas, promoção só com `UserApproval`, precedência, auditoria |
| `gateway` | `AssetGatewayAdapter` (`LocalLibrary`, `ApprovedUrl`, `ReplayCatalog`), veredito de licença, ranqueamento, proveniência |
| `generation` | jobs de geração (opt-in, desligado por padrão), chave de idempotência, provider Replay |
| `failpoint` | pontos de falha (feature `failpoints`, **só testes**): `arm` em processo; `CAPIA_FAILPOINT[_MODE]` para SIGKILL real |

Serviço: `service/autonomy_api.rs` expõe `ai.run.{create,start,list,get,events,pause,resume,cancel,decide,review_decision,rerun,variants,group,cleanup,provenance}`, `ai.memory.{list,add,approve,reject,archive,delete,edit,audit}`, `ai.gateway.{status,set_enabled,add_library,add_url_source,remove}` e `ai.generation.set_enabled`. `service/demo.rs` (feature `testkit`, só dev/E2E) é um "cérebro" Replay por papel.

Testes (sem rede nem chave): `cargo test -p capia-intelligence --test autonomy_run|autonomy_scenarios|autonomy_budget|autonomy_crash|autonomy_crash_acquire|autonomy_kill|autonomy_security|autonomy_memory|autonomy_variants|autonomy_service|autonomy_properties` (os de crash/kill usam a feature `failpoints`). Aceitação: `node tools/phase5-acceptance/run-all.mjs` (`tools/phase5-acceptance/README.md`).
