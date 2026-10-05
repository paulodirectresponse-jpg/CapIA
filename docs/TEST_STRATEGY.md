# TEST STRATEGY

## 1. Pirâmide e prioridades

O núcleo (tempo, modelo, comandos, persistência, render) é onde bugs custam projetos corrompidos e confiança perdida — recebe a maior densidade de testes. UI e IA recebem testes de contrato e E2E focados.

| Camada | Tipo de teste | Ferramentas (candidatas) |
|---|---|---|
| `capia-time` | Unitários + **propriedade** (conversões, arredondamento, overflow, limites JS) | `cargo test` (gerador xorshift com semente; sem `proptest`, ADR-041) |
| `capia-model` / `capia-commands` | Propriedade (sequências aleatórias de comandos, ≥ 10.000 por execução), unitários por comando, golden por digest, paridade nativo × WASM | `cargo test`; `tests/properties.rs`, `tests/golden.rs`, `tools/check-wasm-parity.mjs` |
| `capia-store` | Round-trip, migrations com fixtures, crash/kill, corrupção, backup/restore | testes de integração, processos filhos |
| `capia-media` | Corpus de conformidade (VFR, fps, rotação, áudio), frame-exactness | corpus em `/testdata` |
| `capia-render` | Golden frames, paridade preview×export, sync A/V | comparação bit-a-bit e perceptual (SSIM/PSNR) |
| `capia-jobs` | Cancelamento, retry, persistência/restart, dependências | integração |
| `capia-ai` | Contrato de providers (mock HTTP), tools (schema/permissão), pipeline com **Replay provider**, evals | `wiremock`, fixtures gravadas |
| Segurança | Chave canário, fuzzing, IPC sem segredos | `cargo-fuzz`, scanners |
| UI | Componentes (ui-timeline: hit-test, snapping, virtualização), E2E | Vitest, Playwright/WebDriver (`tauri-driver`) |
| Desempenho | Benchmarks com projetos sintéticos | `criterion`, traces |

## 1.1 Ativos de teste normativos

- **Suíte de aceitação de comportamento da timeline** — `tests/acceptance/timeline/*.json` (120 cenários: 108 da M03 + 12 da M05, ADR-036/039): critério de aceitação da Fase 2 para `capia-commands`; o harness converte frames→Ticks e exige exatidão e atomicidade nos erros.
- **Paridade nativo × WASM** (S4/ADR-016): hash de um workload determinístico sobre o modelo real, idêntico no nativo e no WASM, em CI.
- **Conformidade de mídia** (S2/ADR-035): clipes sintéticos CFR/VFR com índice de frame gravado na imagem (`spikes/s2-frame-exact/generate.sh` como ponto de partida); seek por índice 100% exato; conformação de cadência = `frame_at`; sync A/V ≤ 1 amostra.
- **Golden frames sem GPU:** wgpu em llvmpipe/WARP no CI (S6 mostrou que roda).

## 2. Invariantes testadas por propriedade (Command Engine)

Gerador produz projetos e sequências aleatórias de comandos (válidos e inválidos):
1. Após qualquer commit, **todas** as invariantes de `TIMELINE_ENGINE.md` §6 valem.
2. `undo(apply(doc, tx)) == doc` (igualdade estrutural) e `redo(undo(x)) == x`.
3. Transação com qualquer comando inválido ⇒ documento inalterado (atomicidade).
4. `serialize → deserialize` = identidade (SQLite e JSON canônico).
5. Determinismo: mesmo snapshot + mesmo comando + mesmo gerador de IDs ⇒ mesmas ops.
6. WASM e nativo produzem as mesmas ops para os mesmos comandos (teste cruzado).
7. Ciclos de nested sempre rejeitados; profundidade respeitada.
8. Rebase sem conflito ⇒ mesmo resultado que aplicar em sequência.
9. **Idempotência (ADR-029):** reenviar uma transação com os mesmos `operation_id` (inclusive após kill no meio do commit) nunca duplica efeitos; mesmo id com payload diferente é rejeitado.
10. **Plan token (ADR-030):** `apply_plan` só aplica o plano revisado: token adulterado/expirado/consumido/de outro ator é rejeitado; mudança de revisão com `diff_digest` diferente ⇒ `PLAN_STATE_CHANGED`; atores `Agent/Api` sem preview ⇒ `PREVIEW_REQUIRED`.

## 3. Tempo e mídia

- Tabela de taxas (`TIMELINE_ENGINE.md` §1.2) verificada exatamente; ida e volta frame↔ticks↔amostra sem perda.
- Limite de 24 h e inteiros seguros em JS verificados.
- **Corpus de mídia** (gerado sinteticamente com FFmpeg quando possível + amostras reais licenciadas): CFR 23,976/24/25/29,97/30/50/59,94/60; VFR de iPhone e Android; rotação 90/180/270; áudio 44,1/48 kHz, AAC com priming, edit lists; HDR HLG/PQ; arquivos truncados/corrompidos; codecs H.264, HEVC, VP9, ProRes, MJPEG; imagens PNG/JPEG/WebP/HEIC.
- **Clapperboard sintético:** vídeo com contador de frames impresso + bip de áudio em frames conhecidos → detecta automaticamente erro de frame e drift A/V após cortes, speed, nested e export.
- Frame-exactness: para N timestamps aleatórios, frame decodificado via seek == frame obtido por decode sequencial.

## 4. Render

- **Golden frames:** projetos de teste renderizam frames específicos comparados com referências versionadas (tolerância zero para o caminho de export sem encode; SSIM ≥ 0,99 após encode).
- **Paridade preview×export:** mesmo snapshot, mesmos tempos → bit-idêntico (resolução total, sem proxy); com proxy, SSIM ≥ 0,95 e timing idêntico.
- Testes por GPU: CI com adaptador de software (WARP no Windows) + execução periódica em máquinas com NVIDIA/Intel/AMD.
- Texto: golden por fonte/estilo; fallback de fonte faltante.

## 5. Persistência e recuperação

- Fixtures `.capia` de cada `schema_version` → migram e passam validação.
- **Crash tests:** processo filho executa transações/jobs e é morto em pontos aleatórios (inclusive durante checkpoint WAL); o pai reabre e verifica integridade + "última transação confirmada presente, não confirmada ausente".
- Corrupção sintética (bytes aleatórios) → fluxo de recuperação via backup/snapshot funciona.
- Disco cheio / sem permissão → erro claro, sem corromper.

## 6. IA

- **Replay provider:** grava requisições/respostas reais uma vez; CI reproduz deterministicamente pipelines inteiros sem rede nem custo.
- **Contrato de adapters:** cada adapter testado contra servidor mock com respostas reais anonimizadas (tool calls, streaming, erros 429/5xx, `retry_after`).
- **Tools:** cada tool testada para schema inválido, permissão negada, stage errado, orçamento estourado.
- **Gate de escrita:** teste garante que `timeline.*` de escrita é inacessível fora de EDIT/CORRECT ou sem plano validado.
- **Prompt injection:** corpus de briefs/transcripts maliciosos → nenhuma tool fora da permissão é executada (as permissões são o teste; o modelo pode "querer").
- **Evals (Fase 4–5):** conjuntos de demandas com rubricas (aderência ao brief, timing, legendas, pacing vs. referência), rodados manualmente/periodicamente com modelos reais; resultados versionados.
- **Memória:** nenhuma promoção automática de escopo.

## 7. Segurança

- Chave canário configurada em testes de integração: grep em logs, `.capia`, `app.db`, eventos IPC capturados, dumps → zero ocorrências.
- Verificação de vínculo de host (redirect para outro domínio não leva o header).
- Fuzzing: loader JSON canônico, deserialização de comandos, parsers de documentos, protocolo `capia://` (path traversal).
- Secret scanning no CI.

## 8. Desempenho (benchmarks com orçamento)

**Máquina de referência (ADR-033):** Windows 11 x64; mínima = 4+ cores, 16 GB, GPU DX12/wgpu (iGPU moderna ok), SSD, edição 1080p; recomendada = 8+ cores, 32 GB, dGPU 6 GB+ VRAM, NVMe. As metas abaixo valem na **mínima**, salvo nota. O CI em nuvem (WARP/llvmpipe) valida **correção**, não desempenho.

| Cenário | Meta |
|---|---|
| Comando simples em projeto de 10.000 clips | < 1 ms (core) |
| Transação de 500 ops + validação + commit | < 200 ms |
| Abrir projeto 10.000 clips / 50 sequences | < 2 s |
| Undo/redo | < 50 ms |
| Timeline UI: scroll/zoom com 5.000 clips | 60 fps |
| Seek no preview com proxy | < 100 ms |

Regressões > 10% falham o CI de benchmark (rodado em máquina dedicada, não em cada PR).

## 9. CI

- PR: build Windows (+ Linux para crates puros), clippy, fmt, testes unitários/propriedade (orçamento de casos reduzido), testes de integração rápidos, secret scan, verificação de regras de dependência entre crates.
- Noturno: propriedade com orçamento alto, corpus de mídia completo, crash tests, fuzzing curto, benchmarks, golden de GPU real.
- Mídia de teste: gerada por script ou baixada de fonte licenciada; nunca mídia de clientes no repositório.

## Persistência (M06)

- **Crash real:** `crates/capia-store/tests/crash.rs` re-executa o binário de teste como filho e o mata (`abort` ou `kill` externo) em `before_begin`, `in_tx_after_entry`, `in_tx_after_writes`, `before_commit`, `after_commit`, além de SIGKILL aleatório em loop. Invariante: estado A completo **ou** B completo, `integrity_check`, log de operações coerente, reenvio idempotente.
- **Propriedade com salvar/reabrir:** `properties.rs` roda engine persistido × engine em memória em *lockstep*, reabrindo aleatoriamente, e compara `export_state()`. Orçamento: 1.000 sequências por gerador por padrão (cada commit faz fsync); CI release com `CAPIA_PROP_CASES=5000`.
- **Corrupção/migrations:** `schema.rs` (12 variantes + byte-fuzz de 300 rodadas; migration v2 sintética).
- **Mutação manual** registrada em `docs/STATUS.md` (11 mutações, todas detectadas).

## Assets, mídia e composição (M07)

- **Fixtures reais mínimas e versionadas** (`tests/fixtures/media`, ≈ 300 KiB desde a M08; geradas de forma determinística por `tools/gen-media-fixtures.sh` com fontes sintéticas): vídeo+áudio, vídeo sem áudio, WAV, PNG com alfa, JPEG, arquivo inválido. Versionar (em vez de gerar no teste) tira a dependência do *encoder* da máquina; o FFmpeg só é necessário para **ler** (ffprobe). Goldens de metadados normalizados em `crates/capia-media/tests/golden` comparam só campos estáveis entre versões (sem `bit_rate`).
- **FFmpeg no CI é explícito** (apt / Chocolatey) e a versão vai para o log; `CAPIA_REQUIRE_FFMPEG=1` transforma a ausência em FALHA. Lógica de import/dedup/offline/relink roda com um `MediaProbe` falso (`capia_assets::testing`), independente do FFmpeg.
- **Processos:** timeout, flood de stdout, backend ausente, JSON malformado e injeção de argumentos (Unix e Windows) em `capia-media`.
- **Propriedade com assets** (`capia-project/tests/properties_assets.rs`): modelo de referência independente (arquivos × conteúdos; pares de mesmo tamanho distinguíveis só por hash) prevê cada resultado de import/relink/verify; mistura clips, `delete_asset`, comandos de composição, undo/redo e **reabertura** (documento e catálogo idênticos). Orçamento: 200 casos × 40 passos por padrão; `CAPIA_IO_PROP_CASES` (5.000 no Linux/release no CI; 100 no Windows).
- **Composição:** `capia-commands/tests/compose.rs` (24) + os cinco comandos no gerador de `properties_nested`/`capia-store/tests/properties` (undo/redo exatos, DAG válido, reabertura).
- **Atomicidade do import:** crash tests reais (`capia-store/tests/crash.rs`) com o catálogo na transação do commit; escritor obsoleto não deixa linha de catálogo.
- **Mutação manual:** 14 mutações + a combinada de ciclo (STATUS).

## Jobs e pipeline de mídia (M08)

- **Executor** (`capia-jobs/tests/executor.rs`, 13): justiça (créditos 6/3/1: o background avança sob enxurrada de interativos), cancelamento de job na fila e em execução, dedup, fila cheia, *panic* contido, `shutdown ⇒ interrupted`, progresso monotônico.
- **Processos reais:** cancelar/timeout/`Flow::Stop` **matam o filho** (o PID some — Unix `/proc`, Windows `tasklist`), proxy cancelado não deixa arquivo (`capia-media/tests/pipeline.rs`).
- **Quadros identificáveis:** fixtures cujo luma é função do número do quadro (`cfr_gop.mp4` GOP longo + B-frames, `vfr.mp4` com buracos de timestamp, mkv `ffv1` com offset de 3 s) provam por **pixels** que o quadro entregue é o do índice — não `N/fps`. Áudio: intervalos exatos em amostras a 44.100 e 48.000 Hz, início ≠ 0, clipes curtos, além do fim, stream de áudio não-padrão.
- **Formatos binários** `CIDX`/`CWFM`: ida e volta exata; **toda truncagem e todo byte alterado** é rejeitado sem pânico.
- **Cache v2:** produção atômica, parcial nunca publicado, corrompido vira *miss*, 12 produtores da mesma chave geram **uma** vez, chaves diferentes não se bloqueiam, GC/invalidar/limpar.
- **Hash:** cancelável entre blocos, detecta crescimento, regravação e troca do arquivo; **colisão deliberada** da impressão rápida separada pelo SHA-256 (32 MiB, difere só fora das regiões amostradas).
- **Pipeline com FFmpeg real** (`capia-project/tests/pipeline.rs`, `force_relink.rs`): import assíncrono (ticket → pending → finalizado; identidade = SHA-256), latência de retorno vs hash completo, cancelamento, arquivo mudando durante o hash, índice/waveform/proxy/decode via cache, dedup de jobs, fonte trocada recusada, **o proxy nunca é fonte de decode**, relink em lote por conteúdo (nunca por nome; ambíguo/rejeitado/isca de mesmo tamanho), force relink (conflitos estruturados, sem trim).
- **Crash real** (`capia-project/tests/crash_media.rs`): o processo filho é morto (SIGKILL/TerminateProcess) **dentro** do índice, do waveform, da codificação do proxy e com um import pendente. Pai: projeto válido, job `interrupted` (nunca `completed`), nenhum derivado parcial publicado, retry funciona. *(Achou um bug real: lock de cache por arquivo `create_new` deixava a chave presa — agora lock do SO.)*
- **Propriedade** (`properties_media.rs`): sequências aleatórias importar/derivar/decodificar/cancelar/offline/relink em lote/reabrir contra um modelo independente; `CAPIA_MEDIA_PROP_CASES` (6 por padrão; 40 no Linux/release; 2 no Windows).
- **Mutação manual** (`tools/mutation-m08.py`): 13 mutações com o teste que as mata (ver STATUS).
- **Medições** (`#[ignore]`, release): `capia-jobs/tests/perf.rs`, `capia-assets/tests/perf_hash.rs`, `capia-project/tests/perf_media.rs`.

## Fase 2 — conclusão (render, decode, áudio, preview, export)

| Área | Testes |
|---|---|
| Compositor/mixer | `capia-render/tests/golden_video.rs` (10 goldens: tela cheia, 2 sobrepostos, 50% de opacidade, scale+position, imagem transparente sobre vídeo, nested, nested multinível, VFR, retime, z-order de 2 tracks) e `golden_audio.rs` (8, tolerância documentada); digests em `tests/golden/*.json` (bless com `BLESS=1`) |
| Decode | `capia-decode/tests/service.rs` (13: LRU por bytes, hit, conteúdo trocado, multi-stream, concorrência, sem contaminação entre projetos, prefetch, prioridade, supersession, cancelamento, ociosidade, shutdown) |
| Áudio | `capia-media/tests/audio_seek.rs` (PCM bit-exato 44,1/48 kHz, AAC por posição exata, offset não zero, custo independente do início), `capia-decode/tests/audio.rs` (fronteiras de bloco, curto, além do fim, blocos adjacentes, orçamento, invalidação) |
| Render do projeto | `capia-project/tests/render_project.rs`, `parity.rs` (preview × export, reabrir, frio/quente, proxy, ordem), `properties_render.rs` (timelines aleatórias × **oráculo** de decode direto; `CAPIA_RENDER_PROP_CASES`) |
| A/V | `capia-project/tests/av_drift.rs`: flash × beep **medidos no MP4 exportado** (23,976/29,97/59,94/25/30 × 44,1/48 kHz), VFR por tempo, timeline sintética de 10 min |
| Export | `capia-project/tests/export.rs` (atômico, cancelamento, staging órfão, encoders, GPL recusado, MP4 de referência), `capia-media` `encoder::tests` |
| Preview | `capia-preview/tests/scheduler.rs` (seek, scrub t1→t4, cadência, quadro atrasado, fim, seek durante reprodução) |
| Crash real | `capia-project/tests/crash_phase2.rs`: kill com sessão de decode ativa, no render de range, no índice de áudio, no export intermediário (2 pontos) e no MP4 (2 pontos) |
| Aceitação | `capia-cli/tests/phase2_e2e.rs`: `HOOK_A`, `HOOK_B`, `BODY_MASTER` (nested) pelo binário real |
| Mutação | `tools/mutation-phase2.py` — 16 mutações, 16/16 detectadas |
| Desempenho | `capia-render/tests/perf.rs`, `capia-decode/tests/perf.rs`, `capia-project/tests/perf_render.rs` (`--ignored`; só medem) |


## Editor (Fase 3) — ADR-076

- **E2E (Playwright, `packages/e2e`):** UI compilada × **engine e FFmpeg reais**. Devserver no Linux; **app Tauri real no Windows** por CDP/WebView2 (`CAPIA_E2E_TARGET=tauri`). Suites: `flows` (17 fluxos críticos com conferência da verdade persistida via `sequence.get`), `editing` (seleção, marquee, grupo, copiar/colar, ripple, faixas, keyframes, legendas, idioma, atalhos, playback), `crash` (kill -9 após edições, kill no meio do export sem arquivo parcial, preferências corrompidas), `visual` (capturas + sondas de pixel; sem goldens frágeis), `perf` (metas §6 com 5.000 clips; *smoke* no CI, estrito com `CAPIA_PERF_STRICT=1`).
- **Unitários só com lógica:** kit de componentes, comandos de edição puros, agendador do preview, áudio, teclas, preferências, ponte WASM, transporte; **fronteira UI→engine** por varredura de fonte (`architecture.test.ts`) e controlador com cliente falso.
- **Rust:** `capia-commands/tests/editor.rs`, `capia-render/tests/editor_render.rs` (inclui `design_size`), cache de grafo (`render_project.rs`), `capia-editor-api/tests/session.rs` (inclui `Job` fora do lock, `revision_changed`, `render.audio`), `capia-webview-surface` (região de frame), `capia-desktop` (IPC fixo).
- **Humano/hardware (não automatizável):** `tools/phase3-acceptance/`.

## Fase 4 — testes de IA

Sem credenciais externas no CI: Replay (digest/roteiro/respondedor) + servidores HTTP falsos que falam o protocolo de cada fabricante (suíte de contrato idêntica para OpenAI-compatível, Anthropic, Google). Segurança: `capia-ai/tests/security.rs`, `capia-intelligence/tests/{service,assistant,demand_flow}.rs`, UI (`architecture.test.ts`, `aiController.test.ts`, `AiPanel.test.tsx`), E2E (`packages/e2e/tests/ai.spec.ts`), tudo agregado por `tools/phase4-acceptance/security/run.mjs`. Qualidade: corpus de cenas anotado por construção + *held-out*; 10 briefings em modo Replay. Qualidade de LLM real, corpus real e smoke com chaves reais são **externos** e opcionais.
