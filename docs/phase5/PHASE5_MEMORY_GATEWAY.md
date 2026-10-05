# CapIA — FASE 5 — MEMORY, ASSET GATEWAY, GERAÇÃO E PROVENIÊNCIA

Este documento define memória controlada, aquisição externa de assets, geração de mídia e proveniência.

---

# 1. Memory scopes

Quatro escopos:

1. System
2. User
3. Client
4. Project

Precedência:
Project > Client > User > System

Conflitos devem ser mostrados ao Brain com origem.

---

# 2. MemoryItem

Schema mínimo:

```rust
struct MemoryItem {
  id: MemoryId,
  scope: MemoryScope,
  client_id: Option<String>,
  kind: MemoryKind,
  content: String,
  structured: Option<Json>,
  source: MemorySource,
  status: MemoryStatus,
  confidence: f32,
  evidence: Vec<EvidenceRef>,
  created_at: Timestamp,
  updated_at: Timestamp,
  last_used_at: Option<Timestamp>,
}
```

Kinds:
- Preference
- Rule
- Style
- Fact
- Correction

Status:
- Proposed
- Active
- Rejected
- Archived

---

# 3. Memory write policy

Project:
- IA pode propor;
- pode ativar conforme policy explícita do projeto.

User/Client:
- IA só propõe;
- promoção exige aprovação humana explícita.

System:
- somente produto/versionado.

---

# 4. Memory proposal triggers

Não criar proposta por qualquer edição.

Exemplos de trigger:
- mesma correção repetida ≥3 vezes;
- preferência explicitamente declarada;
- regra do cliente em briefing;
- padrão consistente em approvals.

Persistir evidence.

---

# 5. Memory approval UI

Mostrar:
- proposal;
- scope sugerido;
- evidence;
- confidence;
- effect examples;
- approve;
- reject;
- edit;
- change scope.

Nenhuma promoção escondida.

---

# 6. Memory retrieval

Retrieve por:
- run context;
- client;
- project;
- task type;
- relevance;
- active status.

Não enviar memória irrelevante inteira ao modelo.

---

# 7. Memory contamination prevention

Testes:
- Client A nunca aparece no Client B;
- Project A não contamina Project B;
- rejected item não reaparece como active;
- archived não é aplicado;
- source/evidence preserved.

---

# 8. Memory explainability

Cada Run registra quais memory items foram usados.

UI:
“Esta decisão usou:
- Project rule X
- Client preference Y”

---

# 9. Memory editing/deletion

Usuário pode:
- edit;
- deactivate;
- archive;
- delete according to storage policy.

Run histórico mantém referência/digest suficiente para auditoria sem reconstruir conteúdo apagado de forma indevida.

---

# 10. Asset Gateway responsibility

Gateway é única fronteira para aquisição externa que não seja provider AI.

Usos:
- search;
- fetch;
- download;
- comments/metadata quando adapter suporta.

Não expor HTTP genérico ao Brain.

---

# 11. Gateway adapter interface

Conceitualmente:

```rust
trait AssetGatewayAdapter {
  fn kind(&self) -> GatewayKind;
  async fn search(&self, req: SearchRequest, ctx: GatewayContext) -> Result<SearchResult>;
  async fn fetch(&self, req: FetchRequest, ctx: GatewayContext) -> Result<FetchResult>;
  async fn probe(&self) -> ProbeResult;
}
```

Cada adapter tem:
- allowed hosts;
- auth boundary;
- capability;
- provenance mapping.

---

# 12. Initial adapters

Implementar pelo menos:
- generic approved URL fetch through explicit allow/validation path;
- one searchable source or Replay/mock adapter;
- local/project library adapter.

Se uma integração externa específica exigir contrato/credencial indisponível:
- arquitetura + Replay + optional smoke;
- não bloquear CI.

---

# 13. No arbitrary network

O modelo nunca recebe:
- http.get;
- curl;
- browser;
- unrestricted URL downloader.

Ele pede:
- `gateway.search`
- `gateway.fetch`

Tool executor valida.

---

# 14. URL security

Validar:
- scheme;
- host;
- redirects;
- content type;
- max size;
- DNS/SSRF protections;
- localhost/private ranges according to policy;
- credential forwarding.

Não baixar file://.

---

# 15. Download staging

Download:
1. staging temp;
2. size/hash;
3. content sniff/probe;
4. provenance;
5. import through asset system;
6. atomic publish.

Falha não deixa asset parcial como válido.

---

# 16. AssetNeed

ProductionPlan gera `AssetNeed`.

Campos:
- id;
- kind;
- purpose;
- description;
- required;
- source priorities;
- constraints;
- target duration;
- budget;
- status.

---

# 17. Acquisition strategy

Order default:
1. project assets;
2. user/global library;
3. gateway;
4. generation.

Configurable by policy.

Não gerar se asset existente atende bem.

---

# 18. Search ranking

Ranking pode considerar:
- semantic match;
- aspect;
- duration;
- resolution;
- provenance/license;
- brand restrictions;
- visual diversity;
- cost.

Persistir score components quando prático.

---

# 19. Asset approval

Policy pode exigir approval para:
- paid asset;
- unclear license;
- generated face/person;
- brand-sensitive use;
- replacement of required asset.

---

# 20. Provenance record

Campos:
- asset id;
- acquisition kind;
- source adapter;
- source URI/id;
- source metadata;
- license info;
- fetched_at;
- content hash;
- parent asset;
- provider/model for generated;
- generation params;
- prompt ref/hash;
- seed if available;
- cost;
- approval ref.

---

# 21. Provenance visibility

UI mostra badge:
- Project
- Library
- Downloaded
- Generated

Inspector pode abrir provenance.

---

# 22. Generation system boundary

Fase 5 pode usar generation tools já previstas:
- image;
- video;
- TTS.

Implementar como jobs/adapters.

Não misturar generation provider com Brain provider necessariamente.

Capability Router escolhe.

---

# 23. Generation request

Campos:
- purpose;
- prompt/spec;
- reference assets;
- format;
- duration/resolution;
- model;
- budget;
- provenance;
- safety constraints;
- idempotency key.

---

# 24. Generation approval

Se custo > auto threshold:
WAITING_USER.

Opcionalmente approval sempre para video generation.

---

# 25. Generation idempotency

External generation pode ser cara.

Persistir:
- provider job id;
- request digest;
- status.

Após crash:
- poll existing job;
- nunca disparar novo job automaticamente sem confirmar que o anterior não existe.

---

# 26. Generation versions

Generated asset edits/regenerations:
- version lineage;
- parent;
- prompt changes;
- model changes.

Nunca sobrescrever arquivo anterior silenciosamente.

---

# 27. Import generated media

Após geração:
- download/staging;
- probe;
- hash;
- provenance;
- asset import;
- cache/proxy jobs.

Depois vira asset normal.

---

# 28. Gateway disabled behavior

Hard ROADMAP criterion.

Se adapter desligado:
- app continua;
- Run tenta fallback configurado;
- se não houver fallback e asset required → WAITING_USER com erro claro.

Não crash.

---

# 29. Generation disabled behavior

Se generation off:
- Planner vê capability unavailable;
- Producer deve preferir existing/gateway;
- se geração essencial → WAITING_USER.

---

# 30. Asset poisoning defense

Conteúdo baixado é hostil.

Reusar regras existentes:
- no execution;
- ffprobe timeout/limits;
- safe parser boundaries;
- hash identity;
- no metadata-as-instruction.

---

# 31. Metadata prompt injection

Title/description/subtitles de asset entram como `untrusted_data`.

Nunca promovem memory/tool permissions.

---

# 32. Licensing policy

Registrar status:
- KnownAllowed
- KnownRestricted
- Unknown
- UserProvided
- Generated

Product policy decide se Unknown pode ser usado automaticamente.

Default seguro:
approval required.

---

# 33. Spend tracking

Gateway/generation:
- estimated;
- reserved;
- actual;
- currency;
- provider/source.

Run total atualizado.

---

# 34. Memory + gateway interaction

Não salvar automaticamente:
“esse site é sempre bom”
como User memory.

Pode propor se padrão recorrente.

---

# 35. Generated asset memory

Não promover estilo de geração para User/Client sem approval.

Project memory pode registrar parâmetros específicos da demanda.

---

# 36. Asset reuse

Antes de adquirir:
- search content hash/semantic catalog;
- avoid duplicates;
- reuse approved asset when suitable.

---

# 37. Cache keys

Acquisition/generation deterministic metadata can cache:
- search results TTL;
- fetched metadata;
- analyses.

Binary asset identity remains content hash.

---

# 38. Offline recovery

Se downloaded source disappears:
local imported asset remains valid.

Provenance source unavailable ≠ asset offline.

---

# 39. Gateway auditing

Registrar:
- query;
- adapter;
- result IDs;
- selected result;
- download;
- cost;
- license info;
- errors.

Redact credentials.

---

# 40. Generation auditing

Registrar:
- model;
- provider;
- purpose;
- prompt ref;
- parameters;
- request/result ids;
- cost;
- resulting asset.

---

# 41. Memory auditing

Registrar:
- proposed;
- approved;
- rejected;
- edited;
- scope changed;
- used by Run.

---

# 42. Undo and assets

Undo de timeline não deleta asset adquirido automaticamente.

Asset remains in catalog unless explicit cleanup.

Run selective undo only removes document changes safely.

---

# 43. Run cleanup suggestions

Após undo/cancel:
- mark unused generated/downloaded assets;
- offer cleanup;
- never auto-delete source needed by other sequences.

---

# 44. Multi-client safety

Client id must be explicit.

No heuristic cross-client merge.

---

# 45. Project export portability

Provenance metadata needed to explain external/generated assets must remain with project records as designed.

Secrets remain external.

---

# 46. Memory tests

- proposal only;
- explicit promotion;
- precedence;
- conflict;
- client isolation;
- project isolation;
- repeated corrections threshold;
- rejected memory;
- archive;
- deletion.

---

# 47. Gateway tests

- disabled;
- search replay;
- fetch replay;
- timeout;
- redirect;
- oversized;
- wrong MIME;
- hash mismatch;
- SSRF;
- license unknown;
- cancellation;
- resume.

---

# 48. Generation tests

- Replay generation;
- paid approval;
- crash after submit;
- resume existing job;
- no duplicate generation;
- invalid output;
- provenance;
- version lineage;
- disabled fallback.

---

# 49. Security tests

- malicious downloaded metadata;
- injected captions;
- source URL redirect to private host;
- auth header redirect leak;
- generation prompt injection via source asset;
- memory poisoning attempt.

---

# 50. UI tests

Memory:
- proposal approve/reject.

Gateway:
- asset need/resolution.

Generation:
- pending approval/progress/result.

Provenance:
- inspect source.

---

# 51. Definition of Done — Memory/Gateway

- 4 scopes;
- proposals;
- explicit promotion;
- retrieval;
- explainability;
- client isolation;
- Gateway abstraction;
- adapter(s);
- disabled behavior;
- acquisition;
- provenance;
- generation jobs;
- idempotency;
- cost;
- security;
- UI;
- tests.

