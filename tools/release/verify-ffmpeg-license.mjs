#!/usr/bin/env node
// Verifica que o FFmpeg empacotado é a configuração LGPL aprovada (ADR-032): sem GPL, sem nonfree, sem x264/x265.
// Uso: node tools/release/verify-ffmpeg-license.mjs <dir-com-ffmpeg> [--allow-dev-unapproved] [--require-metadata]
//        [--report <arquivo.json>]
// Saída 0 = aprovado (ou, com --allow-dev-unapproved, reprovado mas rotulado "dev/test only" no relatório).
// Nunca declara aprovado um binário que falhe nas regras: `--allow-dev-unapproved` só muda o código de saída
// e marca `distributable: false` (o artefato do CI fica rotulado DEV-TEST-ONLY e o release.yml não aceita isso).
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Flags de configuração proibidas e a razão. */
export const FORBIDDEN_FLAGS = [
  ["--enable-gpl", "GPL: proibido (ADR-032)"],
  ["--enable-nonfree", "nonfree: proibido (ADR-032)"],
  ["--enable-libx264", "x264 é GPL: proibido (ADR-032)"],
  ["--enable-libx265", "x265 é GPL: proibido (ADR-032)"],
  ["--enable-libxvid", "xvid é GPL: proibido"],
  ["--enable-libfdk-aac", "fdk-aac é nonfree: proibido"],
  ["--enable-libfdk_aac", "fdk-aac é nonfree: proibido"],
  ["--enable-libvidstab", "vid.stab é GPL: proibido"],
  ["--enable-frei0r", "frei0r é GPL: proibido"],
  ["--enable-rubberband", "rubberband é GPL: proibido"],
];

/** Extrai a linha `configuration:` do `-version`/`-buildconf`. */
export function configurationFlags(text) {
  const flags = new Set();
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*configuration:\s*(.*)$/.exec(line);
    const src = m ? m[1] : /^\s*--/.test(line) ? line : "";
    for (const f of src.split(/\s+/)) if (f.startsWith("--")) flags.add(f);
  }
  return flags;
}

/**
 * Avalia os textos de `-version`, `-buildconf` e `-L` (licença). Função pura.
 * @returns {{approved:boolean, reasons:string[], warnings:string[], license:string, flags:string[]}}
 */
export function evaluateFfmpegText({ version = "", buildconf = "", license = "" }) {
  const reasons = [];
  const warnings = [];
  const flags = configurationFlags(`${version}\n${buildconf}`);
  if (flags.size === 0) {
    reasons.push("não foi possível ler a configuração de build (sem linha `configuration:`)");
  }
  for (const [flag, why] of FORBIDDEN_FLAGS) {
    if (flags.has(flag)) reasons.push(`${flag}: ${why}`);
  }
  for (const f of flags) {
    if (/^--enable-lib(x264|x265)\b/.test(f) && !FORBIDDEN_FLAGS.some(([k]) => k === f)) {
      reasons.push(`${f}: x264/x265 proibidos`);
    }
  }
  // O texto de licença impresso pelo binário vale mais que a configuração declarada.
  const lic = license.replace(/\s+/g, " ");
  let licenseKind = "unknown";
  if (/GNU Lesser General Public License/i.test(lic)) licenseKind = "LGPL";
  if (/GNU General Public License/i.test(lic.replace(/GNU Lesser General Public License/gi, ""))) {
    licenseKind = "GPL";
  }
  if (licenseKind === "GPL") reasons.push("o binário declara licença GPL (ffmpeg -L)");
  if (/nonfree/i.test(lic) && /unredistributable/i.test(lic)) {
    reasons.push("o binário declara ser não redistribuível (nonfree)");
  }
  if (flags.has("--enable-version3")) {
    warnings.push(
      "--enable-version3: resulta em LGPL-3.0 (a build aprovada pela ADR-032 não usa version3); exige revisão jurídica",
    );
  }
  if (!flags.has("--disable-autodetect")) {
    warnings.push("--disable-autodetect ausente: a build pode ter dependências não declaradas");
  }
  return {
    approved: reasons.length === 0,
    reasons,
    warnings,
    license: licenseKind,
    flags: [...flags].sort(),
  };
}

function run(exe, args) {
  const r = spawnSync(exe, args, { encoding: "utf8", timeout: 30000, windowsHide: true });
  if (r.error) return { ok: false, text: String(r.error.message) };
  return { ok: r.status === 0, text: `${r.stdout ?? ""}\n${r.stderr ?? ""}` };
}

const exeName = (b) => (process.platform === "win32" ? `${b}.exe` : b);

/** Inspeciona um diretório com ffmpeg/ffprobe (e DLLs/licenças). */
export function inspectDir(dir, { requireMetadata = false } = {}) {
  const reasons = [];
  const warnings = [];
  const ffmpeg = join(dir, exeName("ffmpeg"));
  const ffprobe = join(dir, exeName("ffprobe"));
  if (!existsSync(ffmpeg)) reasons.push(`ffmpeg ausente em ${dir}`);
  if (!existsSync(ffprobe)) reasons.push(`ffprobe ausente em ${dir}`);
  if (reasons.length > 0) return { approved: false, reasons, warnings, files: [] };
  const v = run(ffmpeg, ["-hide_banner", "-version"]);
  const b = run(ffmpeg, ["-hide_banner", "-buildconf"]);
  const l = run(ffmpeg, ["-hide_banner", "-L"]);
  const ev = evaluateFfmpegText({ version: v.text, buildconf: b.text, license: l.text });
  reasons.push(...ev.reasons);
  warnings.push(...ev.warnings);
  const files = readdirSync(dir);
  const hasLicenseFile = files.some((f) => /^(LICENSE|COPYING|LICENSES?)\b/i.test(f));
  if (!hasLicenseFile)
    reasons.push("arquivos de licença (LICENSE*/COPYING*) ausentes ao lado do binário");
  const metaPath = join(dir, "ffmpeg-build.json");
  let metadata = null;
  if (existsSync(metaPath)) {
    try {
      metadata = JSON.parse(readFileSync(metaPath, "utf8"));
    } catch {
      reasons.push("ffmpeg-build.json inválido");
    }
  } else if (requireMetadata) {
    reasons.push("ffmpeg-build.json (origem, tag, SHA-256 do fonte, id do build) ausente");
  } else {
    warnings.push("ffmpeg-build.json ausente (obrigatório em release)");
  }
  return {
    approved: reasons.length === 0,
    reasons,
    warnings,
    license: ev.license,
    flags: ev.flags,
    version_line: v.text.split(/\r?\n/).find((x) => x.trim()) ?? "",
    metadata,
    files,
  };
}

function main(argv) {
  const dir = argv.find((a) => !a.startsWith("--"));
  if (!dir) {
    console.error(
      "uso: verify-ffmpeg-license.mjs <dir> [--allow-dev-unapproved] [--require-metadata] [--report f.json]",
    );
    return 2;
  }
  const ri = argv.indexOf("--report");
  const reportFile = ri >= 0 ? argv[ri + 1] : null;
  const res = inspectDir(dir, { requireMetadata: argv.includes("--require-metadata") });
  const allowDev = argv.includes("--allow-dev-unapproved");
  const report = {
    ...res,
    distributable: res.approved,
    label: res.approved ? "approved-lgpl" : allowDev ? "DEV-TEST-ONLY-UNAPPROVED" : "rejected",
  };
  if (reportFile) writeFileSync(reportFile, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  if (res.approved) return 0;
  console.error(`FFmpeg NÃO aprovado: ${res.reasons.join("; ")}`);
  return allowDev ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(main(process.argv.slice(2)));
}
