#!/usr/bin/env node
// Gera um projeto de EXEMPLO 100 % sintético: clips de vídeo/áudio gerados pelo FFmpeg (fontes lavfi:
// nenhum conteúdo de terceiros, nada versionado), um briefing curto em texto e um clip de referência;
// cria um `.capia` e importa os assets pela CLI `capia` (Command Engine / sistema de assets).
//
//   node tools/sample-project/make-sample.mjs [pasta-de-saída] [--no-project] [--dry-run]
//
// OPCIONAL: sem ffmpeg/ffprobe, imprime uma mensagem clara e sai com código 0 (nada é fabricado).
// Sem a CLI `capia` (nem cargo) gera só a mídia e o briefing e diz como importar depois.
// Saída padrão: ./sample-project-out/  →  media/*.mp4|wav|png, briefing.txt, sample.capia
// (+ sample.capia-cache/ ao importar). A pasta de saída é descartável.
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { delimiter, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const BRIEFING = `Briefing de exemplo (SINTÉTICO — nenhum produto ou marca real)

Produto: Aplicativo fictício "Foco Diário" (agenda de hábitos).
Público: pessoas de 25 a 40 anos que querem criar rotina.
Oferta: 7 dias grátis; depois R$ 19,90/mês. Cancele quando quiser.
Objetivo: instalar o app e iniciar o teste grátis.
Tom: direto, amigável, sem exageros.
Plataforma: vídeo vertical 9:16 para anúncios em redes sociais.
Duração: 20 a 30 segundos.
Chamada para ação: "Comece seus 7 dias grátis agora".
Deve incluir: o gancho nos 3 primeiros segundos; legenda em todo o vídeo.
Deve evitar: promessas de resultado garantido.
`;

/** Clips sintéticos (todos de fontes `lavfi`). `ffArgs` vão ao ffmpeg depois de `-y -v error`. */
export function mediaSpecs(outDir) {
  const m = (name) => join(outDir, "media", name);
  const vid = (src, secs, name, extra = []) => ({
    name,
    role: name.startsWith("reference") ? "reference" : "raw",
    ffArgs: [
      "-f",
      "lavfi",
      "-i",
      src,
      "-t",
      String(secs),
      "-c:v",
      "mpeg4",
      "-q:v",
      "4",
      ...extra,
      m(name),
    ],
  });
  return [
    {
      name: "raw-talking-head.mp4",
      role: "raw",
      ffArgs: [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1080x1920:rate=30:duration=12",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=220:duration=12:sample_rate=48000",
        "-filter_complex",
        "[1:a]tremolo=f=4:d=0.6,aformat=channel_layouts=stereo[a]",
        "-map",
        "0:v",
        "-map",
        "[a]",
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        "-c:a",
        "aac",
        "-b:a",
        "96k",
        m("raw-talking-head.mp4"),
      ],
    },
    vid("mandelbrot=size=1080x1920:rate=30", 5, "raw-broll-1.mp4", ["-an"]),
    vid("gradients=size=1080x1920:rate=30:speed=0.05", 5, "raw-broll-2.mp4", ["-an"]),
    {
      // referência: 3 "planos" de cores diferentes com cortes secos (bom para o Reference Analyzer)
      name: "reference-ad.mp4",
      role: "reference",
      ffArgs: [
        "-f",
        "lavfi",
        "-i",
        "color=c=#C83232:s=540x960:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "color=c=#3366CC:s=540x960:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "color=c=#32A852:s=540x960:r=30:d=2",
        "-filter_complex",
        "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
        "-map",
        "[v]",
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        m("reference-ad.mp4"),
      ],
    },
    {
      name: "music.wav",
      role: "raw",
      ffArgs: [
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:duration=30:sample_rate=48000",
        "-af",
        "tremolo=f=2:d=0.4,volume=0.4",
        m("music.wav"),
      ],
    },
    {
      name: "logo.png",
      role: "raw",
      ffArgs: [
        "-f",
        "lavfi",
        "-i",
        "color=c=#3366CC:s=600x600:d=1",
        "-frames:v",
        "1",
        m("logo.png"),
      ],
    },
  ];
}

/** Comandos `capia` (argumentos APÓS o executável) para criar o projeto e importar os assets. */
export function cliPlan(outDir, specs) {
  const project = join(outDir, "sample.capia");
  return [
    { label: "create", args: ["create", project] },
    ...specs.map((s) => ({
      label: `import ${s.name}`,
      args: ["asset", "import", project, join(outDir, "media", s.name)],
    })),
    { label: "assets", args: ["asset", "list", project] },
  ];
}

export function parseArgs(argv) {
  const flags = new Set(argv.filter((a) => a.startsWith("--")));
  const positional = argv.filter((a) => !a.startsWith("--"));
  return {
    outDir: resolve(positional[0] ?? "sample-project-out"),
    noProject: flags.has("--no-project"),
    dryRun: flags.has("--dry-run"),
  };
}

export function findOnPath(name, pathVar = process.env.PATH ?? "", exists = existsSync) {
  const exts = process.platform === "win32" ? [".exe", ".cmd", ""] : [""];
  for (const dir of pathVar.split(delimiter).filter(Boolean))
    for (const e of exts) if (exists(join(dir, name + e))) return join(dir, name + e);
  return null;
}

const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));

/** Como invocar a CLI: binário informado/compilado, ou `cargo run` se houver cargo; senão null. */
export function resolveCli({ env = process.env, exists = existsSync, which = findOnPath } = {}) {
  if (env.CAPIA_CLI_BIN && exists(env.CAPIA_CLI_BIN)) return [env.CAPIA_CLI_BIN];
  const ext = process.platform === "win32" ? ".exe" : "";
  for (const d of ["release", "debug"]) {
    const p = join(root, "target", d, `capia${ext}`);
    if (exists(p)) return [p];
  }
  const cargo = which("cargo");
  return cargo ? [cargo, "run", "-q", "-p", "capia-cli", "--"] : null;
}

function main() {
  const { outDir, noProject, dryRun } = parseArgs(process.argv.slice(2));
  const ffmpeg = findOnPath("ffmpeg");
  if (!ffmpeg) {
    console.log(
      "ffmpeg não encontrado no PATH: o projeto de exemplo é opcional e foi PULADO (nada foi gerado).",
    );
    console.log("Instale o FFmpeg (ou use o do instalador do CapIA) e rode de novo.");
    return;
  }
  const specs = mediaSpecs(outDir);
  if (dryRun) {
    for (const s of specs) console.log(`ffmpeg -y -v error ${s.ffArgs.join(" ")}`);
    for (const c of cliPlan(outDir, specs)) console.log(`capia ${c.args.join(" ")}`);
    return;
  }
  mkdirSync(join(outDir, "media"), { recursive: true });
  for (const s of specs) {
    console.log(`gerando ${s.name} (${s.role})`);
    execFileSync(ffmpeg, ["-y", "-v", "error", ...s.ffArgs], { stdio: "inherit" });
  }
  writeFileSync(join(outDir, "briefing.txt"), BRIEFING, "utf8");
  console.log(`mídia e briefing em ${outDir}`);
  if (noProject) return;
  const cli = resolveCli();
  if (!cli) {
    console.log(
      "CLI `capia` e cargo não encontrados: projeto NÃO criado. Compile (`cargo build -p capia-cli`) ou defina CAPIA_CLI_BIN e rode de novo.",
    );
    return;
  }
  for (const step of cliPlan(outDir, specs)) {
    console.log(`capia ${step.label}`);
    const r = spawnSync(cli[0], [...cli.slice(1), ...step.args], { cwd: root, stdio: "inherit" });
    if (r.status !== 0) {
      console.error(`falhou: capia ${step.args.join(" ")} (exit ${r.status})`);
      process.exit(1);
    }
  }
  console.log(`\nProjeto de exemplo: ${join(outDir, "sample.capia")}`);
  console.log(
    "Abra-o no CapIA, crie uma sequence 9:16, arraste os clips e rode uma AI Run com o briefing.txt (IA opcional).",
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
