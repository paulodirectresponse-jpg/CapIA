# capia-decode

Serviço de decode persistente (ADR-059..062). Depende só de `capia-time` e `capia-media`; fora do WASM.

- **`DecodeService`:** pool limitado de sessões de ffmpeg (N workers = N sessões), reaproveitadas por `(projeto, conteúdo, stream)`; fila `Interactive > Playback > Background`; cancelamento; **supersession por `Lane`** (scrub rápido entrega só o último); expulsão de sessão ociosa e por pressão; métricas (`DecodeMetrics`, `reuse_rate`).
- **Cache de quadros:** `ByteLru` com **orçamento em bytes**; chave `(namespace, conteúdo, stream, PTS, formato, versão do backend)`; `invalidate_content/namespace`.
- **Prefetch:** `prefetch(src, from, Forward|Backward|Jump, lane)` em *background*.
- **Áudio:** `PcmCache` (blocos de 16.384 amostras, `ByteLru`, read-ahead de 7 blocos) sobre `decode_audio_indexed`.
- A fonte é sempre o original; o proxy não existe aqui. Testes: `tests/service.rs` (13), `tests/audio.rs` (5), unidade do `ByteLru`; `tests/perf.rs` (`--ignored`).
