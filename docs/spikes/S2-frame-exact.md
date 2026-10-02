# S2 — Decode frame-exato, VFR e sincronia A/V

**Pergunta:** a regra `frame_at(t) = último frame com pts ≤ t` (ADR-007) sobre um *frame index* dá seek frame-exato e cadência correta com CFR/VFR? Sincronia A/V? E no Windows (HW decode, rotação, HDR)?

**Resultado:** **parte lógica CONFIRMADA** (executada, mídia sintética). **Parte Windows/hardware NÃO MEDIDA.**

## Método

`spikes/s2-frame-exact/`: `generate.sh` cria 4 clipes H.264 com B-frames (CFR 29,97, CFR 24, VFR ~35% de frames descartados, VFR com pausas longas). O **índice do frame é gravado na imagem** como 16 blocos binários (robusto a compressão), *antes* do descarte de frames. A **verdade de referência é calculada pela receita de geração**, não pelo ffprobe/ffmpeg — o teste não valida o ffmpeg com o próprio ffmpeg. `test.py` e `extra.py` executam os testes. (ffmpeg 6.1 do container.)

## Resultados

| Teste | Resultado |
|---|---|
| **A. pts → Ticks** (aritmética racional, sem float; 705.600.000/s) | **Exato** (erro 0 ticks) nos 4 clipes; todos os pts inteiros em ticks. Time bases reais encontradas: 1/30000, 1/12288, 1/15360 — **1/15360 e 1/12288 não dividem 705.600.000**; a conversão deve ser `pts × num × 705.6M / den` em inteiro de 128 bits com arredondamento definido (erro ≤ 0,5 tick ≈ 0,7 ns) |
| **B. Seek por índice** (nossa regra: ordinal = nº de frames com pts ≤ t, decode por ordinal) | **60/60 exato** em cada um dos 4 clipes (240 amostras aleatórias) |
| **B′. Seek ingênuo `ffmpeg -ss t`** | **0/60, 1/60, 0/60, 0/60**: devolve o frame **seguinte** (primeiro com pts ≥ t), errando por 1 a 3 frames em VFR leve e por **até 75 frames** no clipe com pausas (devolve o próximo frame após a pausa em vez de segurar o anterior) |
| **C. Conformar VFR → grade de saída**: filtro `fps` do ffmpeg vs. sample-and-hold | Diverge em **113/300 (23,976), 102/300 (24), 77/300 (25), 41/300 (50)**; coincide em 29,97 e 59,94 para este conteúdo. Cadência de saída **não** pode depender do filtro `fps` do ffmpeg |
| **D. Áudio AAC (priming 1024 amostras)** | Erro de **+1 amostra (0,021 ms)** com o comportamento padrão do libav (honra *edit list*); **+1025 amostras (21,35 ms)** se a edit list é ignorada. O demux/decode **deve** manter o tratamento padrão do libav |
| **E. Rotação** | `ffprobe` expõe side-data de rotação (`90`); libavcodec **não** rotaciona sozinho — o engine aplica a display matrix |
| **F. Custo do índice** | 17.983 pacotes (10 min, 96,7 MB): **0,18 s** de varredura de pacotes (sem decodificar) |

## Não medido (requer Windows/hardware/mídia real)

- Decode por hardware (D3D11VA/D3D12VA/NVDEC/QSV) e seu comportamento de seek/latência; a build LGPL (S3) *contém* os hwaccels, mas não foram executados.
- **HDR (HLG/PQ) → SDR**; rotação aplicada no render; mídia real de iPhone/Android (só sintética aqui); bindings Rust do libav (usou-se o CLI); determinismo entre threads do decoder; decode de HEVC/ProRes/VP9 reais além do que o ffmpeg decodifica no container.

## Decisões / impacto

1. **ADR-035** (novo): o engine é dono da seleção de frame e da conformação de cadência; `-ss` e o filtro `fps` do ffmpeg **não** são usados para timing em preview nem export. `TIMELINE_ENGINE.md` §1.5 permanece; a conversão pts→ticks passa a ser regra explícita.
2. Teste permanente (Fase 2): reproduzir `generate.sh`/`test.py` como teste de conformidade em `capia-media` (corpus em `/testdata`, gerado por script).
