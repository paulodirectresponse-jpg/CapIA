# capia-cli

Binário `capia`: cliente headless do Command Engine sobre arquivos `.capia`. Não duplica parser nem engine (usa `capia-project`).

```
capia create  <arq.capia> [--json]
capia inspect <arq.capia>            capia validate <arq.capia>
capia apply   <arq.capia> <tx.json|-> [--actor user|agent|api] [--label ...]
capia undo|redo <arq.capia>          capia history <arq.capia>
capia dump    <arq.capia> [--pretty]   # JSON determinístico do documento
```

`apply` aceita uma Transaction, uma lista de envelopes ou um envelope (os comandos serializáveis do engine). Atores `agent`/`api` passam por `preview → apply_plan`. Saída: 0 ok · 1 falha (JSON estruturado em stderr) · 2 uso. Testes: `tests/cli.rs` (executa o binário).

## M08 — jobs, derivados e relink

```
capia asset import <arq.capia> <arquivo> --async         # ticket na hora; hash+probe em job; finaliza e sai
capia asset force-relink <arq.capia> <asset-id> <arquivo> [--dry-run]   # troca de CONTEÚDO (valida clips)
capia asset relink-folder <arq.capia> <pasta> [--max-depth N] [--max-files N] [--follow-links]
capia media index    <arq.capia> <asset-id>                              # índice de quadros no cache
capia media frame    <arq.capia> <asset-id> [--at SEG | --index N] [--out quadro.ppm]
capia media waveform <arq.capia> <asset-id> [--buckets N]
capia media proxy    <arq.capia> <asset-id> [--max-width N] [--max-height N] [--quality 2..31] [--no-audio]
capia job list|status|cancel <arq.capia> [<job-id>] [--state ...]        # persistidos; cancel vale entre processos
capia cache info|clean <arq.capia> [--all]
```

`media index|waveform|proxy` rodam como **job** (`--priority`, `--progress`); `job cancel` de outro processo mata o FFmpeg. Testes: `tests/media.rs` (binário real, FFmpeg real).

## Fase 2 — render, export e encoders

```
capia media encoders [--json]                           # detecção real + política (nunca x264/x265)
capia render frame <p.capia> --sequence ID [--at SEG | --frame N] [--width W --height H] [--out q.ppm] [--json]
capia render audio <p.capia> --sequence ID [--start SEG] [--duration SEG] [--rate HZ] [--channels N] --out a.wav
capia export intermediate <p.capia> --sequence ID --out PASTA [--start SEG] [--duration SEG | --frames N] [--overwrite]
capia export mp4 <p.capia> --sequence ID --out a.mp4 [--codec h264|mpeg4-reference] [--encoder NOME] [--overwrite]
```

O render lê o **original** (nunca o proxy) e não escreve no documento. `export mp4 --codec h264` usa o primeiro encoder aprovado e disponível; sem nenhum ⇒ `MEDIA_ENCODER_UNAVAILABLE` (sem fallback e sem arquivo); `--encoder libx264` ⇒ `MEDIA_ENCODER_PROHIBITED`. `mpeg4-reference` (não é H.264) só por pedido explícito. Teste de aceitação da fase: `tests/phase2_e2e.rs`.
