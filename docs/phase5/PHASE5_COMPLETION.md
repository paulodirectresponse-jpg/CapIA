# CapIA — FASE 5 / AUTONOMIA — ESPECIFICAÇÃO DE CONCLUSÃO

**Objetivo:** concluir toda a engenharia da Fase 5 em uma única missão autônoma.

**Base:** `claude/phase4-intelligence`  
**HEAD base verde:** `bec75a3cda243dcbcebfc2e4246f1e81ec90a20a`  
**Target branch:** `claude/phase5-autonomy`

---

# 1. Missão

Transformar as capacidades assistidas da Fase 4 em um pipeline autônomo completo:

`brief + bruto + referência → plano → aquisição → edição → revisão → correção → variações editáveis`

O sistema deve ser capaz de executar uma demanda completa com o mínimo de intervenção humana, mantendo:
- rastreabilidade;
- reversibilidade;
- editabilidade;
- orçamento;
- aprovação;
- segurança;
- retomada após falha;
- proveniência.

---

# 2. Estado de entrada

A Fase 4 está em:

`ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`

Isso significa:
- engenharia concluída;
- CI verde;
- providers/router/tools/intelligence disponíveis;
- pendências externas reais ainda abertas.

Essas pendências NÃO bloqueiam a engenharia da Fase 5 e NÃO podem ser marcadas como concluídas sem evidência real.

---

# 3. Fronteira da Fase 5

Esta fase IMPLEMENTA:
- AI Run persistente;
- state machine autônoma;
- Producer;
- Planner;
- EditPlan;
- validação de plano;
- Editor autônomo;
- Critic;
- correções iterativas;
- approvals;
- budgets;
- memory;
- asset acquisition;
- generation hooks;
- variants;
- selective undo;
- autonomy UI;
- resumability;
- test harness completo.

Esta fase NÃO IMPLEMENTA:
- REST API pública;
- MCP Server público;
- webhooks externos;
- instalador assinado;
- auto-update;
- API token scopes para terceiros;
- distribuição/beta da Fase 6.

---

# 4. Princípios inegociáveis

1. Timeline é fonte de verdade.
2. Tudo produzido pela IA vira clips/propriedades normais e editáveis.
3. IA nunca manipula UI.
4. IA nunca escreve direto no documento.
5. Toda escrita usa Command Engine.
6. Escrita de Agent exige `preview → apply_plan`.
7. Nenhuma escrita antes de plano validado.
8. Run é persistente e retomável.
9. Retry nunca duplica edição.
10. Assets externos têm proveniência.
11. Gastos têm orçamento e política.
12. Memória global/client nunca é promovida automaticamente.
13. Falha de provider não corrompe projeto.
14. App continua funcional se Gateway/generation/provider estiver desligado.
15. Kill/restart deve retomar com segurança.

---

# 5. Estado final esperado

O usuário fornece:
- briefing;
- copy;
- vídeos brutos;
- referências;
- objetivos;
- formatos desejados.

O CapIA:
1. entende;
2. planeja;
3. mostra/aprova plano quando necessário;
4. verifica assets;
5. adquire ou gera o que falta;
6. edita;
7. revisa;
8. corrige;
9. cria variantes;
10. entrega sequences prontas e totalmente editáveis.

---

# 6. AI Run

Criar entidade persistente `AiRun`.

Campos mínimos:
- id;
- project_id;
- status;
- stage;
- created_at;
- updated_at;
- started_at;
- completed_at;
- brain_profile_id;
- demand_spec_version;
- production_plan_ref;
- edit_plan_refs;
- budget;
- cost_actual;
- time_budget;
- loop_budget;
- approval_policy;
- current_checkpoint;
- error;
- resume_token/version;
- actor identity.

Statuses:
- Pending
- Running
- WaitingUser
- Failed
- Cancelled
- Completed

---

# 7. Stages

State machine obrigatória:

UNDERSTAND  
→ PLAN  
→ VALIDATE_PLAN  
→ ACQUIRE  
→ EDIT  
→ REVIEW  
→ CORRECT  
→ DONE

Transições adicionais:
- FAILED
- CANCELLED
- WAITING_USER

Cada stage:
- tem input versionado;
- produz output persistido;
- registra tool calls;
- registra usage/cost;
- é idempotente ou tem chave de idempotência;
- pode ser retomado.

---

# 8. Checkpoints humanos

Configuração:
- approve DemandSpec;
- approve ProductionPlan;
- approve EditPlan;
- approve spend;
- approve generated media;
- approve final output.

Defaults:
- dúvidas abertas relevantes → WAITING_USER;
- plano de edição → aprovação configurável;
- gastos acima do limite → sempre aprovação;
- ação destrutiva não trivial → preview obrigatório.

---

# 9. Budget system

Budgets mínimos:
- dinheiro;
- tokens;
- chamadas;
- tempo;
- número de generations;
- número de review loops;
- número de replans.

Se estourar:
- não continuar silenciosamente;
- salvar estado;
- WAITING_USER;
- explicar motivo e estimativa para continuar.

---

# 10. Persistência

Persistir separadamente:
- Run;
- stage outputs;
- approvals;
- plans;
- reviews;
- memory proposals;
- asset acquisition records;
- generation records;
- usage/cost;
- provenance.

Não colocar tudo dentro do documento da timeline.

---

# 11. Crash/resume

Hard requirement.

Depois de kill:
- projeto abre;
- Run aparece como resumível;
- stage incompleto é classificado;
- stage concluído não é repetido desnecessariamente;
- nenhuma edição é duplicada;
- nenhum asset é duplicado;
- nenhum gasto externo deve ser repetido sem verificar idempotência/provider job id;
- transaction ids permanecem consistentes.

---

# 12. Cancellation

Cancel deve:
- abortar chamadas de provider;
- cancelar jobs;
- interromper gateway;
- impedir late tool apply;
- preservar stage outputs já concluídos;
- deixar projeto consistente.

Run cancelado pode:
- permanecer auditável;
- permitir “duplicate/retry run”;
- não continuar em background.

---

# 13. Run locking

Prevenir dois escritores autônomos conflitantes na mesma sequence.

Permitir:
- leitura concorrente;
- edição manual em outra sequence;
- política para edição manual na mesma sequence;
- detecção por revision/diff digest.

Conflito:
- não sobrescrever;
- voltar a PLAN/VALIDATE ou WAITING_USER.

---

# 14. Variants

Implementar `generate_variants`.

Casos:
- múltiplos hooks;
- hook + body master;
- CTAs alternativos;
- pacing variants;
- format variants.

Preferir nested/shared master quando apropriado.

Cada variante:
- sequence própria;
- lineage/provenance;
- custo;
- plan;
- editable normally.

---

# 15. Selective undo

Implementar undo por ator/run quando possível.

Necessário:
- identificar transações do Run;
- desfazer Run sem apagar edições manuais posteriores incompatíveis silenciosamente;
- detectar conflitos;
- oferecer strategy:
  - safe undo;
  - partial undo;
  - manual conflict resolution.

Nunca reescrever histórico ocultamente.

---

# 16. UI de autonomia

Criar painel de Runs.

Mostrar:
- estágio;
- progresso;
- custo;
- duração;
- plano;
- approvals;
- tools;
- assets;
- findings;
- corrections;
- variants;
- logs resumidos;
- erros;
- resume;
- cancel.

Não expor chain-of-thought.

---

# 17. Plan UI

Mostrar:
- ProductionPlan;
- EditPlan;
- assets necessários;
- custos estimados;
- estrutura de sequences;
- beats;
- referências usadas;
- constraints;
- warnings;
- diff previsto.

Permitir:
- approve;
- reject;
- ask change;
- edit high-level parameters.

---

# 18. Review UI

Mostrar findings do Critic:
- severity;
- category;
- range;
- evidence;
- suggested fix;
- status.

Permitir:
- apply correction;
- ignore;
- lock decision;
- stop loop.

---

# 19. Memory UI

Mostrar:
- active memory;
- proposed memory;
- scope;
- evidence;
- confidence;
- origin;
- approve/reject/archive.

User/Client promotion exige ação explícita.

---

# 20. Gateway UI

Mostrar:
- asset need;
- source;
- adapter;
- result;
- license/provenance;
- cost;
- generated/downloaded state.

---

# 21. Provenance

Todo asset adquirido/gerado precisa:
- source;
- adapter/provider;
- URL/id quando permitido;
- timestamp;
- license/status;
- model/provider;
- prompt hash/ref;
- generation params;
- parent asset refs;
- content hash;
- user approval if required.

---

# 22. Security

Preservar todas as garantias da Fase 4.

Novos riscos:
- prompt injection durante autonomous loop;
- asset poisoning;
- malicious metadata;
- memory poisoning;
- gateway exfiltration;
- repeated spend;
- recursive run creation;
- infinite correction loop.

Adicionar testes específicos.

---

# 23. Performance

Autonomia não deve bloquear UI.

Requirements:
- jobs async;
- progress stream;
- bounded concurrency;
- pause/resume;
- no giant context dump;
- cache deterministic analyses;
- reuse transcripts/reference grammar.

---

# 24. Determinism for tests

Provider Replay obrigatório para:
- complete Run;
- failures;
- retries;
- reviews;
- corrections;
- memory proposals;
- asset acquisition;
- variants.

Replay fixture deve poder reproduzir a Run completa sem rede.

---

# 25. Definition of Done — engenharia

- AI Run persistente;
- state machine;
- resume;
- cancel;
- Producer;
- Planner;
- EditPlan;
- Validate;
- Editor;
- Critic;
- Correct loop;
- approvals;
- budgets;
- memory;
- gateway;
- provenance;
- variants;
- selective undo;
- UI;
- E2E;
- crash tests;
- security;
- docs;
- CI verde no HEAD.

---

# 26. Estados finais permitidos

### PHASE 5 COMPLETE
Somente se:
- engenharia concluída;
- ≥10 demandas reais;
- avaliação humana real;
- média ≥ “utilizável com ajustes leves”;
- demais critérios do ROADMAP realmente medidos.

### PHASE 5 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING
Quando:
- engenharia completa;
- CI verde;
- Replay/corpus automático verde;
- restam apenas demandas reais/avaliação humana/providers externos.

Não inventar resultados.

---

# 27. Documentação final

Atualizar:
- STATUS;
- ROADMAP;
- AI_SYSTEM;
- AI_PROVIDERS;
- ASSET_SYSTEM;
- SECURITY;
- ARCHITECTURE;
- DATA_MODEL;
- COMMAND_SYSTEM;
- TEST_STRATEGY;
- CLAUDE.md;
- READMEs;
- DECISIONS/ADRs.

---

# 28. Final report

Entregar:
- branch;
- commit;
- CI;
- Run state machine;
- Producer/Planner;
- Editor/Critic;
- crash/resume;
- budgets;
- approvals;
- memory;
- gateway;
- generation;
- variants;
- selective undo;
- security;
- performance;
- E2E;
- external pending items;
- next phase status.

Não iniciar Fase 6.
