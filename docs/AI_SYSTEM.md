# AI SYSTEM — Orquestração, agentes, tools, memória

> Providers/modelos/credenciais: `AI_PROVIDERS.md`. Segurança: `SECURITY.md`.
> Status: **arquitetura**; implementação a partir da Fase 4 (tools/inteligência) e Fase 5 (agentes/autonomia).

## 1. Princípios

1. A IA é **cliente da Engine API** (como a UI). Não acessa o documento, banco, filesystem ou shell diretamente.
2. A IA só age por **tools** declaradas, com schema, permissão e validação.
3. Toda mudança na timeline vira **transação** do Command Engine (atômica, desfazível, com `Actor::Agent`).
4. **Pensar antes de agir:** nenhuma tool de escrita na timeline é liberada antes de um Edit Plan validado.
5. O editor funciona sem IA; falha de IA nunca corrompe ou bloqueia o projeto.

## 2. Camadas

```
AI Orchestrator (pipeline state machine, AI Run persistente, orçamento, checkpoints humanos)
   │ usa
Brain (modelo primário, via Brain Profile)  ── papéis: Interpreter, Producer, Planner, Editor, Critic
   │ chama
Tool Executor (registry, schema, permissões, políticas, auditoria)
   │ chama
Engine API (command/query/assets/gateway/jobs/render)      Capability Router → modelos auxiliares (vision, STT, image, video)
```

## 3. Pipeline obrigatório (state machine)

```
UNDERSTAND → PLAN → VALIDATE_PLAN → ACQUIRE → EDIT → REVIEW → CORRECT ─┐
     ▲                                                    │            │
     └──────────── (replanejar se REVIEW exigir) ◄────────┴────────────┘
                                                                       ▼
                                                                     DONE
(qualquer stage) → FAILED | CANCELLED | WAITING_USER
```

| Stage | Agente | Entrada | Saída (persistida) | Tools liberadas |
|---|---|---|---|---|
| UNDERSTAND | Demand Interpreter | DemandInputs, memória | `DemandSpec vN` + perguntas abertas | read-only: documentos, assets, análise de mídia, reference analyzer |
| PLAN | Producer → Planner | DemandSpec, ReferenceGrammar, inventário de assets | `ProductionPlan` (deliverables, assets necessários) + `EditPlan` por deliverable | read-only + `plan.*` |
| VALIDATE_PLAN | Orchestrator + Critic | EditPlan | Relatório: `preview` das transações (gera `plan_token` + `diff_digest`), checagem de assets/duração/formato, custo estimado | `command.preview`, read-only |
| ACQUIRE | Producer | lista de assets faltantes | assets importados/baixados/gerados (jobs) | `assets.*`, `gateway.*`, `generate.*` (com orçamento/aprovação) |
| EDIT | Editor | EditPlan aprovado + assets | transações commitadas | `timeline.*` (escrita) |
| REVIEW | Critic | timeline (digest + frames amostrados), EditPlan, DemandSpec | `Review { findings[], score }` | read-only + `render.frame` |
| CORRECT | Editor | findings acionáveis | novas transações | `timeline.*` |
| DONE | Orchestrator | — | resumo, custo, deliverables prontos | — |

Regras:
- **Assets pendentes no VALIDATE_PLAN:** assets ainda não adquiridos entram no preview como *placeholders* com tipo e duração declarados no `ProductionPlan`. Após ACQUIRE, o Editor executa um **novo preview** com os assets reais (novo `plan_token`; aprovação humana, se exigida, referencia o novo `diff_digest`) antes de `apply_plan`; divergências (ex.: B-roll mais curto que o previsto) voltam ao Planner como ajuste local do plano.
- **Gate de escrita:** tools `timeline.*` de escrita só existem no contexto do agente durante EDIT/CORRECT e só se `EditPlan.status ∈ {Validated, Approved}`.
- **Checkpoints humanos configuráveis:** aprovar DemandSpec (padrão: só se houver perguntas abertas), aprovar EditPlan (padrão: ligado), aprovar gastos acima do limite (sempre).
- **Limites de loop:** máx. N ciclos REVIEW→CORRECT (padrão 2) e orçamento de custo/tempo por Run; ao estourar → WAITING_USER com relatório.
- **Retomável:** cada stage grava suas saídas; crash ou falha de provider retoma do último stage concluído.

## 4. Agentes lógicos (papéis, não necessariamente modelos distintos)

Cada papel = (system prompt versionado, conjunto de tools permitido, schema de saída, política de modelo). Por padrão todos usam o Brain; o Brain Profile pode atribuir modelos diferentes por papel.

Contratos (tipos de saída estruturada, validados por JSON Schema):

```ts
// Demand Interpreter
interface DemandSpec {
  version: number; client_id?: string; product: { name: string; claims: string[]; restrictions: string[] };
  objective: string; audience?: string; platform_targets: ("tiktok"|"reels"|"shorts"|"meta_feed"|"youtube"|"other")[];
  deliverables: { key: string; type: "ad"|"hook"|"body"|"vsl"|"cutdown"; count?: number;
                  format: {aspect: "9:16"|"1:1"|"4:5"|"16:9"; max_duration_s?: number}; notes?: string }[];
  copy?: { script_segments: { id: string; role: "hook"|"body"|"cta"; text: string }[] };
  style: { reference_ids: string[]; captions?: string; music?: string; pacing?: string; brand?: string };
  must_include: string[]; must_avoid: string[]; open_questions: string[];
  sources: { input_id: string; excerpt?: string }[];       // rastreabilidade
}

// Producer
interface ProductionPlan {
  demand_spec_version: number;
  deliverables: { key: string; sequence_strategy: "standalone"|"hook_plus_master"; hooks?: string[]; master?: string }[];
  asset_needs: { id: string; kind: AssetKind; description: string; source: "project"|"library"|"gateway"|"generate"; est_cost?: number }[];
}

// Planner
interface EditPlan {
  id: string; deliverable_key: string; target_sequence?: string; format: SequenceFormatSpec;
  grammar_ref?: string;                                    // ReferenceGrammar usada
  beats: { id: string; role: "hook"|"problem"|"demo"|"proof"|"cta"|string;
           script_segment_id?: string; target_duration_s: number;
           primary: { asset_id: string; source_range?: [number, number] };
           overlays: { kind: "broll"|"text"|"caption"|"product"|"sfx"|"music"|"zoom"|"transition"; spec: object }[] }[];
  global: { captions_style?: string; music?: object; loudness_lufs?: number };
  constraints_checked: string[];
}

// Editor: EditPlan → TransactionRequest (COMMAND_SYSTEM.md §4) — nunca escreve fora disso
// Critic
interface Review { plan_id: string; revision: number; score: number;
  findings: { id: string; severity: "blocker"|"major"|"minor"; category: "timing"|"sync"|"captions"|"framing"|"brand"|"brief"|"pacing"|"audio";
              at?: { sequence: string; range_s: [number, number] }; evidence: string; suggested_fix?: string }[] }
```

Interface comum de execução de papel:
```ts
interface AgentRole<I, O> { name: string; prompt_version: string; allowed_tools: ToolName[];
  output_schema: JSONSchema; run(input: I, ctx: RunContext): Promise<O> }
```

## 5. Tool System

### Registry
```rust
struct ToolDef { name: "timeline.insert_clip", version, description, input_schema, output_schema,
                 permission: Permission, side_effect: None|Document|Network|Spend|LocalCompute,
                 cost_hint: Option<CostHint>, timeout, idempotent: bool }
enum Permission { ReadProject, ReadMedia, WriteTimeline, ManageAssets, NetworkFetch, SpendMoney, RunHeavyCompute }
```

### Catálogo inicial (nomes estáveis)
| Namespace | Tools |
|---|---|
| `project` | `read_brief`, `get_demand_spec`, `list_sequences`, `list_deliverables` |
| `timeline` | `get_state` (digest paginado), `query_clips`, `preview` (lote de comandos com `operation_id` → `plan_token` + diff), `apply_plan` (só o token), `rollback_preview` |
| `assets` | `search`, `get`, `import_local` (só de pastas autorizadas), `get_transcript`, `get_analysis` |
| `media` | `analyze` (job), `sample_frames`, `transcribe` (job), `detect_scenes` |
| `reference` | `analyze` (job), `get_grammar` |
| `gateway` | `fetch` (url), `search`, `fetch_comments` (jobs) |
| `generate` | `image`, `video`, `tts` (jobs; exigem `SpendMoney`) |
| `render` | `frame` (imagem de um frame para o Critic), `preview_clip` (baixa res) |
| `plan` | `save_production_plan`, `save_edit_plan`, `request_approval` |
| `memory` | `recall`, `propose` (nunca grava direto em User/Client) |

Os comandos da timeline expostos à IA são **os mesmos** do Command Engine (schema gerado), embrulhados em `timeline.apply`.

### Execução de uma tool call
1. Tool existe e está liberada para (papel, stage)? 2. Input valida no schema? 3. Permissão do Actor e política (orçamento, aprovação, pastas permitidas)? 4. Executa via Engine API com `Actor::Agent{run_id, role}`; escritas na timeline só por `preview → apply_plan` (ADR-030), com `operation_id` determinístico derivado de `run_id+stage+índice` (ADR-029). 5. Saída validada e **truncada/sumarizada** para caber no contexto. 6. Auditoria (`ai_run_steps`).

### Proibições estruturais
Não existem tools de shell, filesystem arbitrário, rede arbitrária (`http.get` genérico) nem de leitura de settings/credenciais. Rede só pelo Asset Gateway (adapters) e Providers. Isso é garantido por construção (não há implementação), não por prompt.

## 6. Representação da timeline para LLMs

- `timeline.get_state(sequence, detail: "outline"|"clips"|"full", range?, page?)` retorna **digest** compacto: tracks, clips (id, tempo em segundos *e* frames, tipo, asset, texto, propriedades não default), transições, duração.
- Tempos para a IA: entrada aceita **frames** ou **segundos decimais** com conversão determinística para Ticks (round para frame) — a IA nunca manipula Ticks brutos.
- Visão da timeline: frames-chave renderizados (`render.frame`) e transcript alinhado ("o que é dito em cada trecho"), essencial para DR.

## 7. Demand Interpreter

- Entradas: DOCX, PDF, TXT, vídeos, imagens, pastas, links, briefings, copy, referências. Todos entram primeiro como **assets/inputs do projeto** (import local ou Gateway); a IA nunca lê caminhos arbitrários.
- Extração local (Rust): texto de DOCX/PDF/TXT; vídeos → transcript + análise; imagens → descrição via visão (router).
- Saída: `DemandSpec` versionada com `sources` (rastreio de onde cada informação veio) e `open_questions`.
- Separação rígida: **Brief** (DemandSpec) · **Research** (ResearchItems, ReferenceGrammar) · **Edit Plan** · **Timeline**.

## 8. Reference Analyzer (requisito V1)

Pipeline (job `ReferenceAnalysis`), local-first, IA só para semântica:

| Etapa | Técnica | Local/Cloud |
|---|---|---|
| Shot detection / cortes | diferença de histograma/scene score (libav) + refinamento | Local |
| Duração de shots, cortes por minuto | estatística | Local |
| Zoom/motion | estimativa de escala/movimento entre frames (optical flow leve) | Local |
| Transições | padrões de fade/dissolve/whip (luminância/blur) | Local |
| Texto/legendas na tela | OCR (modelo local ONNX ou visão) — posição, densidade, estilo, palavras por bloco | Local preferido |
| Fala | transcrição (STT do router) | Local ou cloud |
| Música/SFX | onset detection, separação simples voz/música, classificação de SFX | Local |
| Semântica dos shots | frames amostrados → modelo de visão: talking head, B-roll, produto, demo, depoimento, texto | Cloud (amostrado) |
| Pattern interrupts | combinação: mudança brusca de enquadramento/zoom/SFX/cor | Derivado |

Saída:
```ts
interface ReferenceGrammar {
  reference_asset_id: string; duration_s: number;
  timeline_map: { t0: number; t1: number; shot_type: string; role?: string; camera: { zoom?: string; motion?: string };
                  captions?: { style: string; words_per_block: number }; overlays: string[]; sfx: string[];
                  transition_in?: string; pattern_interrupt?: boolean; speech?: string }[];
  stats: { avg_shot_s: number; median_shot_s: number; cuts_per_min: number; broll_ratio: number;
           caption_coverage: number; zooms_per_min: number; sfx_per_min: number; interrupt_interval_s: number };
  style_summary: string;      // texto curto para o Planner
}
```
O Planner usa `stats` como alvos e `timeline_map` como esqueleto estrutural (não como cópia).

## 9. Memória

| Escopo | Onde | Escreve | Exemplo |
|---|---|---|---|
| System | embutida no app (versionada) | equipe do produto | "Legendas não cobrem a área de UI do TikTok" |
| User | app.db | usuário (explícito) ou aprovação de proposta | "Prefiro cortes secos, sem transições" |
| Client | app.db, por `client_id` | idem | "Cliente Nulle: nunca usar a palavra 'cura'" |
| Project | `.capia` | IA pode propor e aplicar no escopo do projeto | "Nesta demanda, CTA sempre com fundo amarelo" |

```rust
struct MemoryItem { id, scope, client_id: Option, kind: Preference|Rule|Style|Fact|Correction,
  content: String, structured: Option<Json>, source: ExplicitUser|InferredFromEdits|Imported,
  status: Proposed|Active|Rejected|Archived, confidence: f32, evidence: Vec<EvidenceRef>, created_at, last_used_at }
```

Regras anticontaminação:
1. Edições manuais geram, no máximo, **propostas** (`Proposed`) — e só quando um padrão se repete (ex.: ≥ 3 correções iguais).
2. Promoção para Client/User exige **aprovação explícita**. Nada é promovido a global automaticamente.
3. Precedência na recuperação: Project > Client > User > System; conflitos são mostrados ao Brain com a origem.
4. Toda memória usada num Run é registrada (explicabilidade); usuário pode ver/editar/apagar.
5. Sem fine-tuning automático.

## 10. Custo, latência e falhas

- **Estimativa antes de gastar:** Producer calcula custo estimado de geração/análise; acima do limite → aprovação.
- **Contabilidade:** tokens e custo por chamada/papel/Run/projeto (preços do Model Registry).
- **Latência:** streaming para UI; paralelismo de tool calls independentes (ex.: análises); modelos rápidos para subtarefas via router; cache de resultados determinísticos (transcripts, análises, descrições de frames por fingerprint).
- **Falhas de provider:** retry/backoff, fallback configurado no Brain Profile, Run vai a WAITING_USER se nada funcionar. Nunca deixa transação aberta: rollback automático.
- **Determinismo para testes:** provider `Replay` grava/reproduz respostas (`TEST_STRATEGY.md`).

## 11. Interfaces para automação externa (Fase 6)

`ai.start_run({ inputs: [copy, raw_video, reference], profile, deliverables, approvals: "auto"|"required" })` → `run_id`; eventos de progresso; resultado com deliverables. REST/MCP chamam exatamente isso — mesma engine, mesmas permissões (o cliente de API é um `Actor::Api` com escopos).

---
**Estado de implementação (Fase 4):** Tool System (`capia-ai::tools`), Demand Interpreter, Reference Analyzer, assistente pontual, transcrição/legendas/silêncio/cenas — ver ADR-081..086 e `docs/STATUS.md`. Fora desta fase: AI Run autônomo, Producer/Planner/Critic, variantes, memória autônoma, Asset Gateway completo (Fase 5).
