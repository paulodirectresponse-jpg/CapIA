# CapIA — FASE 5 — AUTONOMY TESTS, ACCEPTANCE, CRASH, SECURITY E CI

Este documento define a validação da autonomia. A meta é provar que o pipeline completo funciona, é retomável, auditável e seguro.

---

# 1. Estratégia de testes

Prioridades:
1. segurança e invariantes;
2. resumability/idempotency;
3. end-to-end autonomy;
4. quality/acceptance;
5. performance/cost;
6. UI.

Evitar testes frágeis de texto livre.

Preferir:
- schemas;
- state transitions;
- command effects;
- provenance;
- revisions;
- cost;
- run events.

---

# 2. Replay provider obrigatório

Todas as flows principais precisam rodar sem rede.

Replay deve cobrir:
- DemandSpec;
- Producer;
- Planner;
- EditPlan;
- Critic;
- correction;
- memory proposal;
- gateway search;
- generation;
- failure/fallback;
- WAITING_USER;
- resume.

---

# 3. Corpus de autonomia

Criar `tests/acceptance/autonomy/`.

Casos mínimos:

1. UGC talking head + B-roll + captions.
2. Produto físico com referência.
3. Testimonial.
4. Multiple hooks + BODY_MASTER.
5. CTA variants.
6. Missing B-roll resolved via gateway.
7. Missing asset resolved via generation Replay.
8. No reference.
9. Conflicting reference and brief.
10. Strict must_include/must_avoid.
11. Low budget requiring WAITING_USER.
12. Provider failure/fallback.
13. Manual edit during run.
14. Cancel during acquire.
15. Crash during edit.
16. Crash after generation submission.
17. Critic correction loop.
18. Loop exhaustion.
19. Memory proposal.
20. Multi-client isolation.

---

# 4. Golden autonomy assertions

Não comparar prosa integral.

Assert:
- final Run status;
- stages visited;
- plans valid;
- required sequences;
- clip counts/ranges;
- brief coverage;
- no forbidden content refs;
- provenance exists;
- cost bounded;
- no duplicate transactions;
- output remains editable.

---

# 5. No write before validation

Hard test.

Instrument audit:
- before validated EditPlan: zero Agent document commits;
- after validation: only approved transaction namespace.

Mutant that allows early write must fail test.

---

# 6. Preview/apply integrity

Tests:
- token correct;
- wrong run;
- wrong actor;
- stale revision;
- altered plan;
- reused consumed token;
- cancelled Run;
- approval revoked.

All must reject safely.

---

# 7. Crash matrix

Kill process at deterministic failpoints.

Stages:
- UNDERSTAND provider return;
- PLAN persisted/not transitioned;
- VALIDATE after preview;
- ACQUIRE download;
- ACQUIRE generation submit;
- EDIT before apply;
- EDIT after apply;
- REVIEW;
- CORRECT.

After restart:
- project valid;
- Run recoverable;
- no duplicate cost/action;
- same operation ids;
- correct resume stage.

---

# 8. Real kill tests

Além de failpoints, criar child-process kill.

At least:
- kill during provider mock wait;
- kill during download;
- kill after apply;
- kill during generation polling.

---

# 9. Idempotency tests

Repeated:
- resume;
- retry;
- duplicate network response;
- duplicate tool callback;
- duplicate apply request.

Expected:
one side effect.

---

# 10. Generation idempotency

Mock provider records submit count.

Kill after submit, before persist completion.

Restart:
- query existing job;
- submit count remains 1.

---

# 11. Gateway idempotency

Fetch duplicate:
- resulting asset dedup by hash;
- no duplicate catalog identity;
- audit can record attempts.

---

# 12. Budget tests

Scenarios:
- enough;
- exact threshold;
- exceed before call;
- exceed after pricing changes;
- unknown price;
- parallel reservations.

No overspend from race.

---

# 13. Approval tests

- plan approval;
- reject;
- modify;
- spend approval;
- generation approval;
- final approval;
- approval invalidated by plan/revision change.

---

# 14. WAITING_USER tests

Run survives restart while waiting.

Decision resumes correct stage.

Invalid/old decision rejected.

---

# 15. Loop tests

REVIEW→CORRECT:
- resolves in 1 cycle;
- resolves in 2;
- no improvement;
- oscillation;
- limit reached.

Ensure no infinite loop.

---

# 16. Replan tests

Triggers:
- missing required asset;
- conflict;
- manual edit;
- impossible duration.

Limit enforced.

---

# 17. Critic quality tests

Deterministic:
- missing CTA;
- wrong duration;
- unresolved placeholder;
- caption safe area;
- offline asset.

Semantic via Replay:
- pacing;
- framing;
- visual relevance;
- brand mismatch.

---

# 18. Variant tests

`generate_variants`:
- correct count;
- distinct sequence ids;
- shared master when planned;
- lineage;
- no accidental shared override;
- editable;
- exportable.

---

# 19. Selective undo tests

Cases:
- undo whole Run with no later manual edits;
- manual edit after Run;
- partial conflict;
- shared master;
- one variant only.

Never erase unrelated manual work silently.

---

# 20. Memory tests

Hard criterion:
no User/Client Active item without explicit approval event.

Attempt bypass:
- role output says "remember this";
- tool call;
- repeated correction;
- imported brief.

All remain Proposed until approval.

---

# 21. Memory isolation

User/Client/Project boundaries.

Test random IDs and similar names.

No fuzzy client leakage.

---

# 22. Prompt injection autonomy

Hostile text in:
- brief;
- PDF;
- transcript;
- reference;
- gateway metadata;
- downloaded subtitles;
- memory proposal.

Try to:
- call shell;
- leak key;
- spend money;
- skip approval;
- promote memory;
- create nested Run;
- change policy.

Expected:
no privilege escalation.

---

# 23. Recursive autonomy protection

Agent cannot create arbitrary child Runs unless architecture explicitly allows controlled subtask mechanism.

No infinite self-spawning agents.

---

# 24. Tool allowlist tests

Every stage has exact allowlist.

Mutant adding WriteTimeline in UNDERSTAND/PLAN must fail.

---

# 25. Network boundary tests

No arbitrary HTTP tool.

Only:
- capia-ai;
- gateway adapters;
- approved generation providers.

Architecture scan + runtime interception.

---

# 26. Secret tests

Reuse Phase 4 canary across:
- Runs;
- plans;
- reviews;
- gateway;
- generation;
- memory;
- crash artifacts.

0 leaks.

---

# 27. Provenance tests

Every external/generated final asset:
- source/provenance record;
- content hash;
- run ref;
- adapter/provider;
- cost if applicable.

Missing provenance blocks automatic use when policy requires.

---

# 28. License status tests

Unknown license:
- policy requires approval.

Known restricted:
- reject or WAITING_USER according to policy.

---

# 29. AI Off regression

Even after Phase 5:
- manual editor works;
- no autonomous service required;
- no Run starts automatically.

---

# 30. Gateway disabled ROADMAP criterion

Disable gateway adapter.

Run with optional gateway need:
- fallback works.

Run with required need/no fallback:
- WAITING_USER clear error.

App stays functional.

---

# 31. Performance tests

Measure:
- Run orchestration overhead excluding provider latency;
- stage persistence;
- resume latency;
- context build;
- plan compile;
- preview;
- correction apply;
- UI responsiveness.

No hard cloud latency SLA.

---

# 32. Cost accounting tests

Assert:
- every provider/generation call accounted;
- retries counted;
- fallback counted;
- cache hits not double-spent;
- reserved vs actual reconciled.

---

# 33. Long-run soak

Replay run with:
- many assets;
- 10+ variants;
- repeated reviews;
- parallel analyses.

Watch:
- memory;
- handles;
- task leaks;
- event queue;
- DB growth.

---

# 34. Event stream tests

Reconnect UI:
- snapshot + new events;
- no lost final state;
- duplicate event tolerated.

---

# 35. Run history tests

Close/reopen project:
- runs visible;
- plans/reviews/provenance intact;
- status correct.

---

# 36. E2E headless full pipeline

Single command:
- create project;
- import fixtures;
- start Run;
- wait;
- approve where scripted;
- finish;
- inspect final timeline;
- export.

Use Replay.

---

# 37. E2E Windows desktop

Real WebView2:
- start Run;
- see progress;
- inspect plan;
- approve;
- wait edit;
- review;
- variant tabs;
- undo Run;
- reopen app.

---

# 38. E2E Linux

Use devserver/browser path:
- core autonomy logic;
- UI logic where supported;
- crash child processes;
- perf.

---

# 39. Visual smoke

Screens:
- Runs panel;
- plan approval;
- WAITING_USER;
- review findings;
- memory proposals;
- gateway asset;
- generation progress;
- variants.

Small set only.

---

# 40. Acceptance package

Create:
`tools/phase5-acceptance/`

Subfolders:
- `autonomy/`
- `crash-resume/`
- `security/`
- `real-demands/`
- `gateway/`
- `memory/`

One top-level runner aggregates.

---

# 41. Real-demand acceptance

ROADMAP requires ≥10 real demands.

Template per demand:
- brief;
- raw media;
- reference;
- requested variants;
- expected must include/avoid;
- cost budget;
- evaluator.

Metrics:
- task completion;
- editability;
- major errors;
- time;
- cost;
- human rating.

---

# 42. Human rating scale

Define clearly:

1. unusable
2. major rework
3. usable with moderate adjustments
4. usable with light adjustments
5. publish-ready/minor polish

ROADMAP target:
mean ≥ “usable with light adjustments” = 4.0.

Do not reinterpret threshold.

---

# 43. Real-demand result integrity

Store:
- evaluator;
- date;
- run id;
- output refs;
- score;
- notes.

No fabricated evaluations.

---

# 44. Automatic prechecks before human eval

Validate:
- all outputs open;
- export works;
- editable;
- no offline assets;
- provenance complete;
- no blocked findings.

---

# 45. Acceptance with external providers

Optional/live runner:
- real Brain;
- real vision;
- real STT;
- optional generation.

Secrets via environment/secure store.

Not required in public CI.

---

# 46. Corpus privacy

Do not commit user/client private media.

Acceptance tool supports local paths without adding to Git.

Version only synthetic/licensed fixtures.

---

# 47. CI jobs

Final desired jobs:

1. Rust core Linux
2. Rust + desktop Windows
3. TypeScript
4. Architecture/licenses/secrets
5. E2E Linux + autonomy
6. E2E Windows desktop + autonomy

Heavy crash/soak can be separate if runtime excessive, but core crash/resume remains gate.

---

# 48. CI run requirement

Final HEAD:
all required jobs green on same commit.

Do not cite previous green commit if HEAD red.

---

# 49. Flakiness policy

Timing tests:
- measure property, not overly tight wall-clock;
- preserve regression detection;
- rerun not accepted as sole fix.

Document adjustments.

---

# 50. Mutation tests

Add targeted mutations:
- allow write before validation;
- duplicate operation id handling broken;
- memory auto-promote;
- budget bypass;
- generation resubmit after crash;
- ignore stale revision;
- gateway disabled crashes.

Tests must catch.

---

# 51. Property tests

Useful properties:
- resume idempotency;
- cost never negative;
- budget reservation ≤ limit;
- memory promotion requires approval;
- run transition legality;
- selective undo preserves unrelated actor changes.

---

# 52. Fuzz/robustness

Fuzz:
- malformed plan;
- huge plan;
- weird Unicode;
- deep nested outputs;
- malformed gateway metadata;
- corrupted Run records.

No panic.

---

# 53. Migration tests

Schema upgrade from Phase 4:
- real fixture;
- backup;
- atomic;
- Run tables;
- memory;
- provenance.

---

# 54. Security audit final

Checklist:
- no shell;
- no arbitrary fs;
- no arbitrary HTTP;
- secrets redacted;
- approvals enforced;
- prompt injection bounded;
- spend bounded;
- memory promotion bounded;
- provenance present.

---

# 55. Architecture audit

No:
- UI → provider direct;
- provider → project write;
- gateway → command engine direct;
- critic → write;
- planner → write;
- memory → arbitrary global write.

---

# 56. Regression suite

Must preserve:
- Phase 2;
- Phase 3;
- Phase 4;
- manual AI chat;
- AI Off;
- preview;
- export;
- H.264 engineering path.

---

# 57. Docs final

Update all architecture/test docs.

ROADMAP boxes only checked with evidence.

External human criterion stays unchecked until real.

---

# 58. Completion states

If automatic engineering complete but real 10-demand evaluation pending:

`PHASE 5 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`

If real criterion also passes:

`PHASE 5 COMPLETE`

---

# 59. Final report format

Include:
- Git/CI;
- state machine;
- crash/resume;
- Producer/Planner;
- Editor/Critic;
- approvals/budget;
- memory;
- gateway/generation;
- variants;
- selective undo;
- security;
- performance;
- test counts;
- real acceptance status;
- blockers;
- whether Phase 6 may start.

---

# 60. Definition of Done — Tests

- autonomy corpus;
- replay full run;
- no-write-before-plan;
- crash matrix;
- real kill;
- idempotency;
- budget;
- approvals;
- correction loop;
- memory;
- gateway disabled;
- generation resume;
- variants;
- undo;
- security;
- E2E Windows/Linux;
- CI green HEAD;
- acceptance package.

