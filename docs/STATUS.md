# STATUS

**Última atualização:** 2026-10-03 · **Fase atual:** FASE 2 — Motor (headless) · **Missão corrente:** M05 concluída (Core Engine Foundation) · **Próxima:** M06 (ver "Próxima missão")

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
| Persistência `.capia`, assets, mídia, render, preview, CLI | ❌ ainda não (restante da Fase 2) |
| CI | ✅ executa em runner real desde a M05 (ver "Validação"); jobs Linux verdes incl. perf e paridade WASM |
| Decisões abertas | **OD-1** (gate da Fase 3) · **OUTPUT-H264** (antes da entrega do Editor). **As 8 decisões S7 estão definitivas** (ADR-039) |
| Git | `main` = M01–M04 + correção de CI; M05 em `claude/m05-core-engine` (sem PR aberto: não solicitado) |

## O que existe (M05)

**Comandos (v1):** `register_asset` · `create_sequence` · `add_track` · `set_track_flags` · `delete_track` · `add_marker` · `move_marker` · `delete_marker` · `insert_clip` · `move_clips` (estrito) · `delete_clip` · `trim_clip` · `split_clip` · `set_clip_speed` · `set_property` · `add_keyframe` · `move_keyframe` · `delete_keyframe` · `set_keyframe_interp`.
**Funções puras de UX/engine:** `resolve_placement` · `resolve_snap` · `threshold_ticks` · `resolve_group_move` · `eval_property`.
**Garantias testadas:** toda escrita passa pelo Engine; `Agent`/`Api` só por preview (`PREVIEW_REQUIRED`); erro ⇒ documento idêntico; `apply∘undo = id`, `undo∘redo = id`; replays nunca duplicam edições; token adulterado/expirado/de outro ator/outra chave é rejeitado; plano só aplica se o `diff_digest` recomputado for idêntico.

### Ainda NÃO implementado (escopo declarado fora da M05)

- Persistência `.capia` (SQLite), `applied_operations`/histórico em disco, teste de *kill -9* no commit (depende do `capia-store`).
- Comandos: pastas/organização, `duplicate/delete/rename_sequence`, `reorder_*`, `duplicate_clip`, `replace_clip_media`, `freeze_frame`, `detach_audio`, nested (`create_nested_from_selection`, `make_unique`, `flatten_nested`, `set_follow_length`, `generate_variants` — o **modelo** e as invariantes de nested/ciclo/profundidade já existem), efeitos, transições, texto/legendas, grupos de clips, clipboard, deliverables, comandos de assets além de `register_asset`.
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

**M06 — Persistência e Engine API headless (resto do critério 1 da Fase 2):**
1. `capia-store` (arquivo `.capia` = SQLite): documento, `applied_operations` e histórico **na mesma transação** do commit; autosave por transação; migrations; recovery; **teste de kill -9 no meio do commit** (fecha o item de idempotência da Fase 2) e abrir 10.000 clips < 2 s.
2. `capia-cli` mínimo sobre a Engine API: criar projeto com 3 sequences (2 hooks + `BODY_MASTER` *nested*), transações, undo/redo (script reproduzível).
3. Comandos de nested (`create_nested_from_selection`, `make_unique`, `flatten_nested`, `set_follow_length`) e `generate_variants`, usando o modelo/invariantes já prontos.
Depois: `capia-assets` → `capia-media` (probe/frame index/decode) → `capia-render` (compositor wgpu) → export. Não iniciar Fase 3 antes de OD-1.

## Blockers

1. **S1 em Windows** — gate da Fase 3 (não bloqueia a Fase 2).
2. **OUTPUT-H264** — antes da entrega do Editor.
3. *(resolvido na M05)* Push ao GitHub (403) e primeira execução do CI.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs da missão.
- Nenhum código de terceiros incorporado (`docs/PROVENANCE.md` §4 vazio). SHA-256/HMAC foram escritos do zero a partir de FIPS 180-4/RFC 2104/4231.
- Ao concluir uma missão, atualize esta página.
