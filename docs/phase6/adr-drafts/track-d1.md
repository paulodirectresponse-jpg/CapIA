# ADR (rascunho) — Fase 6 / Track D-1: fixture de projeto grande e política do gate de desempenho

Status: **proposta** (o integrador numera e move para `docs/DECISIONS.md`).

## Contexto
A Fase 6 exige medir o engine num projeto grande (≥ 30 sequences, ≥ 5.000 clips, nested, legendas, áudio,
catálogo e histórico de Runs) preservando as metas da Fase 3, com um gate de regressão que não seja nem
frouxo (aprovar em vazio) nem frágil (reprovar por ruído de runner).

## Decisões
1. **Fixture = crate dev-only `capia-fixtures`.** Constrói um `.capia` real **somente** por Command Engine,
   `Catalog`/`AutonomyStore` públicos; é determinística por `seed` (`document_digest` estável). Entra na matriz
   de arquitetura como crate de apoio: dependência **apenas** via `[dev-dependencies]`, ignorada no grafo de
   ciclos (o ciclo project ⇄ fixtures é de teste). Sem SQL cru: se o produto não consegue gravar, a fixture não grava.
2. **Catálogo grande sem arquivos.** Registros sintéticos com disponibilidade `offline` (o catálogo aceita
   caminhos inexistentes); evita gerar milhares de arquivos e custa o mesmo para abrir/listar/`assets.list`.
3. **Relatório com amostras brutas.** O benchmark grava p50/p95/máx **e** as amostras, mais a máquina
   (CPU, núcleos, RAM, kernel, rustc, perfil). Números sem máquina não são comparáveis.
4. **Gate = mediana de N=3 execuções** por estatística, comparada a `thresholds.json`. Metas documentadas
   (`hard`: 30/50/100/16 ms, 2 s) **nunca** recebem tolerância; limites de regressão = ≈ 3× o p95 medido × fator
   1,5 (substituível por `CAPIA_PERF_TOLERANCE`). Métrica ausente/NaN/sem limite/pulada sem motivo permitido
   **reprova**. Afrouxar um limite `hard` exige novo ADR (há teste que trava os valores).
5. **Asserções no benchmark só de sanidade**; metas finas ficam no gate (um p95 isolado é ruído).
6. **Falha de disco**: "disco cheio" simulado por `RLIMIT_FSIZE` (`ulimit -f` + SIGXFSZ ignorado) num processo
   filho — exercita o caminho de erro de escrita (`EFBIG`) sem root. ENOSPC real e falha de `fsync` ficam fora
   (exigem mount/root) e são declarados como não cobertos.
7. **Soak**: testes rápidos de vazamento (fd/threads/RSS via `/proc/self`; pulam com motivo fora do Linux);
   soak longo (`soak.mjs --seconds`) manual/noturno, nunca no CI principal.
8. **Migração**: matriz v1..atual com dados em todas as tabelas; rollback = backup `.vN.bak` pré-migração;
   migração só para frente; schema novo é recusado por build antigo sem tocar o arquivo. Novo schema ⇒ estender a
   matriz (teste falha de propósito se `MIGRATIONS.len()` mudar sem atualizar).

## Alternativas rejeitadas
* Limites fixos de ms para tudo sem mediana: instável em runner compartilhado.
* Mediana + tolerância também nas metas documentadas: deixaria uma regressão material passar.
* Gerar mídia real para o catálogo: lento, sem ganho de cobertura para abrir/listar.
* Migração reversa: complexidade sem demanda; backup atômico basta.

## Consequências
Corrigido junto: `catalog::read_all` falhava em schema 2 (`validate_file`/`inspect` antes de migrar). Resíduos:
commit com snapshot a cada commit ≈ 100 ms (pior caso sintético); retenção/verificação de backups e checagem de
espaço livre pré-migração não existem (candidatos a trabalho futuro).
