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
{ "transaction": { "label": "AI · Editor: montar AD 2 / Hook 1", "base_revision": 1842,
    "commands": [ { "type": "create_sequence", "ref": "$seq", "args": { ... } },
                  { "type": "insert_clip", "ref": "$c1", "args": { "track": "$seq.tracks.main", ... } },
                  ... 147 comandos ... ] } }
```

Propriedades:
- **Atomicidade:** a transação inteira vira **uma** `HistoryEntry` e **uma** transação SQLite; ou tudo entra, ou nada.
- **Isolamento:** a transação opera numa working copy (fork imutável do snapshot `base_revision`). Leitores continuam vendo a revisão anterior.
- **Validação cumulativa:** cada comando valida precondições sobre o estado já modificado pelos anteriores; invariantes globais são revalidadas no fim.
- **Limites:** `max_ops` (padrão 10.000 primitive ops) e tempo máximo; protege contra planos degenerados.
- **Dry-run:** `dry_run(transaction)` executa tudo e retorna relatório + diff sem commitar — usado no stage VALIDATE PLAN da IA e em previews de "o que vai mudar".
- **Transações aninhadas:** não suportadas (simplicidade); comandos compostos (ex.: `generate_variants`) são expandidos internamente.

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
