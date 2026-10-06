#!/usr/bin/env bash
# Regenera as fixtures mínimas de mídia (ADR-051). Elas são VERSIONADAS: os testes não dependem do
# encoder da máquina. Rode só quando precisar mudar uma fixture, e commite o resultado.
set -euo pipefail
cd "$(dirname "$0")/../tests/fixtures/media"
F="ffmpeg -hide_banner -loglevel error -y -bitexact -fflags +bitexact -flags:v +bitexact -flags:a +bitexact"
# vídeo 64x48, 25 fps, 1 s (25 frames) + áudio 48 kHz estéreo 1 s
$F -f lavfi -i "testsrc=size=64x48:rate=25:duration=1" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" \
   -c:v libx264 -preset veryfast -crf 35 -pix_fmt yuv420p -g 25 -c:a aac -b:a 32k -ac 2 -shortest video_audio.mp4
# vídeo 64x48, 25 fps, 6 s + áudio 6 s (E2E do chat: "remova os primeiros 2 segundos")
$F -f lavfi -i "testsrc=size=64x48:rate=25:duration=6" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=6" \
   -c:v libx264 -preset veryfast -crf 35 -pix_fmt yuv420p -g 25 -c:a aac -b:a 32k -ac 2 -shortest long_6s.mp4
# vídeo + DOIS áudios (48 kHz e 44,1 kHz): seleção de stream padrão explícita
$F -f lavfi -i "testsrc=size=64x48:rate=25:duration=1" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" \
   -f lavfi -i "sine=frequency=880:sample_rate=44100:duration=1" -map 0:v -map 1:a -map 2:a \
   -c:v libx264 -preset veryfast -crf 35 -pix_fmt yuv420p -g 25 -c:a aac -b:a 32k -ac 2 -shortest multi_stream.mp4
# vídeo sem áudio
$F -f lavfi -i "testsrc=size=64x48:rate=25:duration=1" -c:v libx264 -preset veryfast -crf 35 -pix_fmt yuv420p -g 25 -an video_only.mp4
# áudio PCM (duração exata): 48 kHz mono 1 s
$F -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=1" -c:a pcm_s16le -ac 1 audio.wav
# imagens
$F -f lavfi -i "color=c=red:size=32x24:duration=1" -frames:v 1 -pix_fmt rgba image_alpha.png
$F -f lavfi -i "color=c=blue:size=40x30:duration=1" -frames:v 1 -pix_fmt yuvj420p image.jpg
# inválido: bytes que parecem mídia mas não são
printf 'this is not media at all\n' > invalid.mp4
# --- M08: quadros identificáveis (luma = 10 + passo·N) para provar o decode exato ---
# CFR, GOP longo (1 keyframe em 50 quadros) com B-frames: o quadro N tem luma 20+4N
$F -f lavfi -i "nullsrc=size=64x48:rate=25:duration=2,format=yuv420p,geq=lum='20+4*N':cb=128:cr=128" \
   -c:v libx264 -preset veryfast -crf 12 -pix_fmt yuv420p -g 50 -bf 2 -x264-params scenecut=0 -an cfr_gop.mp4
# VFR: 24 quadros (luma 20+8N) com um buraco de 2 quadros a cada 4 (timestamps irregulares reais)
$F -f lavfi -i "nullsrc=size=64x48:rate=25:duration=1,format=yuv420p,geq=lum='20+8*N':cb=128:cr=128,setpts='(N+floor(N/4)*2)/(25*TB)'" \
   -fps_mode passthrough -c:v libx264 -preset veryfast -crf 12 -pix_fmt yuv420p -g 12 -bf 2 -x264-params scenecut=0 -an vfr.mp4
# áudio 44,1 kHz estéreo PCM, 1 s
$F -f lavfi -i "sine=frequency=523:sample_rate=44100:duration=1" -c:a pcm_s16le -ac 2 tone_44k.wav
