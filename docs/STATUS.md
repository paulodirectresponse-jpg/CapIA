# STATUS

**Última atualização:** 2026-10-03 · **Fase atual:** FASE 2 — Motor (headless) · **Missão corrente:** M07 concluída (Assets + Mídia + composição de nested) · **Próxima:** M08 (proposta em "Próxima missão"; **não iniciada**)

## Gates de fase (decisão do Product Owner, ADR-037)

```
Fase 2 — Motor (headless)     EM ANDAMENTO (OD-1 aberto não bloqueia).
Fase 3 — Editor / Preview     NÃO pode iniciar sem OD-1 fechado (S1 medido em Windows).
```

OD-1 = presenter do preview na janela Tauri/WebView2. Pacote de medição pronto: `tools/s1-preview-spike/` (`.\run.ps1`). **Fora do escopo da Fase 2:** preview embutido na janela e UI de editor.

## Estado

| Item | Estado |
|---|---|
| Núcleo de tempo (`capia-time`) | ✅ `Ticks` (aritmética *checked*, half-up/floor), `Rational`, `FrameRate`, `TimeRange` — 25 testes |
| Modelo (`capia-model`) | ✅ Document com compartilhamento estrutural, Sequence/Track/Clip/Marker/Asset, keyframes (Bézier exato), **ops primitivas invertíveis**, invariantes (sobreposição, magnética, alinhamento, família, fonte, nested DAG/profundidade 16, limites) — 20 testes |
| Command Engine (`capia-commands`) | ✅ 19 comandos, transações atômicas, histórico/undo/redo, `operation_id` idempotente (ADR-029), `preview → apply_plan` por token HMAC (ADR-030), rebase/`CONFLICT`, refs `$nome`, auditoria — ver "O que existe" |
| Suíte de aceitação (ADR-036) | ✅ **120/120** cenários no engine real (108 + 12 da M05); mutação detecta regressões |
| Persistência (`capia-store`) | ✅ M06: `.capia` = SQLite v1 (STRICT, WAL, `synchronous=FULL`), journal de eventos + snapshots, migrations explícitas, commit atômico, idempotência durável, undo/redo após reabrir, crash tests com processo morto (ADR-042..044) |
| Facade + CLI (`capia-project`, `capia-cli`) | ✅ M06: `capia create/inspect/validate/apply/undo/redo/history/dump` |
| Nested (comandos) | ✅ M06: `delete/rename_sequence`, `insert_nested`, `set_nested_target`, `set_follow_length` (ADR-045) |
| Mídia (`capia-media`) | ✅ M07: probe via `ffprobe` atrás do trait `MediaProbe`, metadados normalizados (Ticks/Rational), localização do backend, processos com timeout e teto de saída, miniatura (ADR-047) |
| Assets (`capia-assets`) | ✅ M07: identidade por conteúdo (`sha256:` em streaming), import, dedup, online/offline/modified, relink por conteúdo, cache derivado descartável (ADR-046/048/049) |
| Catálogo de mídia no `.capia` | ✅ M07: schema 2 (`media_assets`, `asset_events`), migration real v1→v2, import atômico (documento + catálogo na mesma transação) |
| Composição de nested | ✅ M07: `duplicate_sequence`, `make_unique`, `flatten_nested`, `create_nested_from_selection`, `generate_variants`, `delete_asset` (ADR-050) |
| Frame index/decode, jobs, render, preview, export | ❌ ainda não (restante da Fase 2) |
| CI | ✅ executa em runner real desde a M05 (ver "Validação"); jobs Linux verdes incl. perf e paridade WASM |
| Decisões abertas | **OD-1** (gate da Fase 3) · **OUTPUT-H264** (antes da entrega do Editor). **As 8 decisões S7 estão definitivas** (ADR-039) |
| Git | `main` = M01–M04; M05 em `claude/m05-core-engine`; M06 em `claude/m06-persistence-cli`; M07 em `claude/m07-assets-media` (sem PR aberto: não solicitado) |

## O que existe (M07)

**Identidade (ADR-046):** `AssetId = ast_` + 32 hex do `content_hash`; `content_hash = sha256:<64 hex>` do arquivo **inteiro**, em blocos de **1 MiB** (nunca na memória; teste com leitor-espião prova que nenhuma leitura pede mais que um bloco). O caminho nunca identifica nada. **Dedup:** um asset por conteúdo por projeto (índice único no banco): (A) mesmo arquivo/mesmo path → `existing`, sem escrita; (B) mesmo conteúdo/path novo → `aliased` (alias em `known_paths`) ou, se o asset estava offline/modificado, `relinked` automático; (C) conteúdo diferente/mesmo nome → outro asset; (D) arquivo alterado depois do import → `verify` detecta `modified` (por hash; tamanho diferente já aparece sem hash; mtime nunca decide); (E) alias bom + path principal sobrescrito → o **alias vence** (`quick_status`/`verify_content` preferem o candidato com o tamanho/hash conhecido; determinístico, testado).
**Disponibilidade (ADR-048):** `online | offline | modified`, **derivada do disco** (nunca editada); abrir o projeto custa só `metadata` por asset e nunca falha por mídia ausente; os clips continuam no documento. **Relink** só aceita o **mesmo hash** (`ASSET_HASH_MISMATCH` estruturado com os dois hashes; nada muda); **não existe *force* na M07**. Relink/verify não são comandos do engine (não entram no undo) — ficam na trilha `asset_events`.
**Schema 2 / migration v1→v2:** tabelas `media_assets` (hash único, `json_valid`, `CHECK`s) e `asset_events` (append-only); aditiva, com backup `VACUUM INTO`. Testada sobre um projeto schema 1 **real** (histórico, ramo de redo e `operation_id`s intactos; undo/redo funcionam depois), com falha no meio (rollback), cadeia v1→v2→v3 sintética, schema futuro rejeitado sem tocar o arquivo e **queda de processo no meio da migration** (o arquivo continua v1 íntegro).
**Import atômico:** hash → probe → classificação **antes** de qualquer escrita; o efeito no catálogo viaja na MESMA transação SQLite do `register_asset` (fila drenada pelo `Journal`). Probe/arquivo ruins não deixam nada; escritor obsoleto não deixa linha de catálogo; crash real em 5 estágios ⇒ tudo ou nada nas três camadas (documento, log de operações, catálogo).
**Mídia (ADR-047):** descoberta do backend: caminho configurado → `CAPIA_FFPROBE`/`CAPIA_FFMPEG` → diretório *bundled* (`<exe>/ffmpeg`) → `PATH`; caminho explícito inexistente é erro (não cai para o `PATH`); nada achado ⇒ `MEDIA_BACKEND_NOT_FOUND`. Processo sem shell, caminho como UM argumento `file:<abs>` após `-i`, `-protocol_whitelist file`, **timeout 30 s** (filho morto de verdade — teste confere que o pid some), **stdout ≤ 8 MiB e stderr ≤ 64 KiB** (flood ⇒ processo morto, `MEDIA_PROBE_OUTPUT_TOO_LARGE`), demuxers de playlist/rede rejeitados. Normalização: Ticks por aritmética inteira, `Rational`, streams ordenados, stream padrão explícito (primeiro vídeo que não é capa; primeiro áudio), limites de plausibilidade, rotação, alpha só quando declarado; valores hostis ⇒ `None`/erro estruturado, nunca pânico.
**Cache (ADR-049):** `<projeto>.capia-cache/`, `CacheKey` = SHA-256 de (hash de conteúdo, operação, parâmetros ordenados, produtor+versão do ffmpeg), escrita atômica, **descartável** (apagar não altera o `.capia`; `validate` segue ok; a miniatura regenera). Miniatura: `generate_thumbnail(asset, timestamp)` (PNG, ≤ 1024 px); recusa arquivo com hash diferente do catálogo.
**CLI:** `capia asset import|list|inspect|verify|relink|thumbnail` e `capia media probe` (`--json`, `--ffprobe`, `--ffmpeg`, `--timeout-ms`); erros estruturados, saída 0/1/2.
**Composição (ADR-050):** `duplicate_sequence` (raso ou `deep`), `make_unique`, `flatten_nested`, `create_nested_from_selection`, `generate_variants` (estrutural, sem IA) e `delete_asset` (`IN_USE`). Ids derivados (`S'.X` / `R'~D`), atômicos, com undo/redo/idempotência/persistência; ciclo e profundidade guardados em duas camadas.

## Matriz de requisitos da M07

| Requisito | Estado | Teste que prova |
|---|---|---|
| `capia-media` (probe, normalização) | ✅ | `capia-media`: 21 unit + 14 `real.rs` (goldens), CLI `media_probe_*` |
| `capia-assets` | ✅ | `capia-assets`: 8 unit + 10 `assets.rs` + 2 `real_media.rs` |
| ffprobe abstraído (`MediaProbe`) | ✅ | `FfprobeBackend`, `fake_backend` (travado/flood/JSON lixo/backend ausente), `StaticProbe` nos testes de lógica |
| Descoberta do backend / `MEDIA_BACKEND_NOT_FOUND` | ✅ | `toolchain::tests`, CLI `missing_backend_is_a_structured_error_not_a_panic` |
| Timeout, filho morto, teto de stdout/stderr, sem shell | ✅ | `process::tests` (Unix **e** Windows), `the_child_is_really_gone_after_a_timeout`, `arguments_are_never_interpreted_by_a_shell`, `probe_args` |
| Fixtures reais | ✅ | `tests/fixtures/media` (7 arquivos, 136 KiB, geradas por `tools/gen-media-fixtures.sh`, determinísticas) |
| Hash em streaming | ✅ | `never_asks_for_more_than_one_block`, `streaming_across_block_boundaries…`, 48 MiB, mutação M7 |
| Deduplicação | ✅ | `identity_is_content_not_path_or_name`, `dedup_by_content…`, índice único |
| Import atômico | ✅ | `failed_imports_leave_nothing_behind`, `a_stale_writers_import_leaves_no_trace…`, crash `killing_an_import_at_every_stage…`, `a_failing_catalog_effect_rolls_back…` |
| Schema 2 + migration v1→v2 real | ✅ | `schema.rs` (cadeia v1→v2→v3, histórico/redo/`operation_id`/undo pós-migration), crash `…in_the_middle_of_a_migration…` |
| Assets persistidos | ✅ | `identity_survives_close_and_reopen`, propriedade com reabertura |
| Offline | ✅ | `offline_media_does_not_prevent_opening_and_keeps_the_clips` |
| Verify / modified | ✅ | `verify_detects_modified_content_not_just_missing_files`, `a_good_alias_beats_an_overwritten_primary_path` |
| Relink (ok e rejeitado) | ✅ | `relink_by_content_and_structured_rejection_of_other_content` + CLI |
| Cache / thumbnail | ✅ | `cache_keys_are_deterministic…`, `thumbnail_goes_to_the_cache…` (apagar cache → `validate` ok → regenera) |
| Clips → `AssetId`; referência inválida; asset em uso | ✅ | `clips_reference_assets_and_in_use_assets_cannot_be_deleted` |
| CLI de assets | ✅ | `capia-cli/tests/assets.rs` (6) |
| Nested restante (5 comandos) | ✅ | `compose.rs` (24) + propriedade |
| Propriedade (assets + persistência + composição) | ✅ | `properties_assets.rs` (modelo de referência), `properties` do store com composição |
| Performance 10k assets | ✅ | `perf_assets.rs` (números abaixo) |
| Linux / Windows / WASM | ver relatório de entrega (run final) | job Linux inclui WASM e paridade |
| Imagem: width/height/format/alpha | ✅ | `images()` (PNG com alfa, JPEG) |
| *Force-relink* | ⏳ **adiado** (o enunciado o deixa para depois) | — |
| Rotação em fixture real | ⏳ coberta só por JSON sintético (o ffmpeg não grava Display Matrix de forma portável) | `rotation_from_display_matrix_is_clockwise_and_validated` |

## Validação M07 (container Linux, Rust 1.97, ffmpeg/ffprobe 6.1.1-3ubuntu5)

| Verificação | Resultado |
|---|---|
| `cargo fmt --check` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` | ✅ · ✅ · ✅ **267 testes** (+ 5 de desempenho `#[ignore]`; M06 tinha 163) |
| Propriedade com IO (release, 5.000 casos) | ✅ timeline: 70.000 comandos (41.763 aceitos), 21.374 reaberturas · nested+composição: 70.000 (42.791), 21.401 reaberturas · **assets: 5.000 × 40 passos** (7.456 clips, 10.518 imports criados, 4.624 `existing`, 1.187 `aliased`, 356 relinks automáticos, 1.495 reregistros, 1.991 relinks ok × 3.691 rejeitados, 1.274 `modified`, 3.462 composições, 4.851 undo/redo, **5.949 reaberturas**) — 136 s |
| Propriedade nested em memória | ✅ 3.000 casos, ≈ 24.8 mil comandos aceitos, 4.914 edições nested, **4.755 comandos de composição** |
| Mutação manual (22) | ✅ 19 detectadas: relink sem hash, `AssetId` inexistente aceito, offline ignorado, `make_unique` reaproveitando ids, timeout removido, hash em memória, verify só por tamanho, **catálogo fora da transação**, dedup que nunca acha, `flatten` ignorando a janela, `delete_asset` ignorando uso, prefixo `file:` removido, saída sem teto, **probe falho ignorado (asset sem probe)**, **cache ignorando o hash (cache como permanente)**, **modified reportado como online**, **migration parcial / sem `user_version`**, relink que aceita qualquer arquivo. As 3 restantes são a remoção de **uma** camada de proteção de ciclo/profundidade — sobrevivem *por desenho* (defesa em profundidade); removendo as **duas** camadas juntas os testes de `compose`, `nested` e `nested_graph` falham |
| M05/M06 preservados | ✅ aceitação 120/120, golden digest estável, paridade WASM (150 sequências), `cargo check wasm32-unknown-unknown`, crash tests |
| `pnpm lint/format:check/typecheck/test/test:tools`, `check:arch`, `check:parity`, `cargo deny` | ✅ |

### Desempenho (release, `synchronous=FULL`, Linux, VM sem SHA-NI)

| Medida | Resultado |
|---|---|
| Hash SHA-256 streaming, 256 MiB | 2,6 s ≈ **97 MiB/s** (outlier: esta VM não tem SHA-NI; com aceleração de hardware espera-se ≥ 10× — ver limitações) |
| Hash da fixture de 7,5 KiB | 0,06 ms |
| `ffprobe` da fixture (mediana de 20) | **62 ms** (custo dominante do import) |
| Import completo (hash + ffprobe + commit atômico, mediana de 10) | **68 ms** |
| Miniatura (cache frio / quente) | **83 ms** / 0,02 ms |
| 10.000 assets: 10k `register_asset` num commit durável / 10k linhas de catálogo + eventos | 309 ms / 764 ms |
| Arquivo `.capia` com 10.000 assets | 20,4 MiB |
| **Abrir** (documento + catálogo) / reabrir após import | **299 ms** (meta < 2 s) / 308 ms |
| **Listar** 10k (leitura do catálogo + 10k `stat`) | **207 ms** |
| Lookup por `AssetId` / por hash (média de 100) | 0,05 ms / 0,07 ms |
| `verify` de um asset (stat + hash + evento) | 2,4 ms |
| Import num projeto de 10k assets | **6,5 ms** (meta < 200 ms) |
| Persistência de 10k clips (M06, mesma máquina) | abrir 74 ms · commit comum mediana 9 ms · commit com snapshot 135 ms |

### Estratégia de CI e tempo do Windows (ADR-051)

O `cargo test --workspace` do Windows levava ≈ **17,7 min** na M06 (propriedades com fsync no NTFS). Na M07: **propriedades com IO** leem `CAPIA_IO_PROP_CASES` — Windows usa um orçamento *smoke* (100 casos), o Linux roda o orçamento pesado em **release** (5.000 por gerador, ≈ 6 min no total) mais os benchmarks. O Windows continua rodando integralmente: crash tests reais (incl. migration e import), store, schema/migrations, mídia/assets com FFmpeg instalado via Chocolatey (versão impressa no log; `CAPIA_REQUIRE_FFMPEG=1`), CLI, lógica pura e o build desktop. Os tempos do último run estão no relatório de entrega.

### Achados da auditoria M07 (corrigidos)

- **Fixtures de mídia não estavam no Git** (`.gitignore` bloqueava `*.mp4`/`*.wav`): o CI vermelho revelou; exceção deliberada para `tests/fixtures/media/`.
- ETXTBSY ao executar um backend recém-escrito (corrida de `fork`): retry limitado.
- `join` das threads de leitura travava até o neto do processo fechar o pipe: leitores desacoplados com espera curta.
- `quick_status`/`verify_content` olhavam só o 1º candidato: um alias íntegro não valia nada se o principal fosse sobrescrito → agora prefere o candidato com o tamanho/hash conhecido.
- `Project::asset` montava a lista inteira (O(n)) → leitura direta (O(1), 0,05 ms em 10k).
- Testes só existiam para Unix (`/bin/sh`): adicionados equivalentes Windows para processos e caminhos (drive letters, Unicode, separadores).
- `file:`-prefix e `-protocol_whitelist`: protegidos por teste de argumentos (o caminho absoluto já impede `concat:` relativo; é defesa em profundidade).

### Limitações restantes (M07)

- **Hash completo síncrono no import:** 97 MiB/s nesta VM significa ≈ 100 s para um vídeo de 10 GB sem aceleração SHA-NI; o desenho de `ASSET_SYSTEM.md` (fingerprint amostrado rápido + hash completo em job) fica para a missão de jobs.
- Sem *force-relink*, sem relink em lote por pasta, sem estado `Missing`; `known_paths` guarda texto (caminhos não-UTF-8 viram *lossy* nos aliases).
- `asset_events` cresce a cada `verify` (sem poda).
- O cache de miniatura é chaveado pelo hash do **catálogo**: se o arquivo mudar sem `verify`, um acerto de cache devolve a miniatura do conteúdo original (a geração nova recusa hash diferente).
- Rotação/Display Matrix só testada com JSON sintético; golden de metadados compara campos estáveis entre versões do FFmpeg (a versão do CI é registrada).
- Frame index/decode, proxies, waveform, jobs, render, preview e export continuam fora de escopo.

## O que existe (M06)

**Formato `.capia` v1** (ADR-042): um arquivo SQLite (tabelas `STRICT`): `meta`, `schema_migrations`, `history_entries`, `events`, `applied_operations`, `commit_results`, `snapshots`; triggers *append-only*; `application_id = 0x43415049`. O documento **não** é espalhado em tabelas: estado = último snapshot (JSON canônico + digest) + replay dos eventos posteriores (ADR-043: histórico completo persistido, undo/redo funcionam após reabrir).
**Commit** (ADR-044): entrada de histórico + resultado + `operation_id`s + evento (+ snapshot no cadence) numa única `BEGIN IMMEDIATE`; o engine persiste **antes** de publicar (falha ⇒ memória intacta, `PERSISTENCE_FAILED`). Escritor único pelo lock do SQLite + checagem de *stale head* (`STORE_BUSY` / `STORE_CONFLICT`).
**PRAGMAs:** WAL · `synchronous=FULL` (padrão) · `foreign_keys=ON` · `busy_timeout=5000` · `trusted_schema=OFF` · `cell_size_check=ON` · `quick_check` ao abrir, `integrity_check`+`foreign_key_check` em `validate`.
**Migrations:** só para frente, cada uma em transação própria, backup `VACUUM INTO` (`<arq>.v<N>.bak`) antes de migrar; schema mais novo é **rejeitado sem tocar o arquivo**. Testadas com versão 2 sintética (sucesso, falha com rollback, lacuna, downgrade).
**Erros estruturados:** `PROJECT_NOT_FOUND` · `PROJECT_ALREADY_EXISTS` · `NOT_A_CAPIA_PROJECT` · `PROJECT_CORRUPTED` · `UNSUPPORTED_SCHEMA_VERSION` · `MIGRATION_FAILED` · `STORE_IO_ERROR` · `STORE_BUSY` · `STORE_CONFLICT` · `TRANSACTION_FAILED` · `INVALID_ARGUMENT`. Nenhum erro bruto do SQLite escapa.
**CLI** (`capia`): `create` · `inspect` · `validate` · `apply` · `undo` · `redo` · `history` · `dump` (JSON determinístico). Usa o mesmo parser (`capia_project::parse_transaction`) e o mesmo engine; atores `agent`/`api` passam por `preview → apply_plan`. Saída 0 ok · 1 falha (JSON estruturado em stderr) · 2 uso.
**Nested (ADR-045):** `delete_sequence` · `rename_sequence` · `insert_nested` · `set_nested_target` · `set_follow_length`; ciclo (A→A, A→B→A), profundidade máx. 16 e referência inválida rejeitados; `follow_length` reconciliado após cada comando (magnética: ripple; livre: limitado ao espaço com aviso; travada: intocada com aviso).

## Validação M06 (container Linux, Rust 1.97)

| Verificação | Resultado |
|---|---|
| `cargo fmt` · `clippy --workspace --all-targets -D warnings` · `cargo test --workspace` | ✅ · ✅ · ✅ **163 testes** (time 25 · model 20+5+4 · commands 9+21+1+18+2+2+2 (+1 acc.+1 golden) · store 11 schema + 18 store + 5 crash + 2 properties · cli 12+4 · project 1 …) + 4 de desempenho `#[ignore]` |
| Crash real (`tests/crash.rs`) | ✅ filho morto por `abort` e por `kill` externo em `before_begin` / `in_tx_after_entry` / `in_tx_after_writes` / `before_commit` / `after_commit` (10 combinações): sempre **estado A completo ou B completo**, `integrity_check` ok, log coerente, reenvio idempotente; + 14 rodadas de SIGKILL em instantes aleatórios com snapshots; repetido 15× sem flakiness |
| Propriedade (lockstep persistido × memória, reabrindo aleatoriamente) | ✅ 1.000 sequências por gerador (timeline e nested) por padrão (~90 s em debug por ser I/O com fsync); **CI release: 5.000 por gerador (10.000 no total, ~147 s)**. Rodada medida: 21.000 comandos / 6.395 reaberturas (timeline), 21.000 / 6.368 (nested). Orçamento escolhido por custo de I/O; `CAPIA_PROP_CASES` ajusta |
| Propriedade nested em memória | ✅ 3.000 casos, 25.586 comandos aceitos, 6.103 edições nested |
| Corrupção (`tests/schema.rs`) | ✅ vazio, não-SQLite, SQLite sem schema, schema novo, blob inválido, registro inconsistente, ids inválidos, 300 rodadas de *byte-fuzz*: sempre erro estruturado, **nunca pânico, arquivo intocado** |
| Concorrência | ✅ 2 escritores: `STORE_BUSY`/`STORE_CONFLICT`; threads com retry nunca corrompem e cada escrita entra exatamente uma vez |
| Desempenho (`--release`, 10.000 clips, `synchronous=FULL`, arquivo 2,4 MiB) | ✅ criar 238 ms · **abrir 81 ms** (meta < 2 s) · recarregar 68 ms · **commit comum mediana 7,2 ms, pior 10 ms** (meta < 200 ms) · commit com snapshot de 10 k clips 118 ms · abrir com cauda de 300 eventos 83 ms |
| Mutação manual (11) | ✅ todas detectadas: `operation_id` duplicado ignorado, erro de insert engolido, evento fora da transação, validação de ciclo/profundidade removida, stale-head removido (defesa em profundidade: a PK também barra), digest do snapshot sem checagem, publicar antes do journal, ids não persistidos, cursor de undo não restaurado, `follow_length` desligado, `synchronous` enfraquecido |
| M05 preservado | ✅ aceitação 120/120, golden digest estável, paridade WASM, `cargo check wasm32-unknown-unknown` |
| CI no GitHub (Linux + Windows) | ver relatório final |

**Achados da auditoria (corrigidos):** carga inconsistente (snapshot e eventos de instantes diferentes) → leitura em transação única; *peek* criava sidecars `-wal/-shm` → leitura crua do cabeçalho; `applied_operations` sem resultado não era detectado → cruzamento em `load`; aritmética sem *checked* sobre revisão/seq vindas do arquivo → `checked_add` e *guards*; blobs gigantes de arquivo hostil → `max_blob_bytes`.

### Limitações restantes (M06)

- Histórico completo e todos os `operation_id` são carregados em memória ao abrir (sem retenção/poda).
- O limite de 512 MiB por blob só foi testado com limite configurado pequeno.
- Custo de fsync por commit (`FULL`); `Normal`/`Off` só para testes de volume.
- Se `tx.commit()` falhar *depois* de o banco ter commitado, a memória fica atrasada; a reabertura recupera (`STORE_CONFLICT`).
- Planos (`preview`) não são persistidos (por desenho).
- Mover clips para track magnética continua `UNSUPPORTED_COMMAND`; permissões por ator continuam adiadas.
- Crash tests foram executados em Linux localmente; Windows roda na CI.

## O que existe (M05)

**Comandos (v1):** `register_asset` · `create_sequence` · `add_track` · `set_track_flags` · `delete_track` · `add_marker` · `move_marker` · `delete_marker` · `insert_clip` · `move_clips` (estrito) · `delete_clip` · `trim_clip` · `split_clip` · `set_clip_speed` · `set_property` · `add_keyframe` · `move_keyframe` · `delete_keyframe` · `set_keyframe_interp`.
**Funções puras de UX/engine:** `resolve_placement` · `resolve_snap` · `threshold_ticks` · `resolve_group_move` · `eval_property`.
**Garantias testadas:** toda escrita passa pelo Engine; `Agent`/`Api` só por preview (`PREVIEW_REQUIRED`); erro ⇒ documento idêntico; `apply∘undo = id`, `undo∘redo = id`; replays nunca duplicam edições; token adulterado/expirado/de outro ator/outra chave é rejeitado; plano só aplica se o `diff_digest` recomputado for idêntico.

### Ainda NÃO implementado (escopo declarado fora da M05)

- Comandos: pastas/organização, `duplicate_sequence`, `reorder_*`, `duplicate_clip`, `replace_clip_media`, `freeze_frame`, `detach_audio`, nested restante (`create_nested_from_selection`, `make_unique`, `flatten_nested`, `generate_variants`), efeitos, transições, texto/legendas, grupos de clips, clipboard, deliverables, comandos de assets além de `register_asset`.
- Mover clips **para/de track magnética** (semântica de *reorder*, Fase 3) → `UNSUPPORTED_COMMAND`.
- Caminhos de ref (`$seq.tracks.main`); só `$nome` simples.
- `permissions` por ator (`PERMISSION_DENIED`; Fase 4) e gate humano de aprovação do plano (`plan_id + diff_digest`).
- Coalescing por `gesture_id`; undo seletivo por ator (Fase 5); `Sequence.revision` individual.
- Conflitos em nível de campo (hoje por entidade — ADR-040).

## Missões

| Missão | Resultado |
|---|---|
| M01 — Fundação arquitetural | ✅ docs + `.gitignore` |
| M02 — Auditoria open-source | ✅ `OPEN_SOURCE_AUDIT.md`; estratégia C |
| M03 — Fechamento e spikes S1–S7 | ✅ S2–S7 medidos; ADR-029..036; `PROVENANCE.md`; `tests/acceptance` (108 cenários) |
| M04 — Scaffold, CI e preparação | ✅ workspace Rust+TS, shell Tauri, CI, licenças/arquitetura, pacote S1, ADR-037/038 |
| **M05 — Core Engine Foundation (Fase 2)** | ✅ tempo, modelo, Command Engine, suíte de aceitação 120/120, propriedade 10.000, idempotência/plano, paridade WASM, ADR-039..041 |
| **M06 — Persistência, nested e CLI (Fase 2)** | ✅ `capia-store`, `capia-project`, `capia-cli`, comandos nested, crash tests reais, propriedade com salvar/reabrir, ADR-042..045 |
| **M07 — Assets, mídia e composição (Fase 2)** | ✅ `capia-media`, `capia-assets`, catálogo no `.capia` (schema 2), import atômico, offline/verify/relink, cache + miniatura, CLI de assets, 5 comandos de composição, ADR-046..051 |

## Validação M05 (saída real; container Linux, Rust 1.97.0, Node 22, pnpm 10.28)

| Verificação | Resultado |
|---|---|
| `cargo fmt --all -- --check` · `cargo clippy --workspace --all-targets -- -D warnings` | ✅ · ✅ sem avisos |
| `cargo test --workspace` | ✅ **84 testes** (time 25 · model 20 · commands 36 · project 2 · desktop 1) + 2 de desempenho `#[ignore]` |
| Aceitação (`tests/acceptance.rs`) | ✅ **120 cenários** (placement 19 · snapping 19 · ripple 25 · retime 19 · group move 15 · keyframes 23) |
| Mutação na suíte (3 defeitos injetados no engine: limiar de snap exclusivo, obstáculo inclusivo no group move, arredondamento por truncamento) | ✅ 3/3 detectados (1, 1 e 3 cenários falham) |
| Propriedade (`tests/properties.rs`) | ✅ **10.000 sequências × 8 comandos** (≈ 50 mil comandos aceitos: insert 19,4 k · trim 4,8 k · delete 4,6 k · keyframe 3,7 k · split 2,7 k · speed 2,4 k · move 0,5 k …); determinismo de replay em 300 sequências × 12 comandos |
| Engine (`tests/engine.rs`) | ✅ 21 testes: replay, `OPERATION_ID_REUSED/CONFLICT`, undo não libera ids, `PREVIEW_REQUIRED`, token adulterado/expirado/outro ator/outra chave/store limitado, drift (rebase seguro × `PLAN_STATE_CHANGED`), `CONFLICT` por `base_revision`, refs, `max_ops`, histórico, `RIPPLE_CONFLICT` estruturado, track travada |
| Robustez (`tests/robustness.rs`) | ✅ ~900 comandos com valores extremos (limites de i64/f64, NaN, ±∞, parâmetros de Bézier) e JSON malformado: só erro estruturado ou commit válido e desfazível; **achado e corrigido na revisão:** overflow de `i128` (pânico em debug / wrap em release) em `mul_div_round`, soma/subtração de `Rational`, escala de frames e `threshold_ticks` — agora aritmética *checked* |
| SHA-256/HMAC próprios | ✅ vetores FIPS 180-4 (incl. 1 M de `a`) e RFC 4231 (casos 1, 2, 6) |
| Desempenho (`--release`, modelo em memória) | ✅ transação de **500 ops em 10.000 clips ≈ 16 ms** (meta < 200 ms) · carregar + indexar + validar 10.000 clips ≈ **17 ms** (meta < 2 s) |
| Paridade nativo × WASM (`pnpm check:parity`) | ✅ 150 sequências aleatórias: digests do documento **idênticos** entre nativo e `wasm32-wasip1` (WASI do Node) — cobre floats de Bézier e JSON |
| Golden digest (`tests/golden.rs`) | ✅ trava a semântica do engine (25 sequências) |
| WASM: `cargo check --target wasm32-unknown-unknown -p capia-time -p capia-model -p capia-commands` | ✅ |
| `pnpm lint` · `format:check` · `test:tools` (15) · `check:arch` · `cargo deny check licenses bans sources` | ✅ ✅ ✅ ✅ ✅ |
| CI no GitHub (runner real) | `main`: M04 falhou **uma vez** no `gitleaks-action` (range inválido com commit raiz) → corrigido (CLI sobre o histórico completo) → **verde**. Branch `claude/m05-core-engine`: jobs Linux (núcleo + perf + paridade, TypeScript, políticas/segredos) ✅ — resultado do job Windows no relatório final |

### Limites do que foi validado

- A suíte de aceitação foi **escrita à mão a partir da especificação**, não gerada pelo engine; porém os 12 cenários novos e a atualização de 4 são da M05 e foram escritos com o mesmo método (valores derivados à mão — ver `provenance`).
- Conflitos de plano são detectados por **recomputação + digest**; não há teste de concorrência real (o engine é *single-writer* por construção; o ator de projeto vem com o `capia-store`/`capia-engine`).
- Desempenho medido só no **modelo em memória**; "abrir projeto" com SQLite será medido com o `capia-store`.
- Paridade WASM por **digest do documento**, não por execução no navegador (WebView/V8); o WASI do Node usa o mesmo motor V8.
- Teste de *crash* (kill no meio do commit) **não** existe ainda (não há persistência).

## OUTPUT-H264 — risco técnico explícito (NÃO resolvido)

Antes da **entrega do Editor** (saída da Fase 3) é preciso um caminho **confiável** de exportação **MP4/H.264 no Windows**:

- **Encoder de hardware** quando disponível (NVENC/AMF/QSV);
- **Media Foundation / FFmpeg** quando adequado (`h264_mf`, builds LGPL — ADR-032);
- **Fallback por software legal e de qualidade aceitável**. Sem x264/x265 GPL no build padrão.

*Sabido (S3):* as builds LGPL trazem NVENC/AMF/QSV/MF; o fallback por software disponível é OpenH264 (**só Constrained Baseline**; PSNR 46,9 dB vs 53,0 dB do x264 no mesmo bitrate em conteúdo **sintético**); o binário OpenH264 compilado do fonte não tem a cobertura de patentes da Cisco.
*Desconhecido:* qualidade/velocidade em vídeo real; disponibilidade de encoder de hardware na máquina mínima (iGPU); comportamento do Media Foundation; política de download/licença do binário OpenH264; patentes H.264/HEVC/AAC (decisão jurídica/comercial).
*Responsável:* Product Owner (com apoio jurídico para patentes). *Sugestão:* spike dedicado em Windows (mesma sessão do S1). *Estado:* **ABERTO**.

## Próxima missão (proposta — NÃO iniciada)

**M08 — Jobs e índice de mídia:** (1) `capia-jobs` mínimo (fila local, cancelamento, progresso) para tirar o hash completo do caminho síncrono (fingerprint amostrado rápido + hash completo em background, como em `ASSET_SYSTEM.md` §4); (2) `capia-media`: *frame index* e *decode* de frames exatos (ADR-035) sobre o trait `MediaProbe` já existente; (3) waveform e proxy como segunda e terceira operações derivadas no mesmo `CacheKey`; (4) *force-relink* e relink em lote por pasta. Depois: `capia-render` (compositor) → export. Não iniciar a Fase 3 antes de OD-1.

## Blockers

1. **S1 em Windows** — gate da Fase 3 (não bloqueia a Fase 2).
2. **OUTPUT-H264** — antes da entrega do Editor.
3. *(resolvido na M05)* Push ao GitHub (403) e primeira execução do CI.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs da missão.
- Nenhum código de terceiros incorporado (`docs/PROVENANCE.md` §4 vazio). SHA-256/HMAC foram escritos do zero a partir de FIPS 180-4/RFC 2104/4231.
- Ao concluir uma missão, atualize esta página.
