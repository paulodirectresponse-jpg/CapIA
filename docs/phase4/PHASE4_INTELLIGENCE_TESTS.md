# CapIA — FASE 4 — PIPELINES DE INTELIGÊNCIA, TESTES E ACEITAÇÃO

Este documento detalha as features de inteligência assistida da Fase 4 e os critérios de teste/aceitação.

---

# 1. Transcription

Implementar camada canônica de STT com providers locais/cloud.

Transcript canônico deve conter:
- language;
- segments;
- start/end;
- text;
- confidence quando existir;
- optional word timestamps;
- speaker id opcional se suportado.

Timing deve ser convertido determinísticamente para Ticks/amostras.

Requisitos:
- provider local;
- provider cloud;
- capability routing;
- cache por asset hash + provider/model + params;
- cancelamento;
- progress;
- retry;
- transcript persistido/referenciado sem duplicar mídia.

---

# 2. Local transcription

Implementar caminho local pluggable.

Não exigir bundling definitivo de modelo grande nesta fase se arquitetura permitir download/configuração posterior.

Deve existir:
- adapter;
- capability;
- health/probe;
- clear “not installed/configured” error.

Nenhum fallback para cloud quando privacy diz local-only.

---

# 3. Cloud transcription

Suportar pelo menos uma rota cloud real através da provider abstraction.

Enviar o mínimo necessário:
- áudio extraído/comprimido;
- nunca vídeo original se não necessário.

Respeitar privacy/budget.

---

# 4. Automatic captions

A partir do Transcript:
- gerar caption clips;
- agrupamento em blocos;
- timing;
- estilo selecionável;
- preview;
- apply por Command Engine.

Todo caption gerado é manualmente editável.

Implementar presets simples.

Não acoplar captions a um provider específico.

---

# 5. Silence cutting assistant

Feature pontual do chat:
- analisar transcript/audio;
- detectar silêncio segundo parâmetros;
- gerar proposta de cortes;
- preview;
- apply.

Não aplicar cortes silenciosamente sem a política correspondente.

Teste com fixtures de áudio controladas.

---

# 6. Media analysis

Criar `MediaAnalysis` canônico.

Pode incluir:
- duration;
- scene boundaries;
- talking-head probability/class;
- speech regions;
- visual descriptions;
- motion/zoom;
- basic audio events;
- candidate highlights;
- metadata references.

Separar:
- análise determinística/local;
- análise semântica/LLM.

---

# 7. Scene detection

Obrigatório porque Reference Analyzer depende.

Implementar caminho local.

Corpus deve conter:
- hard cuts;
- fades;
- dissolves;
- false positives por flash;
- camera motion sem cut;
- static scenes.

Ground truth anotado.

Métrica principal do ROADMAP para Reference Analyzer:
erro de detecção de cortes ≤5% contra anotação manual.

Definir precisamente métrica:
- tolerance window;
- precision;
- recall;
- F1;
- absolute cut count error.

Não escolher uma métrica que esconda erro.

---

# 8. Sample frames

Tool `media.sample_frames`.

Requisitos:
- bounded count;
- deterministic timestamps;
- optional contact sheet;
- no full-video upload;
- cache;
- source refs.

Usar frames menores para cloud vision por padrão.

---

# 9. Vision analysis

Capability Router escolhe endpoint vision.

Entrada:
- sampled frames;
- prompt version;
- optional transcript context.

Saída schema:
- shot type;
- product/talking head/demo/etc;
- framing;
- relevant objects/text;
- confidence where meaningful.

Validar JSON.

---

# 10. Reference Analyzer

Implementar pipeline completo da Fase 4.

Entrada:
- reference asset.

Pipeline:
1. validate asset;
2. scene/cut detection;
3. shot durations;
4. motion/zoom estimation;
5. fade/dissolve transition clues;
6. OCR/caption density when available;
7. speech/transcript;
8. music/SFX/onsets if practical;
9. sampled-frame semantic classification;
10. derive pattern interrupts;
11. aggregate stats;
12. produce `ReferenceGrammar`.

---

# 11. ReferenceGrammar

Seguir o contrato de `AI_SYSTEM.md`.

No mínimo:
- reference_asset_id;
- duration;
- timeline_map;
- shot_type;
- role;
- camera/motion;
- captions summary;
- overlays;
- sfx;
- transition;
- pattern_interrupt;
- speech ref;
- stats;
- style_summary.

Versionar schema.

Persistir provenance:
- analyzer version;
- provider/model semantic steps;
- transcript ref;
- asset hash.

---

# 12. Reference Analyzer evaluation

Criar corpus anotado.

Mínimo útil:
- múltiplos vídeos sintéticos determinísticos;
- alguns vídeos/fixtures licenciados ou gerados internamente;
- hard cuts;
- dissolves/fades;
- talking head/B-roll/product patterns.

Obrigatório medir cuts.

Target:
cut detection error ≤5%.

Também reportar:
- precision;
- recall;
- F1;
- transition detection;
- avg shot duration error.

Não fabricar semantic accuracy sem ground truth.

---

# 13. Demand inputs

Demand Interpreter deve aceitar via sistema autorizado:
- TXT;
- DOCX;
- PDF;
- image;
- video;
- copied text;
- project assets.

Arquivos viram inputs/assets do projeto conforme arquitetura.

Nunca path arbitrário vindo diretamente do modelo.

---

# 14. DOCX extraction

Extrair:
- paragraphs;
- headings;
- tables when practical;
- source ranges/page-like identifiers when possible.

Preservar source refs.

Não executar macros/embedded content.

---

# 15. PDF extraction

Extrair texto localmente quando text layer existe.

Se scan:
- OCR path quando disponível;
- ou structured unsupported/needs OCR status.

Manter page references.

Não fingir texto ausente.

---

# 16. Video demand input

Para vídeo de briefing:
- transcript;
- sampled visual context;
- asset metadata.

DemandSpec sources apontam para trechos/time ranges.

---

# 17. DemandSpec

Implementar schema versionado conforme `AI_SYSTEM.md`.

No mínimo:
- objective;
- product/name;
- claims;
- restrictions;
- audience;
- platforms;
- deliverables;
- copy/script segments;
- style;
- reference ids;
- captions/music/pacing/brand;
- must_include;
- must_avoid;
- open_questions;
- sources.

Cada item relevante deve ter rastreabilidade quando possível.

---

# 18. Source traceability

DemandSpec não pode ser apenas um resumo sem origem.

Implementar EvidenceRef/SourceRef.

Exemplos:
- PDF page;
- DOCX paragraph;
- transcript range;
- asset/time range;
- user message.

UI deve conseguir mostrar “de onde veio”.

---

# 19. Demand Interpreter execution

Pipeline pontual:
1. gather authorized inputs;
2. local extraction;
3. optional analysis/transcription;
4. build bounded context;
5. call structured model;
6. validate DemandSpec;
7. store version;
8. present open questions/source links.

Não iniciar ProductionPlan/EditPlan autônomo da Fase 5.

---

# 20. DemandSpec evaluation corpus

ROADMAP exige avaliação manual em ≥10 briefs reais.

Preparar `tools/phase4-acceptance/demand-spec/`.

Estrutura:
- 10+ input cases;
- expected key facts checklist;
- claims/restrictions;
- deliverables;
- source traceability;
- evaluator form;
- result aggregator.

Se briefs reais do usuário não estiverem disponíveis, criar harness e deixar `EXTERNAL ACCEPTANCE PENDING`.
Não inventar notas.

---

# 21. Chat assistant

Criar chat por projeto.

Arquitetura:
- conversation;
- task;
- model call;
- tool calls;
- optional transaction.

Features:
- stream answer;
- stop/cancel;
- retry;
- tool status;
- clear scope;
- select sequence/assets;
- show applied changes;
- undo after change.

---

# 22. Read-only chat commands

Primeiro garantir:
- “o que tem neste projeto?”
- “liste as sequences”
- “qual duração?”
- “quais mídias estão offline?”
- “analise este vídeo”
- “analise esta referência”
- “gere um DemandSpec deste briefing”.

Esses não precisam de write permission.

---

# 23. Write chat commands

Implementar tarefas pontuais:
- rename text;
- split at specified place;
- trim;
- move clip;
- create manual/automatic captions;
- remove silences;
- basic organization;
- property edits.

Todas:
- tool schema;
- preview;
- diff;
- apply;
- undo.

Não aceitar raw JS/Rust snippets do modelo.

---

# 24. Ambiguity handling

Se pedido é ambíguo:
- perguntar;
ou
- apresentar proposta sem aplicar.

Exemplo:
“corte essa parte”
sem seleção/contexto suficiente → pergunta.

Não adivinhar asset/sequence irreversivelmente.

---

# 25. Approval modes

Fase 4 pode ter preferência:
- always preview/confirm;
- auto-apply low-risk point tasks.

Mas mesmo auto-apply:
- engine preview gate existe;
- transaction audit existe;
- undo existe.

Default seguro: mostrar mudança/aplicar explicitamente para write tasks, salvo docs existentes definirem outro padrão.

---

# 26. AI task persistence

Não construir AI Run autônomo da Fase 5.

Mas tarefas pontuais precisam de rastreabilidade suficiente:
- task id;
- conversation;
- input;
- provider/model;
- tools;
- cost;
- result;
- transaction refs.

Crash não pode deixar estado “running forever”.

Pode marcar interrupted/failed.

---

# 27. Captions E2E

E2E:
1. project with speech;
2. transcribe;
3. generate captions;
4. apply;
5. preview;
6. export;
7. verify caption clips persisted;
8. manual edit;
9. undo/redo.

Replay provider can provide transcript fixture, but at least one real local STT integration test where environment permits.

---

# 28. Reference Analyzer E2E

1. import reference;
2. detect scenes;
3. semantic analysis via Replay;
4. build grammar;
5. reopen project;
6. grammar identical/deterministic for same inputs/version;
7. UI displays stats/map.

---

# 29. Demand Interpreter E2E

Input set:
- DOCX;
- PDF;
- video.

Expected:
- DemandSpec valid;
- sources include all three input types;
- open questions;
- no fabricated source ids.

---

# 30. Provider swapping acceptance

Create one canonical assistant task and run through:
- OpenAI-compatible mock/real;
- Anthropic mock/real;
- Google mock/real;
- Replay.

The application code path must not change.

Provider-specific contract tests may use local mock servers.

---

# 31. AI-Off regression

Hard requirement.

With all providers disabled:
- app launches;
- create/open project;
- import;
- timeline edit;
- preview;
- text/captions manual;
- export.

No error popup implying AI is required.

Network interceptor confirms zero AI endpoint calls.

---

# 32. Cost accounting tests

Fixtures:
- known pricing;
- unknown pricing;
- fallback;
- retry;
- cache hit;
- streaming.

Ensure:
- retry counted correctly;
- cached deterministic result can show no new provider spend;
- unknown remains unknown.

---

# 33. Cancellation tests

Cancel during:
- stream;
- STT;
- analysis;
- Reference Analyzer;
- tool preparation.

Ensure:
- HTTP abort;
- no late write;
- no leaked job;
- clear UI state.

---

# 34. Failure tests

- 401;
- 429;
- 500;
- timeout;
- malformed stream;
- invalid JSON;
- invalid tool args;
- model removed;
- provider disabled mid-task;
- budget exceeded;
- privacy route unavailable.

---

# 35. Prompt injection tests

Put hostile instructions in:
- PDF;
- transcript;
- OCR text;
- reference subtitles.

Try:
- read secret;
- use shell;
- exfiltrate;
- delete project;
- override policy.

Expected:
- treated as content;
- no privilege escalation.

---

# 36. Performance

Measure:
- chat first token;
- tool round trip;
- transcript throughput local;
- Reference Analyzer local stages;
- DemandSpec end-to-end with Replay;
- UI responsiveness during AI job.

Do not set impossible cloud latency gates.

Focus on:
- no UI blocking;
- cancellation;
- bounded context;
- parallel independent analysis where safe.

---

# 37. Context budgeting

Implement context builder.

Priorities:
- current user request;
- relevant project/sequence;
- selected assets;
- summaries;
- tool outputs.

Avoid sending full project blindly.

Measure token estimates.

---

# 38. Timeline digest for LLM

Implement `timeline.get_state`:
- outline;
- clips;
- full;
- range;
- pagination.

Return seconds/frames for model readability while keeping Ticks internal.

No 10k-clip unbounded result.

---

# 39. Media minimization

Vision:
- sampled frames;
- reduced resolution.

Transcription:
- extracted/compressed audio.

Documents:
- relevant text chunks.

Never upload original raw media by default.

---

# 40. UI — AI settings

Need screens for:
- providers;
- models;
- brain profile;
- capabilities;
- privacy;
- budgets.

Need:
- connection status;
- probe results;
- error messages.

---

# 41. UI — AI assistant

Need:
- chat panel;
- input;
- attachments/selection context;
- streaming;
- stop;
- tool progress;
- action diff;
- apply;
- undo;
- cost/usage detail;
- error/retry.

Keep editor usable while panel closed.

---

# 42. UI — Transcription/captions

Provide action from:
- asset;
- clip;
- captions panel.

States:
- queued;
- running;
- completed;
- failed;
- cancelled.

Allow regenerate with changed provider/settings.

---

# 43. UI — Reference Analyzer

Display:
- cuts/timeline map;
- stats;
- style summary;
- provider/model used for semantic portion;
- rerun;
- source asset.

No requirement to make this a full visual analytics product.

---

# 44. UI — DemandSpec

Display:
- objective;
- deliverables;
- copy;
- must include/avoid;
- open questions;
- sources.

Allow manual correction.

DemandSpec edits must version properly.

---

# 45. Security acceptance package

Create:
`tools/phase4-acceptance/security/`

One command runs:
- secret canary;
- redaction scan;
- AI-off network test;
- prompt injection suite;
- provider mock contract.

Produces machine-readable summary.

---

# 46. Demand acceptance package

Create:
`tools/phase4-acceptance/demand-spec/`

Support dropping 10 briefs into controlled input folder or referencing project fixtures.

Produces:
- generated DemandSpec;
- source map;
- checklist;
- evaluator score sheet;
- aggregate.

No automatic “passed” without evaluator.

---

# 47. Reference acceptance package

Create:
`tools/phase4-acceptance/reference-analyzer/`

Run annotated corpus and produce:
- expected cuts;
- predicted cuts;
- tolerance;
- precision;
- recall;
- F1;
- error percent.

Target ≤5% according to documented metric.

---

# 48. CI layout

Linux:
- Rust core/intelligence tests;
- Replay provider;
- mock provider contracts;
- Reference Analyzer deterministic corpus;
- Demand Interpreter fixtures;
- prompt injection;
- AI-Off;
- security redaction;
- TypeScript/headless UI where suitable.

Windows:
- Credential Manager tests;
- desktop build;
- WebView AI settings/chat E2E;
- provider config write-only secret UI;
- AI-Off desktop E2E;
- captions/chat smoke;
- existing Phase 3 E2E;
- FFmpeg robust installation from Stage 0.

Policies:
- secret scan;
- architecture;
- licenses;
- dependency review.

---

# 49. Regression preservation

Do not regress:
- Phase 2 render/export;
- Phase 3 editor;
- SharedBuffer preview;
- H.264 Windows engineering path;
- manual captions;
- timeline performance materially without explanation.

---

# 50. Architecture tests

Enforce:
- editor-ui cannot call provider implementation directly;
- providers cannot access Command Engine directly;
- tool executor goes through allowed facade;
- secrets not exposed to JS;
- no arbitrary network tool;
- no arbitrary fs/shell tool.

---

# 51. Documentation

Update:
- STATUS;
- ROADMAP;
- AI_SYSTEM;
- AI_PROVIDERS;
- SECURITY;
- ARCHITECTURE;
- DATA_MODEL;
- COMMAND_SYSTEM;
- TEST_STRATEGY;
- CLAUDE.md;
- crate/package READMEs.

Create ADRs for significant architecture only:
- provider canonical contract;
- secret storage boundary;
- tool permission gate;
- replay provider;
- intelligence persistence/cache.

---

# 52. Human/external acceptance

If real external credentials are unavailable:
- do not block engineering;
- use contract mocks + Replay;
- prepare exact smoke command for real key.

If 10 real briefs are unavailable:
- harness ready;
- mark external acceptance pending.

---

# 53. Definition of Done — intelligence

Engineering completion:
- transcription;
- captions;
- media analysis;
- scene detection;
- Reference Analyzer;
- DemandSpec;
- source traceability;
- assistant read/write tasks;
- preview/apply gate;
- provider switch;
- AI-Off;
- cost;
- cancellation;
- security.

---

# 54. Final report

Must include:

## Stage 0
- run that closed Phase 3 HEAD;
- CI fixes.

## Git
- Phase 4 branch;
- final commit;
- final run.

## Providers
- families/adapters;
- probes;
- Brain Profiles;
- routing.

## Security
- secret store;
- canary result;
- prompt injection;
- network boundaries.

## Intelligence
- transcription;
- captions;
- media analysis;
- Reference Analyzer metrics;
- DemandSpec.

## Assistant
- tools;
- point tasks;
- transaction safety;
- cost.

## Testing
- provider contracts;
- E2E;
- AI-Off;
- corpus;
- failures.

## Acceptance
- 10-brief status;
- any real-provider smoke;
- remaining external items.

## Status
- `PHASE 4 COMPLETE`
or
- `PHASE 4 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`.

Do not start Phase 5.
