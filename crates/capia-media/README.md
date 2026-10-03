# capia-media

Mídia externa como **entrada hostil** (ADR-047). Probe estruturado via `ffprobe`, normalização determinística e execução limitada de processos. Faz IO (processos) e **não** compila para WASM; só conhece `capia-time`.

- **Contrato:** `trait MediaProbe { fn probe(&self, path) -> Result<MediaInfo, MediaError> }`. O resto do CapIA nunca vê JSON/texto do ffprobe.
- **Backend:** `FfprobeBackend` (`-print_format json -show_format -show_streams`, protocolo restrito a `file`, caminho como UM argumento `file:<abs>` depois de `-i`, sem shell).
- **Localização:** `MediaToolchain::locate(&MediaConfig)` — caminho configurado → `CAPIA_FFPROBE`/`CAPIA_FFMPEG` → diretório *bundled* (`<exe>/ffmpeg`) → `PATH`. Nada achado ⇒ `MEDIA_BACKEND_NOT_FOUND` (sem pânico). Um caminho explícito inexistente é erro (não cai para o `PATH`).
- **Limites:** stdout 8 MiB, stderr 64 KiB, timeout 30 s (processo morto), `-probesize`/`-analyzeduration`; demuxers de playlist/rede (`hls`, `concat`, `dash`, `sdp`, `rtsp`…) rejeitados.
- **Normalização:** tempo em `Ticks` por aritmética inteira (`parse_decimal_ticks`), frame rates em `Rational`, streams ordenados, stream padrão explícito (primeiro vídeo que não é capa; primeiro áudio), limites de plausibilidade (1.000 h, 65.536 px, 1.000 fps, 64 canais, 768 kHz), rotação 0/90/180/270, alpha só quando o `pix_fmt` declara.
- **Miniatura:** `extract_frame_png` (um quadro PNG, escala limitada) — o cache fica em `capia-assets`.
- **Testes:** unidade (normalização hostil, processos), `tests/real.rs` (ffprobe real sobre `tests/fixtures/media`, *goldens* em `tests/golden/`, backend falso com travamento/flood). `CAPIA_REQUIRE_FFMPEG=1` faz a ausência do binário FALHAR; `CAPIA_BLESS=1` regrava os goldens.

## M08 — pipeline de mídia derivada

| Peça | API | ADR |
|---|---|---|
| Runner em streaming (cancelável, mata o filho, teto de stderr, `pid_sink`) | `run_streaming`, `run_collect`, `Flow` | 052 |
| Índice de quadros `CIDX` (PTS/DTS/duração/keyframe reais; CFR/VFR/B-frames/GOP longo) | `build_frame_index`, `FrameIndex` (`frame_at_or_before/after`, `nearest_frame`, `keyframe_before`, `frame_by_index`, `encode/decode`) | 054 |
| Decode exato de quadro (RGBA8, limites checados) | `decode_frame_at`, `decode_frame_by_index`, `RawFrame` | 054 |
| Áudio PCM f32 intercalado, intervalo exato em amostras | `decode_audio`, `decode_audio_blocks`, `samples_to_ticks`, `ticks_to_samples` | 055 |
| Waveform multirresolução `CWFM` (min/max/rms, checksum) | `generate_waveform`, `Waveform`, `WaveformBuilder` | 055 |
| Proxy (`ProxyProfileV1`; MJPEG padrão; H.264 só por encoder de hardware/SO) | `generate_proxy`, `select_encoder`, `list_encoders` | 056 |

Feature `failpoints` (**só testes**): `CAPIA_FAILPOINT=index_running|waveform_running|proxy_running` + `CAPIA_FAILPOINT_MODE=park|abort` para os crash tests reais.
Fixtures com quadros identificáveis (luma = função do número do quadro): `cfr_gop.mp4`, `vfr.mp4`; áudio 44,1 kHz: `tone_44k.wav` (`tools/gen-media-fixtures.sh`).
Testes: `tests/pipeline.rs` (índice, decode exato, áudio, waveform, proxy, cancelamento com PID que some).
