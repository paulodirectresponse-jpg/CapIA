// Valida out.mp4 com ffprobe e decide o veredito. Uso: node verify.mjs <dir-saida> [engine-label]
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir = "out", engine = "unknown"] = process.argv.slice(2);
const reportPath = join(dir, "report.json");
const verdict = { engine, verdict: "H264_FAIL", reasons: [] };
if (!existsSync(reportPath)) {
  verdict.reasons.push("no report received from the page (host did not run the spike)");
} else {
  const report = JSON.parse(readFileSync(reportPath, "utf8"));
  verdict.userAgent = report.userAgent;
  verdict.probes = report.probes;
  verdict.encodeMs = report.encodeMs;
  if (!report.ok) verdict.reasons.push(`encode failed: ${report.error}`);
  else if (!existsSync(join(dir, "out.mp4"))) verdict.reasons.push("no mp4 produced");
  else {
    const probe = JSON.parse(
      execFileSync("ffprobe", ["-v", "error", "-show_format", "-show_streams", "-of", "json", join(dir, "out.mp4")], { encoding: "utf8" }),
    );
    const v = probe.streams.find((s) => s.codec_type === "video");
    const a = probe.streams.find((s) => s.codec_type === "audio");
    const p = report.profile ?? { w: 640, h: 360, fps: 30, seconds: 2 };
    verdict.profile = p;
    verdict.container = probe.format.format_name;
    verdict.durationSeconds = Number(probe.format.duration);
    verdict.video = v && { codec: v.codec_name, profile: v.profile, width: v.width, height: v.height, fps: v.r_frame_rate, pix_fmt: v.pix_fmt };
    verdict.audio = a && { codec: a.codec_name, sample_rate: a.sample_rate, channels: a.channels };
    if (!/mp4/.test(verdict.container)) verdict.reasons.push(`container ${verdict.container} is not mp4`);
    if (v?.codec_name !== "h264") verdict.reasons.push(`video codec ${v?.codec_name} is not h264`);
    if (a?.codec_name !== "aac") verdict.reasons.push(`audio codec ${a?.codec_name} is not aac`);
    if (Math.abs(verdict.durationSeconds - p.seconds) > Math.max(0.3, p.seconds * 0.02)) verdict.reasons.push(`duration ${verdict.durationSeconds}`);
    if (v && Math.abs(eval(v.r_frame_rate) - p.fps) > 0.5) verdict.reasons.push(`fps ${v.r_frame_rate}`);
    if (v && (v.width !== p.w || v.height !== p.h)) verdict.reasons.push(`size ${v.width}x${v.height}`);
    if (v && a && Math.abs(Number(v.duration) - Number(a.duration)) > 0.15) verdict.reasons.push(`A/V duration drift v=${v.duration} a=${a.duration}`);
    const frames = Number(execFileSync("ffprobe", ["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0", join(dir, "out.mp4")], { encoding: "utf8" }).trim());
    verdict.frames = frames;
    if (frames !== p.fps * p.seconds) verdict.reasons.push(`frames ${frames} != ${p.fps * p.seconds}`);
    if (a && Number(a.sample_rate) !== 48000) verdict.reasons.push(`audio rate ${a.sample_rate}`);
    if (verdict.reasons.length === 0) verdict.verdict = "H264_PASS";
  }
}
writeFileSync(join(dir, "verdict.json"), JSON.stringify(verdict, null, 2));
console.log(JSON.stringify(verdict, null, 2));
process.exit(verdict.verdict === "H264_PASS" ? 0 : 1);
