# CapIA — FASE 5 — PRODUCER, PLANNER, EDITOR E CRITIC

Este documento define os quatro papéis centrais da autonomia criativa.

---

# 1. Papéis

## Producer
Responsável por transformar a demanda em estratégia de produção.

## Planner
Transforma estratégia em EditPlan executável.

## Editor
Converte EditPlan validado em comandos/transações.

## Critic
Avalia resultado contra brief, plano, referência e qualidade técnica.

Eles podem usar o mesmo Brain ou modelos diferentes via Brain Profile.

---

# 2. Producer input

Producer recebe:
- DemandSpec;
- ReferenceGrammar;
- project asset inventory;
- relevant memory;
- requested deliverables;
- platform/format requirements;
- budget;
- generation/gateway availability.

Não recebe segredo.

---

# 3. ProductionPlan

Schema mínimo:

```ts
interface ProductionPlan {
  id: string;
  demand_spec_version: number;
  strategy: string;
  deliverables: {
    key: string;
    sequence_strategy: "standalone" | "hook_plus_master" | "shared_master" | "format_variant";
    source_material: string[];
    hooks?: string[];
    master?: string;
    notes?: string;
  }[];
  asset_needs: {
    id: string;
    kind: AssetKind;
    purpose: string;
    description: string;
    source_priority: ("project"|"library"|"gateway"|"generate")[];
    required: boolean;
    estimated_cost?: number;
  }[];
  constraints: string[];
  assumptions: string[];
  risks: string[];
}
```

Versionar.

---

# 4. Producer rules

Producer deve:
- preferir assets existentes quando adequados;
- reutilizar master/nested quando fizer sentido;
- não gerar mídia desnecessariamente;
- respeitar budget/privacy;
- indicar assumptions;
- indicar assets faltantes;
- prever variantes explicitamente.

---

# 5. Planner input

Planner recebe:
- ProductionPlan;
- DemandSpec;
- ReferenceGrammar;
- asset catalog;
- transcripts;
- analyses;
- relevant memory;
- timeline constraints;
- sequence format.

---

# 6. EditPlan

Schema mínimo:

```ts
interface EditPlan {
  id: string;
  version: number;
  deliverable_key: string;
  target_sequence?: string;
  format: SequenceFormatSpec;
  grammar_ref?: string;
  beats: Beat[];
  global: GlobalEditSpec;
  constraints_checked: string[];
  asset_placeholders: AssetPlaceholder[];
  estimated_duration_s: number;
  estimated_cost?: number;
}
```

Beat:
- id;
- role;
- script segment;
- duration target;
- primary asset/range;
- overlays;
- captions;
- sfx;
- transition intent;
- framing;
- motion intent;
- notes;
- dependency refs.

---

# 7. Plan granularity

Plano não precisa microgerenciar cada pixel.

Deve ser detalhado o bastante para:
- prever comandos;
- validar duração;
- validar asset availability;
- estimar custo;
- comparar resultado.

Separar:
- creative intent;
- execution details.

---

# 8. Reference usage

ReferenceGrammar funciona como alvo estrutural.

Pode influenciar:
- average shot length;
- cut rhythm;
- b-roll ratio;
- caption density;
- zoom frequency;
- sfx frequency;
- pattern interrupts.

Não copiar literalmente conteúdo protegido.

---

# 9. Plan constraints

Checar:
- format;
- max duration;
- must_include;
- must_avoid;
- required CTA;
- brand restrictions;
- platform safe areas;
- asset rights/provenance;
- source handle lengths;
- audio availability.

---

# 10. Placeholder semantics

Assets ainda não adquiridos entram como placeholders tipados:
- kind;
- target duration;
- purpose;
- expected visual/audio properties.

Preview de plano deve aceitar placeholder sem fingir mídia real.

Após ACQUIRE:
- resolver placeholder;
- revalidar;
- gerar novo plan_token.

---

# 11. VALIDATE_PLAN

Antes de qualquer escrita:
- compile commands;
- `preview`;
- validate command semantics;
- inspect diff;
- verify assets;
- verify durations;
- verify sequence formats;
- estimate total cost;
- identify destructive impacts;
- produce `ValidationReport`.

---

# 12. ValidationReport

Campos:
- plan_id;
- valid;
- diff_digest;
- plan_token;
- target_revision;
- command_count;
- sequences affected;
- assets missing;
- warnings;
- errors;
- estimated cost;
- approval required;
- placeholder state.

---

# 13. Approval invalidation

Qualquer mudança relevante após approval:
- asset replacement;
- target revision change;
- plan content change;
- new generation;
- duration change material;

invalida approval/diff quando necessário.

Nunca aplicar token referente a outro digest.

---

# 14. Editor role

Editor recebe:
- validated EditPlan;
- resolved assets;
- plan token/diff;
- engine schemas.

Editor NÃO:
- acessa provider secret;
- escreve raw DB;
- inventa command names;
- bypassa preview.

---

# 15. Edit compilation

EditPlan → TransactionRequest.

Cada command:
- schema válido;
- actor Agent;
- deterministic operation_id;
- symbolic refs quando necessário;
- explicit target sequence.

---

# 16. Operation id scheme

Derivar de:
- run_id;
- plan_id;
- stage;
- beat_id;
- command index;
- correction cycle.

Retry gera o mesmo id para mesma ação.

Nova versão do plano gera novo namespace.

---

# 17. Command batching

Agrupar semanticamente:
- sequence setup;
- primary assembly;
- overlays;
- text/captions;
- audio;
- transitions;
- finishing.

Evitar transações gigantes quando isso prejudicar recovery/undo.

Mas preservar atomicidade onde semanticamente necessário.

---

# 18. Editor safety

Antes do apply:
- revision match;
- token valid;
- budget still valid;
- user did not cancel;
- run owns write claim.

Depois:
- persist transaction refs;
- update resulting revision;
- emit event.

---

# 19. Partial edit policy

Não deixar meia edição “silenciosa”.

Se batch intermediário é intencional:
- registrar checkpoint;
- Run status mostra partial progress;
- resume sabe o que já foi aplicado.

---

# 20. Critic input

Critic recebe:
- DemandSpec;
- ProductionPlan;
- EditPlan;
- ReferenceGrammar;
- resulting timeline digest;
- transcript;
- sampled frames;
- audio metrics where available;
- provenance/asset info.

---

# 21. Review schema

```ts
interface Review {
  id: string;
  run_id: string;
  plan_id: string;
  revision: number;
  score: number;
  pass: boolean;
  findings: Finding[];
  summary: string;
}
```

Finding:
- id;
- severity;
- category;
- at;
- evidence;
- expected;
- observed;
- suggested_fix;
- confidence.

---

# 22. Critic categories

- timing
- pacing
- sync
- captions
- framing
- continuity
- brand
- brief
- reference
- audio
- transition
- asset quality
- CTA
- technical

---

# 23. Evidence

Finding precisa de evidência concreta quando possível:
- frame ref;
- transcript ref;
- timeline range;
- plan beat;
- constraint id;
- audio measurement.

Evitar “não gostei” sem explicação.

---

# 24. Critic scoring

Definir rubric versionada.

Exemplo:
- brief compliance;
- timing/pacing;
- continuity;
- audio;
- caption/readability;
- reference alignment;
- technical validity.

Score não pode ser usado como verdade absoluta; findings têm prioridade.

---

# 25. Deterministic critic checks

Antes de LLM:
- duration limits;
- missing required text;
- sequence format;
- offline assets;
- clipping/empty gaps;
- caption safe area;
- audio peak/loudness where available;
- unresolved placeholders.

Esses checks são locais.

---

# 26. Semantic critic

LLM/vision avalia:
- relevance;
- framing;
- pacing;
- hook clarity;
- CTA clarity;
- brand adherence;
- visual-semantic fit.

Usar sampled frames/context bounded.

---

# 27. REVIEW→CORRECT

Se finding actionable:
- transform findings into correction request;
- Editor compila minimal corrections;
- preview;
- apply;
- review again.

Não reconstruir vídeo inteiro por default.

---

# 28. CorrectionPlan

Criar representação:
- finding refs;
- intended changes;
- affected ranges;
- commands;
- expected improvement;
- cost.

---

# 29. Locked decisions

Usuário pode marcar finding/decision como:
- accepted;
- ignore;
- do not change.

Critic não deve insistir no próximo loop.

---

# 30. Replan triggers

Voltar a PLAN quando:
- asset fundamental indisponível;
- duration strategy impossible;
- reference conflict;
- user changes objective;
- major structural finding;
- correction would affect large portion.

---

# 31. Avoid oscillation

Guardar recent changes/findings.

Se:
- correction A desfaz correction B;
- score não melhora;
- findings alternam;

parar e WAITING_USER.

---

# 32. Multi-variant planning

Producer pode planejar:
- shared BODY_MASTER;
- multiple hook sequences;
- independent variants.

Planner gera planos relacionados.

Manter lineage.

---

# 33. Hook/master strategy

Quando vários hooks compartilham body:
- body como master;
- hooks como sequences/nested composition;
- edits no master propagam;
- variant-specific override exige make unique quando necessário.

---

# 34. Format variants

9:16 / 1:1 / 4:5 / 16:9.

Planner deve considerar:
- framing;
- safe areas;
- text layout;
- crop;
- captions.

Não apenas trocar resolution metadata.

---

# 35. Copy alignment

Se DemandSpec tem script segments:
- manter link beat ↔ script_segment;
- timeline clip/caption provenance referencia segmento;
- Critic compara coverage.

---

# 36. Audio planning

Planner define:
- dialogue priority;
- music intent;
- SFX intents;
- ducking/loudness targets quando suportado;
- fades.

Editor usa engine existente.

---

# 37. Caption planning

Planner define:
- style preset;
- density;
- timing policy;
- emphasis if supported.

Automatic captions continuam editáveis.

---

# 38. Transition planning

Use transições com intenção.

Preferir corte seco quando referência/brief indica.

Não encher de transições por default.

---

# 39. B-roll planning

Selecionar por:
- semantic fit;
- source range;
- duration;
- visual variety;
- continuity.

Asset search score deve ser explicável.

---

# 40. Asset selection ranking

Possível ranking:
- brief semantic match;
- transcript match;
- reference role match;
- quality;
- duration handles;
- provenance confidence;
- reuse penalty.

Persistir motivo.

---

# 41. Generated media usage

Só quando:
- allowed;
- budget permits;
- project/library/gateway insufficient;
- policy allows.

Generated asset deve ser normal asset depois de import.

---

# 42. Editor output

Output nunca é “vídeo fechado”.

É:
- sequences;
- clips;
- properties;
- nested;
- text/captions;
- audio;
- transitions;
- provenance.

Totalmente manual-editable.

---

# 43. Plan editability

Usuário pode alterar plano antes de apply.

Mudança cria nova versão/digest.

---

# 44. Role prompts

Prompts versionados.

Cada role:
- prompt_version;
- allowed_tools;
- output_schema;
- context budget;
- model policy.

Não misturar todas responsabilidades em um prompt monolítico.

---

# 45. Role separation tests

Testar:
- Producer não escreve;
- Planner não aplica;
- Editor não reinterpreta objective livremente;
- Critic não escreve.

---

# 46. Fallback models by role

BrainProfile pode:
- Producer model A;
- Planner model A/B;
- Critic vision model B;
- Editor structured model A.

Capability Router resolve.

---

# 47. Plan repair

Structured output inválido:
- limited repair retry;
- validate schema;
- if repeated failure → FAILED/WAITING_USER.

Nunca aceitar plano parcialmente parseado.

---

# 48. Long plans

Paginar/chunk quando necessário.

Mas persistir plano canônico inteiro.

---

# 49. Plan provenance

Registrar:
- model;
- prompt version;
- input digests;
- memory refs;
- reference grammar;
- asset snapshot revision.

---

# 50. Review provenance

Registrar:
- model;
- frames refs;
- timeline revision;
- rubric version;
- DemandSpec version.

---

# 51. Test corpus

Fixtures para:
- UGC;
- product ad;
- testimonial;
- VSL short section;
- multiple hooks;
- missing B-roll;
- no reference;
- conflicting reference/brief;
- strict brand restriction.

Replay respostas versionadas.

---

# 52. Golden plan tests

Para casos determinísticos:
- ProductionPlan schema;
- EditPlan invariants;
- command compilation;
- no write before validation.

Evitar golden textual frágil de prosa.

---

# 53. Critic tests

- detect missing CTA;
- detect duration violation;
- detect caption issue;
- detect offline asset;
- detect plan beat missing;
- semantic findings via Replay.

---

# 54. Correction tests

- minimal fix;
- revision preserved;
- no duplicate commands;
- finding marked resolved;
- second review sees improvement.

---

# 55. Definition of Done

- Producer implemented;
- Planner implemented;
- plan schemas/versioning;
- validation;
- Editor compiler;
- operation id;
- Critic;
- deterministic + semantic checks;
- correction loop;
- multi-variant;
- format variants;
- provenance;
- role separation;
- tests.

