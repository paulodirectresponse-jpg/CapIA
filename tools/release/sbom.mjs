#!/usr/bin/env node
// SBOM (CycloneDX 1.5 JSON) + relatório de licenças + THIRD_PARTY_NOTICES.
//   node tools/release/sbom.mjs [--out-dir dist-release] [--ffmpeg-dir <dir>] [--no-cargo]
// Fontes: `cargo metadata` (crates, com licença) e pnpm-lock.yaml (pacotes JS; licença "NOASSERTION" — a
// licença JS é auditada por `pnpm check:licenses`). FFmpeg entra como componente próprio, com a configuração
// de build lida de `ffmpeg-build.json` (ADR-032).
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Componentes Rust (terceiros) a partir da saída de `cargo metadata`. Ignora crates do workspace. */
export function cargoComponents(metadata) {
  const ws = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((p) => !ws.has(p.id) && p.source)
    .map((p) => ({
      type: "library",
      name: p.name,
      version: p.version,
      purl: `pkg:cargo/${p.name}@${p.version}`,
      licenses: [{ expression: p.license ?? "NOASSERTION" }],
      ecosystem: "cargo",
    }))
    .sort((a, b) => a.purl.localeCompare(b.purl));
}

/** Pacotes JS do pnpm-lock.yaml (v9): chaves `'name@version':` sob `packages:`. */
export function pnpmComponents(lockText) {
  const out = new Map();
  let inPackages = false;
  for (const line of lockText.split(/\r?\n/)) {
    if (/^packages:\s*$/.test(line)) {
      inPackages = true;
      continue;
    }
    if (inPackages && /^\S/.test(line)) inPackages = false;
    if (!inPackages) continue;
    const m = /^ {2}'?((?:@[^/@']+\/)?[^@'\s]+)@([^':\s(]+)[^']*'?:\s*$/.exec(line);
    if (m) {
      out.set(`${m[1]}@${m[2]}`, {
        type: "library",
        name: m[1],
        version: m[2],
        purl: `pkg:npm/${m[1].replace("@", "%40")}@${m[2]}`,
        licenses: [{ expression: "NOASSERTION" }],
        ecosystem: "npm",
      });
    }
  }
  return [...out.values()].sort((a, b) => a.purl.localeCompare(b.purl));
}

export function ffmpegComponent(meta) {
  if (!meta) return null;
  return {
    type: "application",
    name: "ffmpeg",
    version: meta.source_tag ?? "unknown",
    purl: `pkg:generic/ffmpeg@${meta.source_tag ?? "unknown"}`,
    licenses: [{ expression: meta.license ?? "LGPL-2.1-or-later" }],
    ecosystem: "bundled",
    properties: Object.entries(meta).map(([name, value]) => ({
      name: `ffmpeg:${name}`,
      value: String(value),
    })),
  };
}

export function buildSbom({ version, components, now = new Date().toISOString() }) {
  return {
    bomFormat: "CycloneDX",
    specVersion: "1.5",
    version: 1,
    metadata: {
      timestamp: now,
      component: { type: "application", name: "CapIA", version },
      tools: [{ name: "capia tools/release/sbom.mjs" }],
    },
    components: components.map((c) => {
      const out = { ...c };
      delete out.ecosystem;
      return out;
    }),
  };
}

export function licenseReport(components) {
  const byLicense = {};
  for (const c of components) {
    const l = c.licenses?.[0]?.expression ?? "NOASSERTION";
    (byLicense[l] ??= []).push(`${c.name}@${c.version}`);
  }
  const copyleft = Object.keys(byLicense).filter((l) => /\bGPL\b|AGPL|LGPL|MPL/i.test(l));
  return {
    total: components.length,
    unknown: (byLicense.NOASSERTION ?? []).length,
    by_license: Object.fromEntries(Object.entries(byLicense).sort()),
    needs_review: copyleft,
  };
}

export function thirdPartyNotices({ version, components, ffmpeg }) {
  const lines = [
    `CapIA ${version} — THIRD_PARTY_NOTICES`,
    "",
    "Este produto inclui software de terceiros, listado abaixo com a licença declarada.",
    "Os textos completos acompanham cada pacote; o SBOM (sbom.cdx.json) tem a lista máquina-legível.",
    "",
  ];
  if (ffmpeg) {
    lines.push(
      "== FFmpeg (executáveis empacotados em ffmpeg/) ==",
      `Versão/tag: ${ffmpeg.version}`,
      `Licença: ${ffmpeg.licenses[0].expression} (build LGPL, sem GPL/nonfree/x264/x265 — ADR-032)`,
      "Oferta do código-fonte: o fonte exato e a receita de build estão no repositório do produto e na página da release;",
      "você pode substituir as bibliotecas/executáveis por builds LGPL compatíveis (ffmpeg/ é carregado do disco).",
      ...ffmpeg.properties.map((p) => `  ${p.name} = ${p.value}`),
      "",
    );
  }
  lines.push("== Componentes ==");
  for (const c of components) lines.push(`${c.name} ${c.version} — ${c.licenses[0].expression}`);
  return `${lines.join("\n")}\n`;
}

function arg(argv, k, d = null) {
  const i = argv.indexOf(k);
  return i >= 0 ? argv[i + 1] : d;
}

function main(argv) {
  const outDir = arg(argv, "--out-dir", "dist-release");
  mkdirSync(outDir, { recursive: true });
  const version = JSON.parse(readFileSync("package.json", "utf8")).version;
  let comps = [];
  if (!argv.includes("--no-cargo")) {
    const meta = JSON.parse(
      execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
        encoding: "utf8",
        maxBuffer: 256 * 1024 * 1024,
      }),
    );
    comps = comps.concat(cargoComponents(meta));
  }
  if (existsSync("pnpm-lock.yaml"))
    comps = comps.concat(pnpmComponents(readFileSync("pnpm-lock.yaml", "utf8")));
  const ffDir = arg(argv, "--ffmpeg-dir");
  let ff = null;
  if (ffDir && existsSync(join(ffDir, "ffmpeg-build.json"))) {
    ff = ffmpegComponent(JSON.parse(readFileSync(join(ffDir, "ffmpeg-build.json"), "utf8")));
    comps.push(ff);
  }
  writeFileSync(
    join(outDir, "sbom.cdx.json"),
    JSON.stringify(buildSbom({ version, components: comps }), null, 2),
  );
  writeFileSync(join(outDir, "license-report.json"), JSON.stringify(licenseReport(comps), null, 2));
  writeFileSync(
    join(outDir, "THIRD_PARTY_NOTICES.txt"),
    thirdPartyNotices({ version, components: comps, ffmpeg: ff }),
  );
  console.log(`SBOM: ${comps.length} componentes em ${outDir}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main(process.argv.slice(2));
