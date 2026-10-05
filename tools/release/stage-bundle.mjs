#!/usr/bin/env node
// Prepara o que o instalador NSIS empacota e gera a configuração do Tauri para ele (sem tocar em tauri.conf.json,
// que segue sem sidecars para o CI normal não depender deles).
//
//   node tools/release/stage-bundle.mjs --ffmpeg-dir <dir> [--server-bin <exe>] [--allow-missing-server]
//        [--allow-dev-ffmpeg] [--webview embed|download|offline|skip] [--notices <THIRD_PARTY_NOTICES.txt>]
//        [--target x86_64-pc-windows-msvc] [--src-tauri apps/desktop/src-tauri]
//
// Saídas (em src-tauri, ignoradas pelo repositório): bundle-staging/ffmpeg/**, binaries/capia-server-<triple>.exe,
// tauri.installer.generated.json. Use:  pnpm --filter @capia/desktop tauri build --bundles nsis
//        --config src-tauri/tauri.installer.generated.json
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { inspectDir } from "./verify-ffmpeg-license.mjs";

export const WEBVIEW_MODES = {
  // bootstrapper embutido (~2 MB): instala o WebView2 em silêncio se faltar (precisa de rede NESSE caso)
  embed: { type: "embedBootstrapper", silent: true },
  // baixa o bootstrapper na instalação (instalador menor; precisa de rede se o WebView2 faltar)
  download: { type: "downloadBootstrapper", silent: true },
  // runtime completo dentro do instalador (~130 MB): para máquinas SEM rede
  offline: { type: "offlineInstaller", silent: true },
  // não instala (só para CI, onde o runner já tem o WebView2)
  skip: { type: "skip" },
};

/** Configuração mesclada ao tauri.conf.json (função pura, testada). */
export function buildInstallerConfig({ hasServer, webview = "embed", hasNotices = false }) {
  const mode = WEBVIEW_MODES[webview];
  if (!mode) throw new Error(`--webview desconhecido: ${webview}`);
  const resources = { "bundle-staging/ffmpeg/": "ffmpeg/" };
  if (hasNotices) resources["bundle-staging/THIRD_PARTY_NOTICES.txt"] = "THIRD_PARTY_NOTICES.txt";
  const bundle = {
    active: true,
    targets: ["nsis"],
    publisher: "CapIA",
    copyright: "CapIA",
    shortDescription: "CapIA — editor de vídeo com IA",
    resources,
    windows: {
      webviewInstallMode: mode,
      nsis: {
        // Por usuário: sem UAC, instala em %LOCALAPPDATA%\Programs\CapIA, atualiza sem elevação (ADR-draft C-2)
        installMode: "currentUser",
        installerHooks: "installer-hooks.nsh",
        languages: ["PortugueseBR", "English"],
        displayLanguageSelector: false,
        startMenuFolder: "CapIA",
        compression: "lzma",
      },
    },
  };
  if (hasServer) bundle.externalBin = ["binaries/capia-server"];
  return { bundle };
}

export function triple(target) {
  return target ?? "x86_64-pc-windows-msvc";
}

function parseArgs(argv) {
  const o = { flags: new Set() };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const next = argv[i + 1];
    if (next && !next.startsWith("--")) {
      o[a.slice(2)] = next;
      i++;
    } else {
      o.flags.add(a.slice(2));
    }
  }
  return o;
}

function copyDirFlat(src, dst) {
  mkdirSync(dst, { recursive: true });
  const copied = [];
  for (const name of readdirSync(src)) {
    const p = join(src, name);
    if (/\.(exe|dll|json|txt|md)$/i.test(name) || /^(LICENSE|COPYING|ffmpeg|ffprobe)$/i.test(name) || /^(LICENSE|COPYING)/i.test(name)) {
      copyFileSync(p, join(dst, name));
      copied.push(name);
    }
  }
  return copied;
}

export function stage(opts) {
  const srcTauri = resolve(opts["src-tauri"] ?? "apps/desktop/src-tauri");
  const log = [];
  const ff = opts["ffmpeg-dir"];
  if (!ff) throw new Error("--ffmpeg-dir é obrigatório");
  const inspected = inspectDir(resolve(ff), { requireMetadata: !opts.flags.has("allow-dev-ffmpeg") });
  if (!inspected.approved && !opts.flags.has("allow-dev-ffmpeg")) {
    throw new Error(`FFmpeg não aprovado (ADR-032): ${inspected.reasons.join("; ")}`);
  }
  const stagingRoot = join(srcTauri, "bundle-staging");
  rmSync(stagingRoot, { recursive: true, force: true });
  const files = copyDirFlat(resolve(ff), join(stagingRoot, "ffmpeg"));
  log.push(`ffmpeg: ${files.length} arquivos`);
  if (!inspected.approved) {
    writeFileSync(
      join(stagingRoot, "ffmpeg", "DEV-TEST-ONLY-UNAPPROVED.txt"),
      `Este FFmpeg NÃO é a build LGPL aprovada (ADR-032). Artefato de DESENVOLVIMENTO/TESTE: não distribuir.\n${inspected.reasons.join("\n")}\n`,
    );
  }
  if (opts.notices && existsSync(opts.notices)) {
    copyFileSync(opts.notices, join(stagingRoot, "THIRD_PARTY_NOTICES.txt"));
  }
  const binDir = join(srcTauri, "binaries");
  rmSync(binDir, { recursive: true, force: true });
  let hasServer = false;
  if (opts["server-bin"] && existsSync(opts["server-bin"])) {
    mkdirSync(binDir, { recursive: true });
    const ext = basename(opts["server-bin"]).toLowerCase().endsWith(".exe") ? ".exe" : "";
    copyFileSync(opts["server-bin"], join(binDir, `capia-server-${triple(opts.target)}${ext}`));
    hasServer = true;
    log.push("capia-server: empacotado como sidecar");
  } else if (opts.flags.has("allow-missing-server")) {
    mkdirSync(stagingRoot, { recursive: true });
    writeFileSync(
      join(stagingRoot, "SERVER-PLACEHOLDER.txt"),
      "capia-server (Track A) não foi construído nesta execução; o instalador não o inclui.\n",
    );
    log.push("capia-server: AUSENTE (permitido por --allow-missing-server; placeholder)");
  } else {
    throw new Error(
      "capia-server não encontrado: passe --server-bin <exe> ou --allow-missing-server (placeholder documentado)",
    );
  }
  const config = buildInstallerConfig({
    hasServer,
    webview: opts.webview ?? "embed",
    hasNotices: Boolean(opts.notices && existsSync(opts.notices)),
  });
  const cfgPath = join(srcTauri, "tauri.installer.generated.json");
  writeFileSync(cfgPath, JSON.stringify(config, null, 2));
  log.push(`config: ${cfgPath}`);
  return {
    configPath: cfgPath,
    hasServer,
    ffmpegApproved: inspected.approved,
    ffmpegWarnings: inspected.warnings,
    log,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const r = stage(parseArgs(process.argv.slice(2)));
    console.log(JSON.stringify(r, null, 2));
  } catch (e) {
    console.error(`stage-bundle: ${e.message}`);
    process.exit(1);
  }
}
