# capia-intelligence

Pipelines e assistente da Fase 4 (ADR-082..086). Cliente do Engine API: lê por métodos de leitura (lista fechada) e escreve **só** por `preview → apply_plan` (ator `Agent`).

| Módulo | Entrega |
|---|---|
| `transcript` | STT por trechos (FLAC mono 16 kHz), costura em µs inteiros, cache por conteúdo+modelo |
| `captions` | legendas = clips de texto comuns em track `captions`; plano determinístico |
| `silence` | silêncio local (RMS 20 ms) → cortes alinhados ao frame **para dentro** do silêncio |
| `scenes` | detecção de cenas local; métrica `(FP+FN)/N ≤ 5 %`; corpus em `tests/scene_corpus.rs` |
| `reference` | `ReferenceGrammar` determinística (ritmo, transições, áudio, fala, hook/corpo/CTA) |
| `docs`, `demand` | extração segura DOCX/PDF/TXT/MD; `DemandSpec` com fontes **verificadas**; sem tools |
| `assistant`, `tools_exec` | chat pontual com tools, gate, aprovação (Ask/Auto), cancelamento, idempotência |
| `service` | superfície `ai.*` (config write-only, tarefas, eventos), hospedada por devserver/desktop |

Testes: `cargo test -p capia-intelligence` (FFmpeg real; `CAPIA_REQUIRE_FFMPEG=1` no CI). Aceitação: `tools/phase4-acceptance/`.
