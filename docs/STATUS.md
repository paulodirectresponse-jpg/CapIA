# STATUS

**Última atualização:** 2026-10-03 · **Fase atual:** FASE 2 — Motor (headless) · **Missão corrente:** M06 concluída (Persistência `.capia` + CLI + nested) · **Próxima:** M07 (ver "Próxima missão")

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
| Assets, mídia, render, preview | ❌ ainda não (restante da Fase 2) |
| CI | ✅ executa em runner real desde a M05 (ver "Validação"); jobs Linux verdes incl. perf e paridade WASM |
| Decisões abertas | **OD-1** (gate da Fase 3) · **OUTPUT-H264** (antes da entrega do Editor). **As 8 decisões S7 estão definitivas** (ADR-039) |
| Git | `main` = M01–M04; M05 em `claude/m05-core-engine`; M06 em `claude/m06-persistence-cli` (sem PR aberto: não solicitado) |

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

## Próxima missão (sugestão)

**M07 — Comandos nested restantes e início de assets:** `make_unique`, `flatten_nested`, `duplicate_sequence`, `create_nested_from_selection`, `generate_variants` (sobre o modelo/journal já prontos); depois `capia-assets` → `capia-media` (probe/frame index/decode) → `capia-render` (compositor wgpu) → export (critério 1 da Fase 2). Não iniciar Fase 3 antes de OD-1.

## Blockers

1. **S1 em Windows** — gate da Fase 3 (não bloqueia a Fase 2).
2. **OUTPUT-H264** — antes da entrega do Editor.
3. *(resolvido na M05)* Push ao GitHub (403) e primeira execução do CI.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs da missão.
- Nenhum código de terceiros incorporado (`docs/PROVENANCE.md` §4 vazio). SHA-256/HMAC foram escritos do zero a partir de FIPS 180-4/RFC 2104/4231.
- Ao concluir uma missão, atualize esta página.
