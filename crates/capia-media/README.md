# capia-media

Mídia externa como **entrada hostil** (ADR-047). Probe estruturado via `ffprobe`, normalização determinística e execução limitada de processos. Faz IO (processos) e **não** compila para WASM; só conhece `capia-time`.

- **Contrato:** `trait MediaProbe { fn probe(&self, path) -> Result<MediaInfo, MediaError> }`. O resto do CapIA nunca vê JSON/texto do ffprobe.
- **Backend:** `FfprobeBackend` (`-print_format json -show_format -show_streams`, protocolo restrito a `file`, caminho como UM argumento `file:<abs>` depois de `-i`, sem shell).
- **Localização:** `MediaToolchain::locate(&MediaConfig)` — caminho configurado → `CAPIA_FFPROBE`/`CAPIA_FFMPEG` → diretório *bundled* (`<exe>/ffmpeg`) → `PATH`. Nada achado ⇒ `MEDIA_BACKEND_NOT_FOUND` (sem pânico). Um caminho explícito inexistente é erro (não cai para o `PATH`).
- **Limites:** stdout 8 MiB, stderr 64 KiB, timeout 30 s (processo morto), `-probesize`/`-analyzeduration`; demuxers de playlist/rede (`hls`, `concat`, `dash`, `sdp`, `rtsp`…) rejeitados.
- **Normalização:** tempo em `Ticks` por aritmética inteira (`parse_decimal_ticks`), frame rates em `Rational`, streams ordenados, stream padrão explícito (primeiro vídeo que não é capa; primeiro áudio), limites de plausibilidade (1.000 h, 65.536 px, 1.000 fps, 64 canais, 768 kHz), rotação 0/90/180/270, alpha só quando o `pix_fmt` declara.
- **Miniatura:** `extract_frame_png` (um quadro PNG, escala limitada) — o cache fica em `capia-assets`.
- **Testes:** unidade (normalização hostil, processos), `tests/real.rs` (ffprobe real sobre `tests/fixtures/media`, *goldens* em `tests/golden/`, backend falso com travamento/flood). `CAPIA_REQUIRE_FFMPEG=1` faz a ausência do binário FALHAR; `CAPIA_BLESS=1` regrava os goldens.
