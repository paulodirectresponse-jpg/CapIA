#!/usr/bin/env node
// Gera a mídia sintética do teste de aceitação (nada de conteúdo de terceiros, nada versionado).
// Uso: node make-sample.mjs [pasta-de-saida]   (precisa de ffmpeg no PATH)
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";

const out = resolve(process.argv[2] ?? "sample-media");
mkdirSync(out, { recursive: true });
const ff = (...args) =>
  execFileSync("ffmpeg", ["-y", "-v", "error", ...args], { stdio: "inherit" });

// "talking head": 40 s, vertical 1080x1920, 30 fps, com uma "voz" (tom modulado) estéreo 48 kHz
ff(
  "-f",
  "lavfi",
  "-i",
  "testsrc2=size=1080x1920:rate=30:duration=40",
  "-f",
  "lavfi",
  "-i",
  "sine=frequency=220:duration=40:sample_rate=48000",
  "-filter_complex",
  "[1:a]tremolo=f=4:d=0.6,aformat=channel_layouts=stereo[a]",
  "-map",
  "0:v",
  "-map",
  "[a]",
  "-c:v",
  "mpeg4",
  "-q:v",
  "3",
  "-c:a",
  "aac",
  "-b:a",
  "128k",
  join(out, "talking-head.mp4"),
);
// 3 B-rolls de 6 s, cores/padrões diferentes, sem áudio
const brolls = [
  ["mandelbrot=size=1080x1920:rate=30", "broll-1.mp4"],
  [
    "life=size=1080x1920:rate=30:mold=10:ratio=0.1:death_color=#C83232:life_color=#00ff00",
    "broll-2.mp4",
  ],
  ["gradients=size=1080x1920:rate=30:speed=0.05", "broll-3.mp4"],
];
for (const [src, name] of brolls) {
  ff("-f", "lavfi", "-i", src, "-t", "6", "-c:v", "mpeg4", "-q:v", "3", "-an", join(out, name));
}
// música (45 s) e efeito sonoro (1 s)
ff(
  "-f",
  "lavfi",
  "-i",
  "sine=frequency=330:duration=45:sample_rate=48000",
  "-af",
  "tremolo=f=2:d=0.4,volume=0.4",
  join(out, "music.wav"),
);
ff(
  "-f",
  "lavfi",
  "-i",
  "anoisesrc=d=1:c=pink:r=48000:a=0.5",
  "-af",
  "afade=t=out:st=0.2:d=0.8",
  join(out, "sfx-whoosh.wav"),
);
// logo (imagem)
ff("-f", "lavfi", "-i", "color=c=#3366CC:s=600x600:d=1", "-frames:v", "1", join(out, "logo.png"));
console.log(`\nMídia de aceitação gerada em: ${out}`);
