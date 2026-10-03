#!/usr/bin/env bash
# Regenera as fixtures mínimas de mídia (ADR-051). Elas são VERSIONADAS: os testes não dependem do
# encoder da máquina. Rode só quando precisar mudar uma fixture, e commite o resultado.
set -euo pipefail
cd "$(dirname "$0")/../tests/fixtures/media"
F="ffmpeg -hide_banner -loglevel error -y -bitexact -fflags +bitexact -flags:v +bitexact -flags:a +bitexact"
# vídeo 64x48, 25 fps, 1 s (25 frames) + áudio 48 kHz estéreo 1 s
$F -f lavfi -i "testsrc=size=64x48:rate=25:duration=1" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" \
   -c:v libx264 -preset veryfast -crf 35 -pix_fmt yuv420p -g 25 -c:a aac -b:a 32k -ac 2 -shortest video_audio.mp4
# vídeo sem áudio
$F -f lavfi -i "testsrc=size=64x48:rate=25:duration=1" -c:v libx264 -preset veryfast -crf 35 -pix_fmt yuv420p -g 25 -an video_only.mp4
# áudio PCM (duração exata): 48 kHz mono 1 s
$F -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=1" -c:a pcm_s16le -ac 1 audio.wav
# imagens
$F -f lavfi -i "color=c=red:size=32x24:duration=1" -frames:v 1 -pix_fmt rgba image_alpha.png
$F -f lavfi -i "color=c=blue:size=40x30:duration=1" -frames:v 1 -pix_fmt yuvj420p image.jpg
# inválido: bytes que parecem mídia mas não são
printf 'this is not media at all\n' > invalid.mp4
