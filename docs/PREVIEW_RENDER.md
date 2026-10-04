# PREVIEW & RENDER

## 1. Arquitetura conceitual

```
Document snapshot (rev N)
      │  compile(sequence, time_range, quality)     ← função pura, compartilhada
      ▼
Render Graph (DAG por frame/intervalo)
  Source(media, frame_at) → TimeMap → Decode → ColorConvert → Transform/Crop/Fit → Effect* → Composite(blend, z)
  Text/Caption raster ──────────────────────────────────────────────────────────────┘        │
  Nested(sub-graph) ─────────────────────────────────────────────────────────────────┘        │
  Transition(A, B, progress) ─────────────────────────────────────────────────────────────────┘
  Audio: Source → Resample → Gain/Pan/Fades(keyframed) → TrackMix → MasterMix
      ▼
Executor
  ├─ Video: compositor GPU (wgpu: D3D12/Vulkan/Metal) — shaders WGSL únicos
  └─ Audio: mixer CPU f32 (determinístico)
      ▼
  Preview: Presenter (tela)            Export: Encoder (FFmpeg/libav, HW ou SW) + Muxer
```

**Uma única implementação de compilador e compositor** atende preview e export (ADR-003). Isso é o que garante equivalência semântica — não testes de "parecidos".

FFmpeg/libav é usado para **demux, decode, encode, mux, probe e conversões em lote** — não como engine de composição (nada de grafos `-filter_complex` gerados a partir da timeline: inviáveis para preview interativo, difíceis de depurar, e divergiriam do preview).

## 2. Avaliação de opções de preview

| Opção | Prós | Contras | Veredito |
|---|---|---|---|
| **A. WebCodecs + WebGL/WebGPU na WebView** | Mesmo ambiente da UI; HW decode do Chromium | Cobertura de codecs dependente da WebView (ProRes não, HEVC condicional); demux em JS; VFR e frame-exactness difíceis; **export teria de ser outra implementação** (paridade quebrada); WebKit (macOS/Linux no Tauri) não tem paridade de APIs → mata multiplataforma | Rejeitada como engine |
| **B. Nativo Rust: libav decode (HW: D3D11VA/NVDEC/QSV) + wgpu compositor** | Mesmo código do export; controle total de timing; codecs do FFmpeg; GPU moderna; multiplataforma via wgpu | Apresentar a imagem dentro da janela Tauri exige técnica específica (abaixo); mais engenharia | **Escolhida** |
| C. Engine nativa de terceiros (GStreamer, MLT) | Muito pronto | Modelo de dados próprio conflita com o nosso; MLT é GPL/LGPL e XML-based; difícil garantir nossas invariantes e integração com IA | Rejeitada; pode inspirar |
| D. Render no core e enviar frames codificados (JPEG/raw) à WebView por IPC | Simples | 1080p RGBA ≈ 8 MB/frame; latência e CPU altas | Só como fallback de baixa resolução |

WebGL/WebGPU na UI continuam úteis para **desenhar a timeline** (thumbnails/waveforms), não para compor o vídeo.

## 3. Composição e cor

- Pipeline em RGBA 16-bit float linear na GPU; entrada YUV convertida com matriz/range corretos (BT.601/709, full/limited) lidos do probe; rotação de metadados de celular aplicada no probe.
- V1: saída SDR Rec.709. HDR de celular (HLG/PQ) é **tonemapeado** para SDR com operador fixo e documentado (mesmo no preview e export). HDR de saída: fora do escopo V1.
- Text engine em Rust (shaping + rasterização, ex.: cosmic-text/swash ou Skia — escolha na Fase 2) usada pelos dois caminhos; fontes resolvidas por fingerprint, faltantes sinalizadas.
- Efeitos: registry com um shader WGSL por efeito; parâmetros animados avaliados pela mesma função de keyframes de `capia-model`.

## 4. Equivalência preview ↔ export

**Idênticos (obrigatório):** timing (mesmos Ticks, mesma `frame_at`), ordem de composição, transforms, keyframes, texto/layout, efeitos, transições, mix de áudio.
**Podem diferir no preview (sinalizado):** resolução de render (1/2, 1/4), uso de proxy em vez do original, qualidade de filtro de escala, frames descartados em playback sob carga (nunca deslocados no tempo).
**Teste de paridade:** render do mesmo frame pelo caminho "preview em resolução total sem proxy" e pelo caminho de export deve ser bit-idêntico antes do encode; com proxy, diferença perceptual abaixo do limiar (`TEST_STRATEGY.md`).

## 5. Apresentação do preview na janela (OD-1: **FECHADA — P2**, ADR-069; P1 eliminado por airspace medido; ver `docs/spikes/S1-preview-surface.md` §6)

Abstração:
```rust
trait PreviewPresenter { fn resize(&mut self, w, h, scale); fn present(&mut self, frame: &GpuFrame); fn set_visible(&mut self, bool); }
```
Candidatos:
- **P1 — Superfície nativa filha (preferido):** janela filha (HWND) com swapchain wgpu posicionada sob/sobre a região do preview, sincronizada com o layout da WebView (bounds enviados pela UI). Zero cópia para a WebView. Desafio: "airspace" — elementos HTML não podem sobrepor o preview (menus/dropdowns sobre o preview precisam ser janelas nativas ou evitar a região), resize/DPI sincronizado.
- **P2 — Shared buffer para a WebView:** WebView2 `SharedBuffer` (memória compartilhada mapeada como `ArrayBuffer` no JS) + upload para canvas WebGL. Uma cópia CPU por frame; viável em resolução de preview (ex.: 960×540 ≈ 2 MB/frame). Sem problema de airspace; mais latência/CPU; específico de WebView2 (no macOS exigiria outra implementação).
- **P3 — Janela de preview destacável** (sempre nativa) como recurso adicional, útil em dual monitor.

Critérios do spike: 30/60 fps estáveis 1080p→preview, latência de seek < 100 ms, CPU < 25% em notebook médio, comportamento em resize/DPI/multimonitor, overlays de UI sobre o preview (handles de transform, safe areas — que podem ser desenhados pelo próprio compositor como camada de UI).

## 6. Preview engine

- **Playback scheduler:** relógio mestre = áudio (dispositivo WASAPI); vídeo segue o relógio de áudio; descarta frames atrasados em vez de atrasar o áudio.
- **Pipeline de decode por clip ativo** com lookahead (pré-abre decoders dos próximos clips antes do corte); pool limitado de decoders HW.
- **Caches:** (1) frames decodificados (GPU, LRU por memória), (2) frames compostos por `(seq, revision-region-hash, t, quality)`, (3) render cache em disco para trechos pesados (opcional, "render in to out").
- **Invalidação** por evento `DocumentChanged`: só ranges afetados (incluindo instâncias de nested afetadas).
- **Qualidade adaptativa:** Auto (reduz resolução sob carga), Full, Half, Quarter; toggle "usar proxies".
- **Scrub:** seek frame-exato com prioridade sobre playback; mostra keyframe mais próximo imediatamente e refina.

## 7. Proxies, thumbnails, waveforms (jobs)

| Representação | Formato | Chave de cache |
|---|---|---|
| Proxy | H.264 intra-frequente (GOP curto) ou all-intra leve, ~540p/720p, CFR se origem VFR-pesada, áudio PCM/AAC | fingerprint + perfil |
| Frame index | Binário compacto | fingerprint + stream |
| Thumbnails | Tiles JPEG/WebP em 2–3 densidades (ex.: 1 por segundo, 1 por 0,25 s sob demanda) | fingerprint + densidade |
| Waveform | Pirâmide min/max por canal (níveis 2^k amostras) | fingerprint + stream |

Proxies são gerados em background após import (prioridade a clips já na timeline); o usuário pode desligar. Mídia leve (≤ 1080p H.264 com decode HW) pode dispensar proxy.

## 8. Escala: centenas/milhares de elementos

- Compilação incremental do render graph por **intervalo** (só clips ativos em `t`); índice por track O(log n).
- Clips fora da janela de tempo nunca são abertos; decoders são o recurso escasso → pool + LRU.
- Timeline UI virtualizada (só tiles visíveis).
- Texto: cache de rasterização por `(style, text, scale)`.
- Meta: projeto com 10.000 clips em 50 sequences abre em < 2 s (sem gerar caches) e mantém preview fluido na sequence ativa.

## 9. Export

- Entrada: `Deliverable` (sequence + preset). Executado como job `Render` sobre um **snapshot fixo**.
- Fonte: sempre mídia original (nunca proxy), resolução total.
- Encoders: HW (NVENC/QSV/AMF via libav) quando disponível; fallback SW conforme decisão de licenciamento (OD-2).
- Presets V1: H.264/AAC MP4 (padrão social), HEVC MP4, ProRes/DNxHR opcional (intermediário) se a build permitir.
- Loudness: normalização opcional para alvo LUFS (ex.: −14 LUFS) na mixagem final.
- Batch export de múltiplos deliverables com fila e paralelismo limitado por GPU.
- Verificação pós-export: probe do arquivo gerado (duração, fps, resolução, sync de streams) registrado no `ExportRecord`.

## 10. FFmpeg: forma de uso e licenciamento

- **In-process (libav* via bindings Rust)** para decode/preview/export (latência e acesso a frames HW).
- **Sidecar `ffmpeg` CLI** (mesma build) aceitável para jobs batch isolados (proxies, conversões) — isolamento de crash.
- **Política fechada (ADR-032, OD-2):** build **própria e mínima**, **LGPL** (sem `--enable-version3` → LGPL-2.1+), **bibliotecas compartilhadas**, **sem GPL/nonfree, sem x264/x265**, `--disable-autodetect --disable-network`, origem e build **pinadas e reproduzíveis**, manifesto de licenças por release. Não consumir builds de terceiros "como estão" (a BtbN `lgpl-shared` é LGPL-3.0 com ~70 libs externas).
- **Encoders atrás de abstração:** NVENC/AMF/QSV e Media Foundation como caminho principal no Windows; fallback por software: OpenH264 (Constrained Baseline) para H.264 e kvazaar para HEVC (lento); ProRes via `prores_ks` quando permitido. Riscos de patentes e do binário OpenH264 em `DECISIONS.md` ADR-032 e `docs/spikes/S3-ffmpeg-lgpl.md`.
- **Timing é do engine, não do ffmpeg (ADR-035):** nem `-ss` nem o filtro `fps` determinam seleção de frame/cadência; usa-se frame index próprio + `frame_at`.

## 11. Áudio

- Mixer f32, sample rate da sequence (48 kHz), resampler de alta qualidade (determinístico).
- Cadeia: clip (gain keyframed, pan, fades) → track (volume, mute/solo) → master (limiter opcional, loudness no export).
- Ducking automático (música sob voz) como **propriedade/automação explícita** gerada por comando (a IA pode criar), nunca um efeito "mágico" invisível.
- Sincronia validada por testes com mídia de claquete sintética (`TEST_STRATEGY.md`).

## 12. O que a Fase 2 entregou (e o que não)

- **Entregue (headless):** render graph e compositor CPU **de referência** determinístico (`capia-render`, ADR-063..065), decode persistente + cache de quadros (ADR-059/060), seek de áudio por índice + cache de PCM (ADR-061/062), preview headless (scheduler, `FrameSink`, playhead, cadência por relógio injetável, descarte de quadros obsoletos — ADR-066), export intermediário atômico e MP4 por `EncoderCapability` aprovado (ADR-067/068).
- **Equivalência preview ↔ export:** o preview chama o **mesmo** `render_frame`; testes comparam digests SHA-256 quadro a quadro (tocando e em scrub), depois de reabrir, com cache frio/quente/minúsculo e com proxy presente — todos idênticos.
- **Não entregue (de propósito):** compositor wgpu/RGBA16F (Fase 3; a referência CPU é o oráculo dele), texto, transições, efeitos, máscaras, rotação arbitrária, apresentação na janela (OD-1 aberta), H.264 de produção (`OUTPUT-H264` aberta).

## Preview no editor (Fase 3 — ADR-073/074/077)

- **Apresentador P2:** `render.frame` renderiza **dentro** do SharedBuffer do WebView2 (Windows) e o JS sobe a textura WebGL sem cópia por IPC; fora do WebView2/em falha, o **mesmo** apresentador recebe bytes por IPC binário (`data-transport` no canvas; o CI Windows exige `shared-buffer`). Sem janela nativa irmã ⇒ sem airspace.
- **Render único:** o preview chama o mesmo `render_frame` do export; `design_size` faz 540p/720p comporem como o export; texto/transições/fades estão no render (testes `editor_render.rs`).
- **Concorrência:** preparação sob o lock da sessão, compositor/decode fora (`Session::begin` → `Job`); grafo compilado em cache por `document.revision`.
- **Agendamento:** *latest-wins* (um pedido em voo + um pendente; o substituído conta como *dropped slot*); qualidade 540p/720p/Auto com histerese; "proxy" = resolução reduzida (modo de desempenho — o proxy de mídia **nunca** é fonte de decode, ADR-063).
- **Overlays (só UI, nunca na saída):** safe areas, caixa de transformação com mover/escalar (uma transação), seleção.
- **Áudio:** `render.audio` (mixer do export) em blocos de 0,5 s; relógio de áudio mestre a 1×.
- **Medido (CI/sandbox sem GPU, 5.000 clips sólidos, 404×720):** latência de quadro ≈ 14–36 ms p95 25–80; **CPU/pacing em GPU real = residual humano** (`tools/phase3-acceptance/gpu-residual.ps1`).

### Texto: resolução de fontes e fonte ausente (Fase 3)
O render de texto é **determinístico e único** (`capia-render/src/text.rs`, `ab_glyph`, mesma função para preview e export). Só há uma família embutida, `sans` (Liberation Sans Regular/Bold, SIL OFL — `docs/PROVENANCE.md`), embutida no binário: **o resultado não depende de fontes instaladas no sistema**. Pesos ≥ 600 usam o Bold. Qualquer outra `font_family` **não** falha: cai em `sans` e o render devolve o aviso `FONT_FAMILY_UNAVAILABLE` (visível nos metadados do quadro). Tamanho em ‰ da altura do quadro (idêntico em qualquer resolução), alinhamento esq./centro/dir., cor, fundo (caixa) e contorno; posição/escala/opacidade/keyframes são as propriedades comuns do clip.
