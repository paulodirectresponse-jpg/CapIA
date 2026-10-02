#!/bin/sh
# S2 spike: synthetic media whose frame index N is burned into the picture as 16 binary blocks (robust to lossy coding).
# Source frame N is encoded BEFORE any frame dropping, so a VFR clip keeps the ORIGINAL index of each surviving frame.
set -e
OUT=${1:-/tmp/claude-0/s2}; mkdir -p "$OUT"; cd "$OUT"
BARS="geq=lum='if(lt(Y,64),255*bitand(floor(N/pow(2,floor(X/40))),1),128)':cb=128:cr=128"   # 640 px / 16 blocks of 40px
SRC30="-f lavfi -i testsrc2=size=640x360:rate=30:duration=20"
SRC2997="-f lavfi -i testsrc2=size=640x360:rate=30000/1001:duration=20"
# CFR 29.97, closed GOP 30 with B-frames (decode order != presentation order)
ffmpeg -hide_banner -loglevel error -y $SRC2997 -vf "$BARS,format=yuv420p" -c:v libx264 -g 30 -bf 2 -crf 14 cfr2997.mp4
# CFR 24 (ntsc-less)
ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=640x360:rate=24:duration=20 -vf "$BARS,format=yuv420p" -c:v libx264 -g 48 -bf 2 -crf 14 cfr24.mp4
# VFR (phone-like): drop ~35% of frames pseudo-randomly, keep real timestamps (gaps), B-frames on
ffmpeg -hide_banner -loglevel error -y $SRC30 -vf "$BARS,select='lt(mod(n*7919,100),65)+eq(n,0)',format=yuv420p" -fps_mode vfr -c:v libx264 -g 60 -bf 2 -crf 14 vfr.mp4
# VFR with long freezes (screen-recording-like): bursts and holds
ffmpeg -hide_banner -loglevel error -y $SRC30 -vf "$BARS,select='if(lt(mod(n,150),20),1,eq(mod(n,150),75))+eq(n,0)',format=yuv420p" -fps_mode vfr -c:v libx264 -g 90 -bf 2 -crf 14 vfr_bursty.mp4
ls -la "$OUT"/*.mp4 | awk '{print $5, $9}'
