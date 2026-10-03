# DATA MODEL — Entidades, persistência e formato de projeto

> Tipos em pseudo-Rust. A fonte de verdade futura serão os tipos em `capia-model` com schema JSON gerado. Tempo sempre em `Ticks` (`TIMELINE_ENGINE.md` §1).

## 1. Identificadores

- Todos os IDs são **UUIDv7** (ordenáveis por tempo), serializados como string, com tipo forte por entidade (`ClipId`, `SequenceId`...).
- IDs são **estáveis**: nunca reutilizados, nunca dependentes de posição. A IA e a API referenciam entidades por ID.
- Dentro de uma transação, entidades novas podem ser referenciadas por **refs simbólicas** (`"$hook_clip"`), resolvidas no commit (`COMMAND_SYSTEM.md` §5).

## 2. Escopos de armazenamento

| Escopo | Onde | Conteúdo |
|---|---|---|
| **App / Workspace** | `%APPDATA%\CapIA\app.db` (SQLite) | Settings, Provider configs (sem segredo), Model registry, Brain Profiles, Global Library index, Memory (User/Client/System overrides), clients, presets globais, jobs globais |
| **Segredos** | Windows Credential Manager (via `capia-secrets`) | API keys. Nunca em SQLite/JSON |
| **Projeto** | `Nome.capia` (arquivo SQLite único) | Documento, histórico, assets do projeto, Brief/Research/Plans, AI Runs, Project Memory, jobs do projeto, snapshots compactos |
| **Mídia gerenciada do projeto** | `Nome.capia-media\` (pasta irmã, escolhível) | Mídia baixada/gerada/consolidada pelo app |
| **Cache** | `%LOCALAPPDATA%\CapIA\cache\` | Proxies, thumbnails, waveforms, frame index, render cache — **regenerável**, chaveado por fingerprint |
| **Backups** | `%LOCALAPPDATA%\CapIA\backups\<project_id>\` (+ opcional junto ao projeto) | Cópias rotativas do `.capia` |

Justificativa do formato (ADR-015): arquivo único é portátil, atômico (SQLite + WAL), consultável e auditável; cache separado evita projetos de gigabytes; mídia referenciada externamente evita duplicação.

## 3. Entidades do documento (projeto)

```rust
struct Project {
  id: ProjectId, name, schema_version: u32, created_at, updated_at,
  client_id: Option<ClientId>,              // liga à Client Memory
  default_format: SequenceFormat, settings: ProjectSettings,
}

struct Folder   { id, parent: Option<FolderId>, name, order: i32, kind: FolderKind /* Sequences | Assets | Mixed */ }

struct Sequence {
  id, name, folder_id: Option<FolderId>, order: i32,
  format: SequenceFormat, tracks: Vec<TrackId>, markers: Vec<Marker>,
  revision: u64, origin: Origin, notes: String, tags: Vec<String>,
}
struct SequenceFormat {
  width: u32, height: u32, pixel_aspect: Rational, frame_rate: FrameRate,
  audio_sample_rate: u32 /* 48000 */, audio_channels: AudioLayout /* Stereo */,
  background: Color, color_space: ColorSpace /* Rec709 SDR na V1 */,
}

struct Track {
  id, sequence_id, kind: TrackKind /* Visual | Audio */, role: Option<TrackRole>, name, order: i32,
  locked, hidden, muted, solo, magnetic, height_px: u16, volume_db: f32,
}

struct Clip { /* ver TIMELINE_ENGINE.md §3 */ }

struct Transition { id, track_id, left_clip: ClipId, right_clip: ClipId, kind: TransitionKind,
                    duration: Ticks, alignment: Alignment, params: PropertySet }

struct Group  { id, sequence_id, name: Option<String>, members: Vec<ClipId> }
struct Marker { id, time: Ticks, duration: Ticks, label, color, kind: MarkerKind /* Note | Beat | Chapter | AiHint */ }

struct Style  { id, scope: Global|Project, kind: Text|Caption, name, props: PropertySet, version: u32 }

struct Deliverable {
  id, name, sequence_id, export_preset: ExportPresetRef,
  filename_template: String,  // ex.: "{project}_{sequence}_{format}_v{n}"
  status: Draft|Ready|Exported, last_export: Option<ExportRecord>,
}
```

### Propriedades e animação
```rust
struct PropertySet(BTreeMap<PropertyKey, AnimatableValue>);  // chaves tipadas por registry
enum AnimatableValue { Static(Value), Animated(Vec<Keyframe>) }
```
O **Property Registry** declara, para cada chave: tipo, faixa, default, se é animável, unidade e em quais conteúdos se aplica. Ele é a base para validação, inspector automático e schema das tools da IA.

Propriedades V1 (visual): `position`, `scale`, `rotation`, `anchor`, `opacity`, `crop`, `blend_mode`, `fit_mode` (fit/fill/blur_fill/stretch), `zoom` (punch-in), `brightness`, `contrast`, `saturation`, `lut_ref`.
Áudio: `volume_db`, `pan`, `fade_in`, `fade_out`, `audio_offset`.
Texto/legenda: via `Style` + overrides (fonte, tamanho, cor, stroke, sombra, fundo, highlight por palavra, animação de entrada/saída).

### Efeitos
```rust
struct EffectInstance { id, effect_type: EffectTypeId, enabled, params: PropertySet, order: i32 }
```
`EffectType` vem de um registry interno (V1: dezenas, não centenas), cada um com **uma** implementação WGSL usada em preview e export.

### Origin (rastreabilidade sem estado paralelo)
```rust
struct Origin { actor: Actor, run_id: Option<RunId>, plan_step: Option<PlanStepId>, created_at }
```
Usado para badges na UI e auditoria. **Não** altera comportamento de edição.

## 4. Entidades de assets (resumo — detalhes em `ASSET_SYSTEM.md`)

```rust
struct Asset        { id: AssetId, scope: Global|Project, kind: AssetKind, name, tags, active_version: AssetVersionId,
                      library_folder, license: Option<LicenseInfo>, created_at }
struct AssetVersion { id, asset_id, n: u32, media_file: MediaFileId, provenance: Provenance, created_at, note }
struct MediaFile    { id, fingerprint: Fingerprint, full_hash: Option<Blake3>, size, paths: KnownPaths,
                      status: Online|Offline|Missing, probe: ProbeInfo }
struct AssetRef     { asset_id: AssetId, version: VersionPolicy /* Active | Pinned(AssetVersionId) */ }
```
Clips referenciam **`AssetRef`** (lógico), nunca caminhos.

## 5. Entidades de IA (separação Brief / Research / Plan / Timeline)

Armazenadas no projeto, em tabelas próprias, **nunca** misturadas com o documento da timeline:

| Entidade | Conteúdo | Versionado |
|---|---|---|
| `DemandInput` | Arquivos/links/textos recebidos (refs a assets do tipo document/reference) | não |
| `DemandSpec` (Brief) | Especificação estruturada da demanda | sim (`v1, v2…`, cada uma imutável) |
| `ResearchItem` | Análises: `ReferenceGrammar`, notas de produto, comentários coletados, transcripts | sim |
| `EditPlan` | Plano por deliverable: beats, segmentos, intents de timing, assets escolhidos | sim; status `Draft/Validated/Approved/Applied/Superseded` |
| `AiRun` / `AiRunStep` | Execução do pipeline: stage, agente, entradas/saídas, custo, tool calls | append-only |
| `Transcript` | Palavras com tempos (source time) por MediaFile | por fingerprint (reutilizável) |
| `MemoryItem` (Project) | ver `AI_SYSTEM.md` §9 | status |

A **Timeline** (documento) só é alterada via comandos; o vínculo com o plano é o `Origin` dos clips e o `EditPlan.applied_transactions`.

## 6. Persistência, autosave, snapshots, recovery

### Tabelas principais do `.capia`
`meta(schema_version, app_version, project_id)`, `projects`, `folders`, `sequences`, `tracks`, `clips`, `transitions`, `groups`, `markers`, `styles`, `deliverables`, `assets`, `asset_versions`, `media_files`, `history_entries`, `history_ops`, `snapshots`, `demand_inputs`, `demand_specs`, `research_items`, `edit_plans`, `ai_runs`, `ai_run_steps`, `transcripts`, `memory_items`, `jobs`.

Entidades com estrutura estável são colunas; bags de propriedades/keyframes/efeitos são colunas JSON validadas pelo schema.

### Autosave = commit durável
- Cada transação confirmada no Command Engine é gravada numa **única transação SQLite** (mudanças de entidades + entrada de histórico com ops inversas). Não existe "estado sujo não salvo" além do que está na fila (ms).
- Group commit: commits muito frequentes (ex.: 100/s durante uma transação de IA) são agrupados em lotes de até 100 ms, mantendo atomicidade por transação lógica.
- SQLite em modo **WAL**, `synchronous=NORMAL` (FULL em checkpoint), `foreign_keys=ON`, checkpoints periódicos.
- "Salvar" (`Ctrl+S`) existe por hábito do usuário: força checkpoint e cria snapshot leve.

### Snapshots e versões
- **Snapshot** = export do documento em JSON canônico (zstd) guardado na tabela `snapshots`: automático (a cada 10 min de atividade, antes de transações de IA grandes, antes de migrations) e manual com nome ("Versão enviada ao cliente").
- Restaurar snapshot é um comando (desfazível) ou "abrir como novo projeto".

### Backups
Cópia consistente (SQLite Online Backup API) do `.capia` para a pasta de backups: ao abrir, a cada 30 min de atividade, rotação (ex.: 10 últimos + 1 por dia por 7 dias).

### Crash recovery
1. Ao abrir, `PRAGMA integrity_check` rápido (`quick_check`) e verificação do `meta`.
2. WAL garante que transações confirmadas sobrevivem; transações de IA não confirmadas são descartadas (rollback natural) e o AI Run fica `Interrupted`, retomável a partir do último stage concluído.
3. Se o arquivo estiver corrompido: oferecer (a) último backup, (b) último snapshot JSON, (c) reconstrução a partir de snapshot + `history_ops` posteriores.
4. Jobs: ver `ARCHITECTURE.md` §8.

### Formato de intercâmbio JSON canônico
`project.json` (schema versionado, chaves ordenadas, IDs estáveis) é usado para snapshots, testes golden, diffs, suporte, clipboard entre projetos e futura API. É uma **projeção** do SQLite, não um segundo formato concorrente.

### Migrations
- `schema_version` inteiro; migrations **forward-only**, numeradas, em Rust, testadas com fixtures de cada versão anterior.
- Antes de migrar: backup obrigatório. Projeto de versão **mais nova** que o app: abre somente leitura.
- Histórico (`history_ops`) anterior a uma migration é arquivado como auditoria e o undo stack é reiniciado (ops antigas podem não ser aplicáveis ao novo schema) — trade-off explícito (ADR-013).
- JSON canônico também carrega `schema_version` e passa pelas mesmas migrations.

### Portabilidade
- `MediaFile.paths` guarda caminho absoluto **e** caminho relativo ao `.capia`; ao abrir em outra máquina, tenta relativo → absoluto → relink por fingerprint (`ASSET_SYSTEM.md` §5).
- Comando **Package/Collect**: copia `.capia` + mídia usada (ou só trechos usados, futuro) para uma pasta portátil.
- Cache nunca é portado (regenerável).

### Auditoria
`history_entries` (append-only, com `actor`, `label`, `run_id`, timestamp, tamanho) + `ai_runs` permitem responder "quem mudou o quê, quando e por quê" — inclusive para alterações via API/MCP.

## Formato `.capia` v1 (M06)

Implementado em `capia-store` conforme ADR-042..044. **Divergências deliberadas** desta seção §6: `synchronous=FULL` (não NORMAL) e schema mais novo é **rejeitado** (não abre read-only). O documento é persistido como snapshot + eventos, não como tabelas por entidade.

## Schema 2 — catálogo de mídia (M07)

Migration v1→v2 (aditiva, com backup): `media_assets(asset_id PK, kind, content_hash UNIQUE, size_bytes, display_name, location_json, known_paths_json, media_info_json, status, status_checked_ms, imported_ms)` e `asset_events(seq, asset_id, kind, detail_json, at_ms)` (append-only). O catálogo **não** é parte do documento (nem do digest nem do undo): o documento guarda só o `Asset` lógico (`id`, nome, duração, flags); onde está o arquivo, o hash, os metadados normalizados e o estado online/offline/modified são fatos do ambiente (ADR-046/048). O arquivo de mídia nunca entra no `.capia`. Detalhes: ADR-048.
