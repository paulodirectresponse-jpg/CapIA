# Fase 6 — Track D-1: desempenho, soak e compatibilidade de migração

Implementação de `PHASE6_PERFORMANCE_SECURITY.md` §1–8, 23–24, 29 e `PHASE6_COMPLETION.md` §26–27 (parte do
engine; REST/MCP/servidor entram nas outras trilhas). Nada aqui entra no produto.

## 1. Fixture de projeto grande (`crates/capia-fixtures`)

Crate **dev-only** (`publish = false`; matriz de `tools/check-architecture.mjs`: só time/model/commands/store/
project/assets/media/editor-api; nenhum crate de produto pode depender dele fora de `[dev-dependencies]`, e a
aresta é ignorada no grafo de ciclos — o ciclo `project ⇄ fixtures` é de teste e o cargo o permite).

`build_large_project(path, &LargeSpec) -> LargeStats` produz um `.capia` **real** só por APIs públicas:

| Ingrediente | Como |
|---|---|
| Documento | `Project::execute` (Command Engine) em transações de 400 comandos: `CreateSequence/AddTrack/InsertClip/InsertNested/AddKeyframe/SetProperty/AddMarker/RegisterAsset` |
| Padrão (`LargeSpec::default()`) | 36 sequences, 6.000 clips (1.772 mídia, 461 texto, 1.477 legendas, 1.367 áudio, 45 nested, 166 com keyframes), 180 marcadores, 2.500 assets, cauda de 300 transações pequenas (347 entradas de histórico) |
| Nested | DAG de profundidade 3 (main → A → B); a sequence `seq_000` (1.800 clips) compõe 12 filhas |
| Catálogo | 2.500 registros sintéticos `offline` via `Catalog::apply_batch`: **nenhum arquivo de mídia** é gerado (o catálogo aceita caminhos inexistentes — é o estado de uma biblioteca com disco desmontado) |
| Histórico de IA | `AutonomyStore` público: 60 Runs (completed/failed/paused/cancelled), 426 stages, 1.278 eventos, 180 efeitos + livro de orçamento, proveniência e memória |
| Determinismo | SplitMix64 por `seed`; ids derivados; mesmo `LargeSpec` ⇒ mesmo `document_digest` (testado; outra semente ⇒ outro digest) |
| Custo | ≈ 2,4 s em release; arquivo ≈ 17,5 MiB; validado por `Project::validate` |

Também expõe `bench` (p50/p95 por posto mais próximo, `machine_info`, `Report` JSON com **amostras brutas**),
`proc` (VmRSS/fd/threads de `/proc/self`, `None` fora do Linux), `query::clips_in_range` (consulta de viewport,
oráculo ingênuo testado) e `api::ApiProbe` (`Session::call` medido).

## 2. Benchmarks (`crates/capia-project/tests/perf_large.rs`)

```
cargo test --release -p capia-project --test perf_large -- --ignored --nocapture --test-threads=1
```
Grava `target/perf/phase6-large-project.json` (`CAPIA_PERF_OUT` muda o diretório; `CAPIA_PERF_REPS` as
repetições, padrão 9; itens leves 40). Máquina de referência desta medição: **Intel Xeon 2,1 GHz, 4 vCPU, 16 GiB,
Linux 6.18 (VM), rustc 1.97.0, release, FFmpeg presente, cache de SO quente, `synchronous=FULL`**.

| Métrica | p50 | p95 | Meta / limite | Estado |
|---|---|---|---|---|
| `open_project` (doc + catálogo + jobs) | 211–272 ms | 270–323 ms | < 2 s (Fase 2) | ok |
| `reload_state` | 212–238 ms | 254–290 ms | < 2 s | ok |
| `commit_small` (durável FULL) | 2,6–4,0 ms | 3,0–5,5 ms | **< 30 ms** (TIMELINE_UX §6) | ok |
| `commit_bulk_50` | 4,9–6,7 ms | 6–7 ms | regressão | ok |
| `commit_with_snapshot` (pior caso) | 89–102 ms | 96–113 ms | regressão | ver §6 |
| `undo` / `redo` | 2,5–3,5 ms | 3,6–4,0 ms | **< 50 ms** | ok |
| `api_project_open` | ≈ 300 ms | ≈ 320–400 ms | regressão | ok |
| `api_project_snapshot` (36 seq + 2.500 assets) | 27–30 ms | 40–48 ms | regressão | ok |
| `api_sequence_get_biggest` (1.800 clips) | 4,7–5,0 ms | 5–9 ms | regressão | ok |
| `api_history_list` (347 entradas) | 15–17 ms | 19–20 ms | regressão | ok |
| `api_assets_list` (2.500) | 24 ms | 27 ms | regressão | ok |
| `timeline_range_query_60s` | 0,006 ms | 0,02 ms | **< 16 ms** | ok (≈ 70 clips/viewport) |
| `timeline_range_query_all` (1.800 clips) | 0,16–0,19 ms | 0,2–0,3 ms | < 16 ms | ok |
| `render_graph_compile_biggest` | 1,6 ms | 2–4 ms | regressão | ok |
| `preview_frame_cold` (1º quadro após abrir) | 35 ms | 43 ms | regressão | ok |
| `preview_frame_warm` (404×720, CPU) | 29–34 ms | 45–52 ms | **< 100 ms** (scrub) | ok (≈ 3,5 camadas ativas; 43 avisos `SOURCE_UNAVAILABLE` de fontes offline em 40 quadros) |
| `preview_frame_nested` (render recursivo) | 53–59 ms | 58–80 ms | < 100 ms | ok |
| `ai_runs_list` / `ai_runs_full_history_read` (60 runs) | 0,1 / 18 ms | 0,7 / 25 ms | regressão | ok |
| `export_queue_start` (8 itens) | 0,06–0,08 ms | 0,1–0,2 ms | regressão | ok |
| `export_queue_start_to_cancelled` (abre instância própria) | 250–280 ms | 290–340 ms | regressão | ok |

Pico de RSS do processo de benchmark ≈ 185 MiB. Comparação com a Fase 3 (`docs/STATUS.md`, 5.000 clips): engine
commit p50 4,3/p95 10,8 ms → agora 2,6–4,0/3,0–5,5 ms (projeto 6.000 clips e 2.500 assets); undo p95 8,2 → 3,6–4,0 ms;
scrub p95 37,8 ms (404×720, 5.000 sólidos) → 45–52 ms (conteúdo mais pesado: texto, nested e 3,5 camadas médias).
**Nenhuma meta regrediu**; nenhum limite foi relaxado.

Não medido aqui (e por quê): o commit **percebido na UI** (p95 34,9 ms na Fase 3, headless) — exige Chromium/E2E
(`packages/e2e`, fora desta trilha); REST/MCP e eventos de Run via servidor — dependem do crate de servidor
(outra trilha); export com encoder real — depende do FFmpeg/encoders aprovados. O custo medido da fila de export
é só o de orquestração (abrir instância própria + validar lote + cancelar).

## 3. Gate de regressão (`tools/phase6-acceptance/performance/`)

* `run.mjs` roda o benchmark **N = 3** vezes, toma a **mediana** de cada estatística (p50/p95) entre as execuções e
  compara com `thresholds.json`; escreve `target/phase6-acceptance/performance.json` (veredito por métrica, amostras
  por execução, máquina e fixture). `--reports dir…` avalia JSON existentes; `CAPIA_PERF_TOLERANCE` troca o fator.
* Dois tipos de limite. **`hard`**: meta documentada (30/50/100/16 ms, 2 s), **sem** fator de tolerância. **Regressão**:
  ≈ 3× o p95 medido, multiplicado por `tolerance_factor = 1.5` (runner compartilhado). A mediana de 3 execuções
  absorve um pico isolado; uma regressão real (≥ 2 execuções) reprova.
* Sem aprovação vazia: métrica ausente, `n` inválido, NaN, limite ausente ou métrica pulada sem o motivo permitido
  (`allow_skip`, hoje só "no ffmpeg" na fila de export) **reprovam**. `run.test.mjs` cobre isso (8 testes) e verifica
  que as metas `hard` nunca sejam afrouxadas acima do documentado.
* O `perf_large` em si só afirma sanidade (fixture ≥ 30 seq/5.000 clips, respostas com o conteúdo esperado, abrir/
  recarregar < 2 s, commit durável < 200 ms, range query < 16 ms, scrub p50 < 100 ms); as metas de undo/redo 50 ms e
  scrub p95 100 ms ficam no gate (um p95 único falhou por ruído de runner compartilhado numa execução — por isso a
  política de mediana).

Passo de CI sugerido (**não** ligado ao `ci.yml`; para o integrador), no job Linux que já tem Rust+Node e FFmpeg:

```yaml
      - name: Fase 6 — gate de desempenho (projeto grande)
        run: node tools/phase6-acceptance/performance/run.mjs
        env:
          CAPIA_REQUIRE_FFMPEG: "1"
      - name: Fase 6 — relatório de desempenho
        if: always()
        run: cat target/phase6-acceptance/performance.json || true
      # artefatos: target/perf e target/phase6-acceptance (já cobertos por `path: target/perf`; incluir a segunda pasta)
```
Nightly/manual: `node tools/phase6-acceptance/performance/soak.mjs --seconds 28800` (8 h; não rodar no CI principal).

## 4. Soak, vazamento e falha de disco (`crates/capia-project/tests/soak.rs`)

Rápidos (não ignorados, Linux; fora dele **pulam com motivo impresso**, nunca passam em vazio):
* abrir/fechar ×40 (`Project`) e ×25 (`Session`: `project.open`+`snapshot`+`close`): descritores **iguais**, threads
  ≤ +1, RSS ≤ +24 MiB (medido: fd 3→3, RSS ±0);
* commit+undo ×150: revisões = +300, conteúdo do documento idêntico, fd iguais, RSS limitado, arquivo reabre e valida;
* disco: diretório inexistente e diretório no lugar do arquivo ⇒ erro estruturado, nada criado/alterado; somente-leitura
  (diretório e arquivo) ⇒ erro estruturado e projeto intacto (**pula como root**, que ignora permissões — roda no CI);
* **disco cheio simulado**: processo filho sob `ulimit -f 128` (RLIMIT_FSIZE, SIGXFSZ ignorado e herdado no `exec`) ⇒
  `EFBIG`; o commit falha com `PersistenceFailed` ("could not persist the change…") sem texto bruto do SQLite, e o
  projeto reabre válido com todos os commits confirmados duráveis e nenhum parcial. O teste falha se o limite nunca
  for atingido (não é vazio).
* `soak_long` (`#[ignore]`, `CAPIA_SOAK_SECS`, `CAPIA_SOAK_SPEC=default|medium`): ciclos abrir/commit/undo/API/fechar
  com amostras de RSS/fd/threads em `target/perf/phase6-soak.json`; asserta RSS limitado, fd constantes, documento
  inalterado. Rodado 8 s aqui (32 ciclos) só como fumaça; **o soak de 8 h não foi executado**.

**Não viável nesta trilha:** ENOSPC real (precisa de tmpfs/loop montado = root), falha de `fsync`/EIO, remoção do
disco no meio do commit, e soak de servidor/webhook/update (dependem de outras trilhas). `RLIMIT_FSIZE` cobre o
mesmo caminho de erro de escrita (`EFBIG`) mas não o de `ENOSPC` do kernel.

## 5. Compatibilidade de migração (`crates/capia-store/tests/migration_compat.rs`)

Estende `schema.rs`/`crash.rs` (que usam projetos sem linhas nas tabelas novas / matam o processo): para **cada**
schema 1..5 cria um projeto real (`create_at_schema`, histórico com ramo de redo, operation_ids) **com linhas em todas
as tabelas existentes naquela versão** (catálogo, eventos, jobs, tickets, `ai_records`, `ai_usage`, runs, stages,
eventos, efeitos, proveniência, memória, orçamento) e afirma, após abrir no schema atual:
documento/histórico/`operation_id`s idênticos; **todas as linhas de todas as tabelas iguais** (nas colunas que já
existiam); tabelas novas vazias; trilha `schema_migrations` contígua e preservada; backup `.vN.bak` existe só quando
houve migração, é íntegro e abre no schema antigo; o software "antigo" recusa o arquivo migrado
(`UnsupportedSchemaVersion`) sem tocá-lo (hash do arquivo igual); reabrir é idempotente (nenhum arquivo novo);
`validate_file`/`inspect` não destroem; o projeto migrado aceita commit/undo e o `AutonomyStore`.
Falhas: degrau k = 2..5 quebrado (DDL parcial + erro) ⇒ arquivo na versão k−1, DDL parcial revertido, dados
intactos, backup do original íntegro; recuperação por **nova tentativa** e por **restauração do backup** em arquivo novo.

**Bug real encontrado e corrigido** (`crates/capia-store/src/catalog.rs`): `validate_file`/`inspect` em projeto no
schema 2 falhavam (`no such column: fingerprint`) porque a leitura do catálogo pedia a coluna da migration 3. `read_all`
agora detecta a coluna ausente (leitura sem migrar). Coberto pela matriz acima.

### Estratégia de rollback — achados
* Migrações são **só para frente**, uma transação `IMMEDIATE` por degrau; falha ⇒ arquivo fica na última versão
  concluída (provado para todos os degraus).
* O rollback é o **backup `.vN.bak`** (`VACUUM INTO`, consistente, criado antes do primeiro degrau, só se há migração).
  Restaurar = copiar o backup por cima (ou ao lado) com o software antigo/novo fechado; provado que abre e migra de novo.
* Lacunas documentadas (não alteradas por não serem regressão): (a) o backup não é re-verificado após o `VACUUM INTO`
  (`integrity_check`); (b) não há checagem de espaço livre antes do backup — disco cheio ⇒ `MIGRATION_FAILED` com o
  arquivo intocado (o backup falha antes de qualquer degrau); (c) backups nunca são removidos (política de retenção é
  decisão de produto/instalador); (d) o nome com sufixo `-<ms>` evita sobrescrever um backup antigo, mas acumula arquivos.
  Falha ao criar o backup por permissão não foi testada (root); o caminho de erro é `MigrationFailed` por construção.
* Downgrade (abrir schema novo com build antigo) é recusado sem tocar o arquivo — provado; não há migração reversa.

## 6. Resíduos honestos
* `commit_with_snapshot` (≈ 100 ms p95 com `snapshot_every = 1`, snapshot de 6.000 clips a cada commit) é o pior caso
  sintético: com a cadência padrão (256) o snapshot cai 1 em 256 commits e o p95 comum fica em ≈ 5 ms. Não otimizado.
  Se a cadência mudar, o gate cobre (limite 350 ms ×1,5).
* Números de VM compartilhada, cache de SO quente, sem GPU; a validação em hardware de referência continua sendo a do
  pacote de aceitação da Fase 3.
