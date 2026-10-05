# COMMAND SYSTEM — Editing Command Engine

> Única porta de escrita do documento. Usuário (UI), IA (tools), CLI, REST/MCP usam exatamente esta API.

## 1. Camadas

```
Command (intenção semântica, versionada, serializável)       ex.: TrimClip{clip, edge:In, to: Ticks, ripple}
   │  1. schema validation (tipos, faixas)
   │  2. precondition validation (existe? track travada? alinhado a frame?)
   ▼
Expansion → Vec<PrimitiveOp>   (determinística, sobre o snapshot atual)
   │  PrimitiveOp = Insert{entity} | Delete{id} | Set{id, field_path, old, new} | Reorder{...}
   ▼
Apply to working copy  (documento imutável → nova versão com compartilhamento estrutural)
   │  3. invariant validation (TIMELINE_ENGINE.md §6), restrita às entidades afetadas + checks globais baratos
   ▼
Commit → HistoryEntry { ops, inverse_ops, label, actor, ... } → Store (SQLite) → Events (patches)
```

- **Comandos** são o vocabulário estável (UI, IA, API).
- **Primitive ops** são o mecanismo (undo, persistência, sincronização de UI, replay).
- Undo **não** recalcula comandos: aplica `inverse_ops` gravadas. Isso torna undo independente de mudanças futuras na lógica dos comandos.

## 2. Catálogo inicial de comandos (V1)

| Grupo | Comandos |
|---|---|
| Projeto/organização | `create_folder`, `rename_folder`, `move_to_folder`, `delete_folder` |
| Sequence | `create_sequence`, `duplicate_sequence`, `rename_sequence`, `delete_sequence`, `set_sequence_format`, `reorder_sequences` |
| Track | `add_track`, `delete_track`, `reorder_track`, `set_track_flags` (lock/mute/solo/hide/magnetic), `rename_track`, `set_track_role` |
| Clip | `insert_clip`, `overwrite_clip`, `move_clip`, `trim_clip`, `split_clip`, `delete_clip` (`ripple: bool`), `duplicate_clip`, `replace_clip_media`, `set_clip_speed`, `freeze_frame`, `detach_audio`, `set_clip_enabled` |
| Nested | `create_nested_from_selection`, `make_unique`, `flatten_nested`, `set_follow_length`, `generate_variants` |
| Propriedades | `set_property`, `add_keyframe`, `move_keyframe`, `delete_keyframe`, `set_keyframe_interp` |
| Efeitos | `add_effect`, `remove_effect`, `reorder_effect`, `set_effect_param` |
| Transições | `add_transition`, `set_transition`, `remove_transition` |
| Texto/legenda | `insert_text`, `set_text`, `insert_captions` (lote), `replace_caption_track`, `set_caption_words`, `apply_style` |
| Grupos/marcadores | `group_clips`, `ungroup`, `add_marker`, `move_marker`, `delete_marker` |
| Clipboard | `paste_fragment` (fragmento JSON canônico) |
| Deliverables | `create_deliverable`, `update_deliverable`, `delete_deliverable` |
| Assets (refs no doc) | `set_asset_version_policy`, `set_active_version`, `replace_asset_refs`, `relink_media` (comandos porque alteram a resolução de clips) |

Cada comando tem: `type`, `version`, schema JSON, documentação, label humano gerado ("Trim 'Hook 2' em 4 frames"), e lista de códigos de erro possíveis. O schema é a mesma fonte usada para gerar as **tools da IA** e endpoints da API.

## 3. Validação

| Nível | Quando | Exemplos de erro |
|---|---|---|
| Schema | Antes de tudo | `INVALID_ARGUMENT`, `OUT_OF_RANGE` |
| Precondição | Antes da expansão | `NOT_FOUND`, `TRACK_LOCKED`, `NOT_FRAME_ALIGNED`, `INSUFFICIENT_HANDLES`, `WRONG_TRACK_KIND` |
| Invariante | Após aplicar (por comando e no fim da transação) | `OVERLAP`, `GAP_IN_MAGNETIC_TRACK`, `NESTED_CYCLE`, `NESTED_DEPTH`, `DANGLING_REFERENCE`, `TRANSITION_NOT_ADJACENT` |
| Permissão | Antes da expansão, pelo `Actor` | `PERMISSION_DENIED` (ex.: agente sem permissão de deletar sequence) |
| Conflito | No commit | `CONFLICT` (§7) |
| Idempotência / plano | Na submissão e no apply | `OPERATION_ID_REUSED`, `OPERATION_ID_CONFLICT`, `PLAN_TOKEN_INVALID`, `PLAN_EXPIRED`, `PLAN_CONSUMED`, `PLAN_STATE_CHANGED`, `PREVIEW_REQUIRED` (§4.1–4.2) |

Erros são **estruturados e acionáveis** (essencial para a IA se autocorrigir):
```json
{ "code": "OVERLAP", "message": "Clip would overlap clip 0192… on track V2",
  "command_index": 37, "entities": ["0192…"], "hint": { "free_ranges": [[0, 1411200000]] } }
```

## 4. Transações

```
begin_tx(actor, label, base_revision, options{ mode: Atomic, validate_each: true, max_ops })
apply_in_tx(tx, command)*          → resultado por comando (ids criados, warnings), erros não abortam até o commit (modo Atomic acumula)
validate_tx(tx)                    → relatório completo (todos os erros/warnings + diff resumido)
commit_tx(tx)                      → HistoryEntry única  |  erro → nada aplicado
rollback_tx(tx)                    → descarta working copy
```

Equivalente em lote (o formato preferido pela IA e pela API):
```json
{ "transaction": { "transaction_id": "run_01J…:edit:0", "label": "AI · Editor: montar AD 2 / Hook 1", "base_revision": 1842,
    "commands": [ { "operation_id": "run_01J…:edit:0:0", "type": "create_sequence", "ref": "$seq", "args": { ... } },
                  { "operation_id": "run_01J…:edit:0:1", "type": "insert_clip", "ref": "$c1", "args": { "track": "$seq.tracks.main", ... } },
                  ... 147 comandos ... ] } }
```

Propriedades:
- **Atomicidade:** a transação inteira vira **uma** `HistoryEntry` e **uma** transação SQLite; ou tudo entra, ou nada.
- **Isolamento:** a transação opera numa working copy (fork imutável do snapshot `base_revision`). Leitores continuam vendo a revisão anterior.
- **Validação cumulativa:** cada comando valida precondições sobre o estado já modificado pelos anteriores; invariantes globais são revalidadas no fim.
- **Limites:** `max_ops` (padrão 10.000 primitive ops) e tempo máximo; protege contra planos degenerados.
- **Preview (dry-run):** `preview(transaction)` executa tudo sobre a working copy, **não commita**, e devolve relatório + diff + `plan_token` (§4.2). É o único caminho de escrita para atores `Agent` e `Api` (ADR-030).
- **Transações aninhadas:** não suportadas (simplicidade); comandos compostos (ex.: `generate_variants`) são expandidos internamente.

### 4.1 Idempotência: `operation_id` (ADR-029)

Todo comando de uma transação carrega um **`operation_id`** (string ≤ 128 caracteres, único dentro da transação e do projeto). A transação carrega um `transaction_id` opcional (agrupamento/auditoria; a idempotência vale por `operation_id`).

- **Quem gera:** a UI usa UUIDv7; o Orquestrador de IA usa ids **determinísticos** derivados de `run_id + stage + índice` (`run_01J…:edit:0:17`), para que um Run retomado após crash ou timeout reenvie os mesmos ids e **nunca duplique edições**; clientes REST/MCP escolhem os seus.
- **Registro:** tabela `applied_operations(operation_id PK, payload_hash, history_entry_id, applied_at, actor)` no `.capia`, gravada **na mesma transação SQLite** do commit. `payload_hash` = SHA-256 do JSON canônico (RFC 8785) do comando.
- **Reenvio (replay):** se **todos** os `operation_id` da transação já existem com o mesmo `payload_hash` → o engine **não reaplica**; retorna o resultado original (`replayed: true`, mesmo `history_entry_id`, mapa de `refs` original).
- **Conflitos:** mesmo `operation_id` com `payload_hash` diferente → `OPERATION_ID_REUSED`. Parte dos ids conhecida e parte nova → `OPERATION_ID_CONFLICT` (a atomicidade garante que isso só ocorre por bug ou colisão de ids; nada é aplicado).
- **Undo não libera ids.** `operation_id` identifica a *submissão*, não o efeito: após o usuário desfazer, reenviar os mesmos ids devolve `replayed: true` sem reaplicar (respeita o undo). Para reaplicar, use redo ou novos ids.
- **Retenção:** registros são mantidos por no mínimo 30 dias e nunca podados enquanto houver AI Run ativo ou retomável no projeto; poda é só por idade.
- **Escopo:** ids são por projeto. Interações da UI que não passam por preview também carregam `operation_id` (barato e uniformiza auditoria).

### 4.2 Preview → apply vinculado por token (ADR-030)

Objetivo: **impedir que um plano diferente do revisado seja aplicado**, seja por bug, race, retry ou manipulação.

```
preview(transaction)  → { plan_token, diff, report, diff_digest, base_revision, expires_at }
apply_plan(plan_token) → HistoryEntry              // recebe SÓ o token; nunca o plano de novo
```

1. **Normalização e digest.** No preview, o engine normaliza a transação (ordem de comandos, `operation_id`s, args em JSON canônico) e calcula `plan_digest = SHA-256(canonical(transaction))`. Executa em working copy e calcula `diff_digest = SHA-256(canonical(ops primitivas resultantes))`.
2. **Armazenamento do plano revisado.** O engine guarda `{plan_id, transaction normalizada, ops, plan_digest, diff_digest, base_revision, actor, scope, expires_at}` num *preview store* em memória (limitado em número e bytes; TTL padrão 15 min; apagado ao fechar o projeto).
3. **Token.** `plan_token = plan_id ‖ HMAC-SHA256(K, plan_id ‖ plan_digest ‖ diff_digest ‖ base_revision ‖ actor_id ‖ scope ‖ expires_at)`, onde `K` é uma chave de 256 bits **aleatória por processo, só em memória** (nunca persistida nem logada). Tokens não sobrevivem a reinício — comportamento desejado (re-preview).
4. **Apply.** `apply_plan` verifica: HMAC válido (comparação em tempo constante) → `PLAN_TOKEN_INVALID`; não expirado → `PLAN_EXPIRED`; não consumido → `PLAN_CONSUMED`; mesmo `actor`/`client_id` e escopo do preview; permissões ainda válidas. Como o plano vem do store (não do chamador), **não é possível aplicar um plano diferente**.
5. **Drift de revisão.** Se `current_revision ≠ base_revision`: o engine tenta **rebase** (§7); só prossegue se o conjunto de ops primitivas recomputado tiver o **mesmo `diff_digest`**. Caso contrário → `PLAN_STATE_CHANGED` e é preciso novo preview. Se o `diff_digest` mudar, qualquer aprovação humana anterior é invalidada (a aprovação se liga ao `diff_digest`).
6. **Uso único.** O token é consumido no apply bem-sucedido. Reenvio do mesmo `apply_plan` após sucesso devolve o resultado original (idempotente, via §4.1) em vez de `PLAN_CONSUMED` quando os `operation_id` já estão registrados.
7. **Obrigatoriedade.** Atores `Agent` e `Api` **só** escrevem via `preview → apply_plan`; `execute(tx)` direto devolve `PREVIEW_REQUIRED`. Atores `User`/`System` podem usar `execute` direto (gestos interativos), com `operation_id`.
8. **Gate humano.** Em checkpoints que exigem aprovação (ex.: aprovar EditPlan), a UI mostra o `diff` e a aprovação referencia `plan_id + diff_digest`; `apply_plan` só roda com aprovação correspondente.

Limite honesto: o token garante integridade e vínculo entre **revisão e aplicação dentro do mesmo processo**. Ele não impede que um humano ou agente com permissão de preview+apply submeta um plano ruim; isso é papel de validação, permissões e undo.

## 5. Referências simbólicas

Comandos de criação aceitam `ref: "$nome"`. Comandos posteriores na mesma transação podem usar `"$nome"` onde um ID é esperado, além de caminhos derivados (`"$seq.tracks.main"`). O resultado do commit devolve o mapa `ref → id real`. Refs não resolvidas → `UNRESOLVED_REF`.

## 6. Histórico semântico, undo/redo

```rust
struct HistoryEntry {
  id, revision_before, revision_after, label: String, actor: Actor,
  run_id: Option<RunId>, plan_id: Option<EditPlanId>, gesture_id: Option<GestureId>,
  commands: Vec<CommandSummary>,   // o que foi pedido (semântico, para auditoria e explicação)
  ops: Vec<PrimitiveOp>, inverse_ops: Vec<PrimitiveOp>, affected: AffectedSet, timestamp,
}
```

- **Pilha linear por projeto** (ADR-011). Undo aplica `inverse_ops`; redo reaplica `ops`. Nova edição após undo descarta o ramo de redo (o ramo descartado permanece na auditoria).
- Undo é ele mesmo registrado como evento de auditoria (não como entrada desfazível).
- **Coalescing:** comandos com o mesmo `gesture_id` em janela curta (ex.: slider de opacidade) fundem-se numa entrada.
- **Undo por ator (seletivo)** ("desfazer só a última ação da IA") só é permitido se nenhuma entrada posterior tocar as mesmas entidades (`affected` sem interseção); caso contrário a UI oferece desfazer até aquele ponto. V1 implementa undo linear; o seletivo é Fase 5.
- Profundidade: ilimitada em memória até um teto de tamanho (ex.: 256 MB), persistida no `.capia` (útil para auditoria e recovery).

## 7. Concorrência: usuário e IA ao mesmo tempo

- Mutations serializadas pelo actor do projeto (single-writer).
- Toda transação declara `base_revision`. No commit:
  - Se `current == base`: commit direto.
  - Se `current > base`: calcula-se `affected(transação) ∩ affected(entradas desde base)` em nível de entidade+campo. Sem interseção → **rebase automático** (reaplica a transação sobre o atual e revalida). Com interseção → `CONFLICT` com a lista de entidades; a IA replaneja o trecho ou pede ao usuário.
- Opção `lock_scope` para Runs de IA: a sequence alvo fica em "AI editing" — UI permite visualizar e avisa antes de editar (soft lock, nunca bloqueio duro do usuário).

## 8. Eventos

Após commit: `DocumentChanged { revision, entry_id, patches: Vec<PrimitiveOp>, affected }`. Consumidores: UI (read-replica), preview (invalidação de cache por range), render cache, AI (observação), futuros webhooks. Eventos são derivados — nunca a fonte de verdade.

## 9. Consultas (somente leitura)

`get_document(rev)`, `get_sequence(id)`, `clips_in(sequence, range, tracks?)`, `clip_at(track, t)`, `find(filter)`, `history(range)`, `diff(rev_a, rev_b)`, `timeline_digest(sequence, detail)` — representação compacta textual/JSON da timeline para LLMs (ver `AI_SYSTEM.md` §6).

## 10. Compatibilidade e versionamento de comandos

- Cada comando tem `version`. Comandos de versões antigas recebidos via API são atualizados por *upcasters*; comandos desconhecidos → `UNSUPPORTED_COMMAND`.
- Como o histórico guarda primitive ops (não comandos), mudar a lógica de um comando não quebra undo de projetos antigos.

## 11. Implementação (diretrizes)

- `capia-commands` é puro (sem IO), compila para WASM e é usado também pela UI para ghost previews.
- Funções de expansão são determinísticas: mesmo snapshot + mesmo comando ⇒ mesmas ops (IDs novos vêm de um gerador injetado — determinístico em teste).
- Testes de propriedade obrigatórios: `apply ∘ undo = id`, `undo ∘ redo = id`, invariantes sempre válidas após qualquer sequência aleatória de comandos válidos, round-trip de serialização (ver `TEST_STRATEGY.md`).

## 12. Estado da implementação (M05) e desvios deliberados

Implementado em `crates/capia-commands` (ver `docs/STATUS.md` para o que falta). Escolhas registradas na ADR-040/041:

- **Ops primitivas por entidade inteira** (`old`/`new`), não por `field_path`: inversa trivial e verificação de estado em `apply`; conflitos em nível de entidade.
- **Ids derivados** de `operation_id` quando o comando não informa o id (replay/re-preview geram os mesmos ids e o mesmo `diff_digest`).
- **`preview`** de transação já aplicada devolve `already_applied` (sem token). **`apply_plan`** recomputa sobre o estado atual e exige `diff_digest` idêntico; reenvio após sucesso devolve o resultado original (`PLAN_CONSUMED` reservado).
- Tokens: `plan_id.HMAC-SHA256(K, plan_id|plan_digest|diff_digest|base_revision|actor_id|scope|expires_at)`; `K` injetada (por processo, só memória); comparação em tempo constante; store de planos limitado (64) com TTL.
- `Agent`/`Api` → `PREVIEW_REQUIRED` em `execute`. `PERMISSION_DENIED` por comando e o gate humano ficam para a Fase 4.
- Refs: só `$nome` (sem caminhos `$seq.tracks.main`).
- Digests usam JSON canônico interno (chaves ordenadas; subconjunto de RFC 8785), não interoperável com terceiros.

## Journal e nested (M06)

O engine recebe um `Journal` (`set_journal`); cada commit/undo/redo chama `Journal::append` **antes** de publicar o novo estado. Falha ⇒ `PERSISTENCE_FAILED` e memória intacta. `Engine::restore(EngineState)` / `export_state()` reconstroem/inspecionam o estado (documento, histórico, cursor, `operation_id`s, resultados). Comandos nested novos: `delete_sequence` (`IN_USE` se referenciada), `rename_sequence`, `insert_nested`, `set_nested_target`, `set_follow_length` — ver ADR-045.

## Comandos da M07

`delete_asset` (`IN_USE` se algum clip referencia o asset; o catálogo permanece) e os cinco comandos de composição `duplicate_sequence`, `make_unique`, `flatten_nested`, `create_nested_from_selection`, `generate_variants` (semântica e ids determinísticos na ADR-050). **Relink e verify não são comandos** (ADR-048 §6): mudam onde está o arquivo, não a edição, e não entram no undo; ficam na trilha `asset_events`.

## Comando da M08

`update_asset{asset}` (ADR-058): atualiza os metadados **lógicos** (duração, vídeo/áudio, nome) de um asset existente — o lado do documento do *force relink*. Valida todos os clips dependentes (trecho de fonte cabe na nova duração; streams usados existem) e, se algum ficaria inválido, devolve `CONFLICT` com `hint.conflicts[]` (um item por clip: `sequence`, `clip`, `reasons[]` ∈ `SOURCE_RANGE_EXCEEDS_NEW_MEDIA | MISSING_VIDEO_STREAM | MISSING_AUDIO_STREAM`) sem alterar nada — **nunca trim silencioso**. Desfazível como qualquer comando; o catálogo (onde está o arquivo) continua fora do undo (ADR-048 §6).

## Comandos do editor (Fase 3)

Novos comandos (todos com `operation_id`, validados e invertíveis): `set_text`, `set_transition`, `detach_audio`, `group_clips`/`ungroup`, `reorder_clip` (reordenação em trilha magnética), `create_folder`/`rename_folder`/`move_folder`/`delete_folder`, `move_sequence_to_folder`, `set_sequence_format`, `create_deliverable`/`delete_deliverable`; `create_sequence` aceita `width/height/folder`. Refs simbólicos (`ref`/`$ref`) permitem uma única transação criar faixa + clip. **Gestos = uma transação** (arrastar vários clips, merge de legendas, aplicar estilo a todas, drop que cria faixa) ⇒ um passo de undo. **Multi-cliente:** todo `command.execute/undo/redo` publica `revision_changed`; clientes defasados ressincronizam (a UI ignora a revisão que já aplicou). Verificação automática de que a UI só escreve por comandos: `packages/editor-ui/src/architecture.test.ts` + `store/controller.test.ts`.

## Fase 4 — entradas de IA no Command Engine

`Session::agent_preview(actor, label, commands)` e `Session::agent_apply(actor, token)` (só Rust) são as **únicas** portas de escrita da IA: `Actor::Agent` recebe `PREVIEW_REQUIRED` em `execute`. O token HMAC fica preso ao ator; o apply refaz o plano sobre o estado atual e falha com `PLAN_STATE_CHANGED` se o diff mudou. `operation_id` derivado de tarefa+passo+índice ⇒ retry após commit é idempotente (`replayed`). Lista fechada de comandos permitidos ao assistente: ADR-081.
