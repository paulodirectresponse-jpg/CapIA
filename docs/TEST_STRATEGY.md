# TEST STRATEGY

## 1. Pirâmide e prioridades

O núcleo (tempo, modelo, comandos, persistência, render) é onde bugs custam projetos corrompidos e confiança perdida — recebe a maior densidade de testes. UI e IA recebem testes de contrato e E2E focados.

| Camada | Tipo de teste | Ferramentas (candidatas) |
|---|---|---|
| `capia-time` | Unitários + **propriedade** (conversões, arredondamento, overflow, limites JS) | `cargo test`, `proptest` |
| `capia-model` / `capia-commands` | Propriedade (sequências aleatórias de comandos), unitários por comando, golden JSON | `proptest`, `insta` |
| `capia-store` | Round-trip, migrations com fixtures, crash/kill, corrupção, backup/restore | testes de integração, processos filhos |
| `capia-media` | Corpus de conformidade (VFR, fps, rotação, áudio), frame-exactness | corpus em `/testdata` |
| `capia-render` | Golden frames, paridade preview×export, sync A/V | comparação bit-a-bit e perceptual (SSIM/PSNR) |
| `capia-jobs` | Cancelamento, retry, persistência/restart, dependências | integração |
| `capia-ai` | Contrato de providers (mock HTTP), tools (schema/permissão), pipeline com **Replay provider**, evals | `wiremock`, fixtures gravadas |
| Segurança | Chave canário, fuzzing, IPC sem segredos | `cargo-fuzz`, scanners |
| UI | Componentes (ui-timeline: hit-test, snapping, virtualização), E2E | Vitest, Playwright/WebDriver (`tauri-driver`) |
| Desempenho | Benchmarks com projetos sintéticos | `criterion`, traces |

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
