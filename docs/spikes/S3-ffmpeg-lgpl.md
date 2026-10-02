# S3 — Build FFmpeg LGPL

**Pergunta:** existe uma build LGPL/shared, sem GPL/nonfree, com os codecs de que o CapIA precisa, reproduzível e com manifesto de licenças? (parte técnica de OD-2, já decidida pelo PO: LGPL, dynamic linking, sem x264/x265 GPL)

**Resultado: viável. Consumir a build pronta da BtbN "como está" NÃO é recomendável; recomenda-se build própria mínima.** Decisão formalizada em ADR-032.

## 1. Build pré-pronta (a que o OpenCut novo fixa): BtbN `n8.1.3 *-lgpl-shared`

Baixada dos URLs de `crates/media/setup/ffmpeg.json` do OpenCut; **SHA-256 conferidos** (linux64 `703d321c…`, win64 `60a05579…` = valores pinados).

| Achado | Detalhe |
|---|---|
| **Licença real = LGPL-3.0-or-later** | Configurada com `--enable-version3`; `LICENSE.txt` é LGPLv3 e `ffmpeg -L` diz "version 3 or later". O `ffmpeg.json` do OpenCut rotula a mesma build como **"LGPL-2.1-or-later"** — **rótulo incorreto**; não copiar o rótulo |
| Sem GPL/nonfree | `--disable-libx264 --disable-libx265 --disable-libfdk-aac`, sem `--enable-gpl`/`--enable-nonfree` ✓ |
| Dezenas de libs externas | ~70 (`openssl`, `opencore-amr`, `vmaf`, `libass`, `libsrt`, `libssh`, `libvpl`, `libopenh264`, …) → manifesto de licenças e superfície de ataque grandes; `version3` provavelmente exigido por algumas delas |
| **Windows (`ffmpeg-n8.1.3-win64-lgpl-shared`)** | DLLs shared (`avcodec-62.dll` …). Strings confirmam: **encoders** `h264/hevc_nvenc`, `_amf`, `_qsv`, `_mf` (Media Foundation), `libopenh264`; **hwaccels de decode** `d3d11va`, `dxva2`, `d3d12va`, `nvdec`, `cuvid`, `vulkan`. Sem `libx264/libx265` |
| Encoders por software | **H.264: só `libopenh264` (Constrained Baseline)**. **HEVC SW: `libkvazaar`** (lento). AAC nativo, Opus, MP3 (lame), ProRes (`prores_ks`), DNxHD, FFV1, VP9/AV1 (libvpx/svtav1/aom/rav1e) |
| Tamanho | `libavcodec` Linux 92 MB |

Medições do fallback por software (4 vCPU, **conteúdo sintético fácil — não representativo de vídeo real**): OpenH264 1080p30 12 Mb/s: 300 frames em **1,9 s**; PSNR **46,9 dB** (x264 GPL no mesmo bitrate, só como régua: 53,0 dB); kvazaar HEVC 1080p: 300 frames em **13,2 s (~23 fps)**; `prores_ks` perfil HQ 1080p: 14 s.

## 2. Build mínima própria (prova de conceito)

Origem: tag `n8.1.3` do espelho oficial `github.com/FFmpeg/FFmpeg`, commit `1041abdc962f4cc4f394aa8de9dc5236c0c3b9e7` (`ffmpeg.org` bloqueado pelo proxy da sessão). Receita: `spikes/s3-ffmpeg-minimal/configure-minimal.sh` — `--enable-shared --disable-autodetect --disable-network --disable-everything` + componentes explícitos (decoders h264/hevc/prores/vp9/mjpeg/aac/mp3/opus/flac/vorbis/pcm; encoders aac/prores_ks/pcm/mjpeg; demuxers/muxers mov/mp4/matroska/wav/…).

| Verificação | Resultado |
|---|---|
| `configure` | **"License: LGPL version 2.1 or later"** (sem `version3`) |
| Compilação | OK (gcc 13, x86 asm **desligado** — `nasm` ausente; **não** representa desempenho de produção) |
| Dependências dinâmicas | **somente libc/libm/pthread** + as libs do próprio FFmpeg — nenhuma lib externa |
| Tamanho | `libavcodec.so` **5,7 MB** (vs 92 MB da BtbN) |
| Funcional | transcode H.264+AAC → ProRes HQ + AAC OK; `-c:v libx264`/qualquer encoder H.264/HEVC SW → "Unknown encoder" (esperado) |

## 3. O que NÃO foi verificado

- **Reprodutibilidade bit a bit** (duas builds com hash igual); só a receita e a origem estão pinadas.
- Build **Windows** própria (toolchain MSVC/MinGW) e habilitação dos encoders HW (precisam de `nv-codec-headers`, AMF headers e `libvpl`, todos permissivos, mas não testados aqui).
- Execução dos hwaccels Windows (ver S1/S2).
- Obrigações jurídicas: ver riscos abaixo.

## 4. Riscos que engenharia não elimina (registrados, não parecer jurídico)

1. **Patentes/royalties de H.264, HEVC e AAC** na distribuição comercial. Incluir `libopenh264` **compilado do fonte** (como na BtbN) **não** dá a cobertura de patentes da Cisco; essa cobertura vale para o **binário que a Cisco distribui** (baixado em runtime). Decisão jurídica/comercial.
2. Obrigações LGPL: linkagem dinâmica, texto da licença, oferta/hospedagem do fonte e da receita da build usada, permitir substituir as DLLs. Em LGPLv3 há cláusulas adicionais (informação de instalação apenas para "User Products"); a build própria sem `version3` as evita.
3. `prores_ks` é implementação independente do formato ProRes; avaliar risco de marca/uso antes de oferecê-lo como preset de entrega.
4. Encoders de hardware/MF dependem de drivers e do SO do usuário; o fallback de software é qualidade/velocidade inferior (OpenH264 só Baseline).
