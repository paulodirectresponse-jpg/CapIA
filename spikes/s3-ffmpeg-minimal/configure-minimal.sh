#!/bin/sh
# S3 spike: minimal LGPL (v2.1+) shared build. No version3, no gpl, no nonfree, no external libs.
../ffsrc/configure \
  --prefix=$PWD/../prefix \
  --enable-shared --disable-static \
  --disable-programs --enable-ffmpeg --enable-ffprobe \
  --disable-doc --disable-debug --disable-autodetect --disable-network \
  --disable-x86asm \
  --disable-everything \
  --enable-avcodec --enable-avformat --enable-avutil --enable-swresample --enable-swscale --enable-avfilter \
  --enable-protocol=file \
  --enable-demuxer=mov,matroska,mp3,wav,image2,aac,ogg,flac \
  --enable-muxer=mp4,mov,matroska,wav,null \
  --enable-parser=h264,hevc,aac,vp9,av1,opus,mpegaudio \
  --enable-decoder=h264,hevc,prores,vp9,mjpeg,png,aac,mp3,opus,flac,vorbis,pcm_s16le,pcm_s24le,pcm_f32le \
  --enable-encoder=aac,prores_ks,pcm_s16le,mjpeg,png \
  --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,aac_adtstoasc \
  --enable-filter=scale,aresample,null,anull
