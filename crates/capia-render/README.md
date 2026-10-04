# capia-render

Render **headless e determinístico** (ADR-063..065). Crate **puro**: depende só de `capia-time` e `capia-model` (`capia-commands` como dev-dependency dos testes); sem IO, sem FFmpeg; compila para WASM.

- **Grafo:** `RenderGraph::compile(doc, root)` (sequences alcançáveis por nested, `BTreeMap`), `plan_video(seq, t)` (layers de baixo para cima; `content_time` com start/source_in/speed `Rational`/reversed; propriedades no tempo de conteúdo).
- **Compositor CPU de referência:** RGBA8, alfa reto, base preto opaco, *source-over* inteiro half-up, opacidade, `scale`/`position`, rotação em múltiplos de 90°, bilinear em ponto fixo (cobertura pelo centro do pixel), nested por canvas filho. APIs: `render_frame`, `render_video_range`, `frame_digest` (SHA-256 próprio).
- **Mixer:** f32, ganho por clip (`volume_db`, blocos de 256 amostras), mute/solo, retime por varispeed, reamostragem sinc, *hard clip* em ±1 (`mix_audio_range`).
- **Fonte de mídia:** `trait MediaSource` (quadro por tempo de fonte, imagem, PCM na taxa de saída, `cancel_pending`); implementada por `capia-project::ProjectSource`.
- **Fora do escopo:** texto (`TEXT_NOT_RENDERED`), transições, efeitos, máscaras, rotação arbitrária (`ROTATION_NOT_MULTIPLE_OF_90`).
- **Testes:** `tests/golden_video.rs` (10 goldens), `tests/golden_audio.rs` (8), unidade; `BLESS=1` regrava os digests; `tests/perf.rs` (`--ignored`) mede 1080p.
