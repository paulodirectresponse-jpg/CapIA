# CapIA — FASE 5 — ORCHESTRATOR, RUNS, STATE MACHINE E RESUMABILITY

Este documento define o núcleo operacional da autonomia do CapIA.

---

# 1. Responsabilidade do Orchestrator

O Orchestrator coordena uma AI Run completa.

Ele NÃO:
- edita timeline diretamente;
- escolhe provider ignorando router;
- acessa filesystem arbitrário;
- lê segredos;
- pula approvals;
- inventa assets;
- chama ferramentas fora do stage.

Ele FAZ:
- executar a state machine;
- montar contexto por stage;
- chamar os papéis corretos;
- persistir outputs;
- aplicar budgets;
- solicitar approvals;
- coordenar jobs;
- retomar após crash;
- classificar falhas;
- decidir transições.

---

# 2. State machine formal

Estados:

- UNDERSTAND
- PLAN
- VALIDATE_PLAN
- ACQUIRE
- EDIT
- REVIEW
- CORRECT
- DONE
- WAITING_USER
- FAILED
- CANCELLED

Transitions permitidas devem ser codificadas e testadas.

Nenhum `set_stage("qualquer string")`.

Criar enum forte.

---

# 3. Transition table

UNDERSTAND:
- success/no blocking questions → PLAN
- open questions requiring user → WAITING_USER
- provider/system error unrecoverable → FAILED
- cancel → CANCELLED

PLAN:
- success → VALIDATE_PLAN
- missing critical context → UNDERSTAND
- approval policy → WAITING_USER
- failure → FAILED

VALIDATE_PLAN:
- valid + assets available → EDIT
- valid + missing assets → ACQUIRE
- invalid fixable → PLAN
- approval required → WAITING_USER
- budget blocked → WAITING_USER
- failure → FAILED

ACQUIRE:
- all ready → VALIDATE_PLAN
- partial/fallback available → PLAN or VALIDATE_PLAN
- approval needed → WAITING_USER
- unavailable critical asset → WAITING_USER/FAILED
- cancel → CANCELLED

EDIT:
- apply success → REVIEW
- revision drift/conflict → PLAN/VALIDATE_PLAN
- apply failure → FAILED or retry
- cancel before apply → CANCELLED

REVIEW:
- pass → DONE
- findings actionable + loops remain → CORRECT
- blocker requiring replan → PLAN
- budget/loop exhausted → WAITING_USER
- failure → FAILED

CORRECT:
- success → REVIEW
- replan required → PLAN
- conflict → PLAN
- failure → FAILED

---

# 4. AiRun persistence

Persistir `AiRun` fora do documento.

Campos sugeridos:

```rust
struct AiRun {
    id: RunId,
    project_id: ProjectId,
    status: RunStatus,
    stage: RunStage,
    revision: u64,
    brain_profile_id: String,
    demand_spec_version: Option<u64>,
    created_at: Timestamp,
    updated_at: Timestamp,
    started_at: Option<Timestamp>,
    completed_at: Option<Timestamp>,
    budget: RunBudget,
    usage: RunUsage,
    policy: RunPolicy,
    current_attempt: u32,
    parent_run_id: Option<RunId>,
    variant_group_id: Option<String>,
}
```

Stage outputs devem ter tabela/records próprios.

---

# 5. Stage execution record

Cada execução de stage registra:

- run id;
- stage;
- attempt;
- input digest;
- output digest;
- started/ended;
- model endpoint;
- tools called;
- status;
- cost;
- failure;
- idempotency key.

Isso permite auditoria e resume.

---

# 6. Stage idempotency

Cada side effect precisa de chave determinística.

Exemplos:
- tool operation id;
- gateway fetch key;
- generation request key;
- transaction operation id;
- asset import fingerprint;
- stage attempt id.

Retry nunca pode repetir side effect sem verificar estado prévio.

---

# 7. Resume algorithm

Ao abrir projeto:

1. listar Runs Running/Interrupted;
2. carregar último stage record;
3. verificar side effects externos;
4. reconciliar project revision;
5. classificar:
   - safe resume;
   - revalidate;
   - needs user;
   - irrecoverable;
6. continuar do ponto seguro.

Não simplesmente reiniciar Run desde UNDERSTAND.

---

# 8. Crash boundaries

Simular kill em:
- antes de persistir output;
- depois de persistir output;
- depois de provider return;
- durante gateway;
- depois de asset download;
- durante generation polling;
- depois de preview;
- antes de apply;
- imediatamente depois de apply;
- durante review;
- durante correction.

Em todos:
- projeto consistente;
- Run auditável;
- sem duplicação.

---

# 9. Atomicity

Regras:
- stage output persistido antes de marcar stage complete;
- transition persistida atomicamente com cursor de Run;
- apply transaction é do Command Engine;
- Run metadata não substitui guarantees do store.

---

# 10. Run locks

Tipos:
- project-level orchestration lock;
- sequence write claim;
- gateway/generation job ownership.

Não manter lock de projeto durante chamada longa de provider.

Usar optimistic validation + revisions.

---

# 11. Manual edits during Run

Policy:

Se usuário editar:
- outra sequence → permitido;
- target sequence antes de EDIT → invalidar preview se revision relevante mudou;
- target sequence durante WAITING_USER → recompute plan digest;
- depois de EDIT antes de REVIEW → Critic vê nova revision e diferencia actor.

Não apagar edição manual.

---

# 12. Concurrency

Permitir subjobs paralelos:
- analyze assets;
- transcripts;
- reference analysis;
- independent gateway searches.

Limitar:
- provider concurrency;
- CPU;
- GPU;
- disk;
- budget.

---

# 13. Pause / resume

Adicionar pausa explícita.

Pause:
- não começa novas tool calls;
- tenta cancelar atividades canceláveis;
- persiste checkpoint.

Resume:
- verifica revision/budget;
- continua.

---

# 14. WaitingUser

WAITING_USER contém `PendingDecision`.

Tipos:
- OpenQuestion
- PlanApproval
- SpendApproval
- AssetApproval
- GenerationApproval
- ConflictResolution
- BudgetExtension
- FinalApproval

Campos:
- question;
- options;
- context;
- consequences;
- default if any;
- expiration policy.

---

# 15. Approval policy

Config:
- demand_spec: auto/required_if_questions/always;
- plan: auto/always;
- spend threshold;
- generation threshold;
- destructive change threshold;
- final approval.

Auto não significa bypass técnico.

---

# 16. Run budgets

```rust
struct RunBudget {
  max_cost: Option<Money>,
  max_tokens: Option<u64>,
  max_provider_calls: Option<u32>,
  max_generations: Option<u32>,
  max_review_loops: u32,
  max_replans: u32,
  max_wall_time: Option<Duration>,
}
```

Usage deve ser atualizado após cada ação.

---

# 17. Budget reservation

Antes de ação paga:
- estimate;
- reserve;
- execute;
- settle actual;
- release unused.

Evita múltiplos workers excederem orçamento simultaneamente.

---

# 18. Cost prediction

Antes de VALIDATE_PLAN apresentar:
- LLM estimated;
- STT estimated;
- vision estimated;
- gateway paid source if any;
- generation estimated;
- export/local compute optional info.

Preço desconhecido deve ser indicado.

---

# 19. Loop limits

REVIEW→CORRECT padrão 2.

Cada cycle:
- findings before;
- fixes;
- findings after;
- score delta.

Se não melhora:
- stop;
- WAITING_USER ou DONE_WITH_WARNINGS conforme policy.

---

# 20. Replanning limits

Planner não pode entrar em loop infinito.

Track:
- reason;
- prior plan digest;
- new digest;
- replan count.

Mesmo plan repetido sem progresso → stop.

---

# 21. Run event stream

Eventos para UI:
- run_created;
- stage_started;
- stage_progress;
- tool_started;
- tool_finished;
- approval_required;
- cost_updated;
- plan_ready;
- edit_applied;
- review_ready;
- correction_applied;
- run_paused;
- run_resumed;
- run_failed;
- run_done.

Throttling para progress.

---

# 22. Event durability

Eventos importantes persistidos ou reconstructable.

UI reconnect:
- carrega snapshot do Run;
- assina eventos novos;
- não depende de mensagens perdidas.

---

# 23. Error taxonomy

RunError:
- Provider
- Tool
- PlanInvalid
- AssetUnavailable
- BudgetExceeded
- Conflict
- Permission
- Security
- Persistence
- Generation
- Gateway
- Cancelled
- Internal

Cada error:
- recoverable?;
- retryable?;
- suggested transition;
- user message;
- technical detail redacted.

---

# 24. Retry policy

Retry:
- transient provider;
- network;
- 429;
- temporary gateway.

Não retry:
- auth;
- permission;
- invalid plan;
- policy violation;
- budget;
- malformed persistent model output after limited repair attempts.

---

# 25. Provider fallback

Orchestrator pede capability ao router.

Fallback:
- respeita Brain Profile;
- privacy;
- budget;
- task compatibility.

Registra troca.

---

# 26. Context builder por stage

UNDERSTAND:
- DemandInputs;
- prior DemandSpec;
- relevant memory.

PLAN:
- DemandSpec;
- ReferenceGrammar;
- asset inventory;
- memory.

VALIDATE:
- ProductionPlan;
- EditPlan;
- engine preview results.

REVIEW:
- rendered frames;
- timeline digest;
- transcript;
- DemandSpec;
- EditPlan.

Nunca dump total sem necessidade.

---

# 27. Stage outputs

UNDERSTAND:
- DemandSpec version
- questions

PLAN:
- ProductionPlan
- EditPlans

VALIDATE:
- validation report
- plan tokens
- cost estimate
- asset gaps

ACQUIRE:
- acquired assets
- provenance

EDIT:
- transaction refs
- resulting revisions

REVIEW:
- Review(s)

CORRECT:
- correction transactions
- corrected revision

DONE:
- final report
- deliverables
- variants
- total cost.

---

# 28. Multi-deliverable orchestration

ProductionPlan pode ter várias entregas.

Strategies:
- sequential;
- parallel analyses + sequential writes;
- shared master.

Nunca editar duas variants conflitantes no mesmo sequence id.

---

# 29. Variant group

Criar grouping metadata:
- run;
- demand;
- variant key;
- shared master;
- lineage;
- format.

UI consegue comparar variantes.

---

# 30. Run duplication

Permitir “rerun as new”:
- reuse analyses/cache;
- new Run id;
- new operation ids;
- optional modified brief/profile/budget.

Nunca reutilizar plan_token antigo.

---

# 31. Run history

Mostrar Runs anteriores:
- status;
- cost;
- outputs;
- provider profile;
- variants;
- decisions;
- approvals.

---

# 32. Cleanup

Caches operacionais podem ser removidos.

Não remover:
- audit;
- plans necessários para explicar timeline;
- provenance;
- memory decisions.

---

# 33. Migration

Adicionar schema explicitamente.

Testar:
- prior project opens;
- migration atomic;
- new schema rejected by old/new policies as appropriate;
- interrupted migration recovery.

---

# 34. Orchestrator test matrix

Testar:
- happy path;
- question;
- plan reject;
- spend reject;
- provider fallback;
- asset unavailable;
- conflict;
- kill per stage;
- cancellation;
- loop limit;
- replan limit;
- budget limit;
- manual edit conflict;
- resume;
- duplicate run.

---

# 35. Definition of Done — Orchestrator

- state machine formal;
- persistence;
- audit;
- resumability;
- idempotency;
- pause/cancel;
- approval;
- budgets;
- concurrency;
- event stream;
- tests;
- crash suite.

