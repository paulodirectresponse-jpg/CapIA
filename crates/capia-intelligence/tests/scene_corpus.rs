//! Corpus de cenas **anotado por construção** (ffmpeg/lavfi, determinístico): cortes secos, dissolves
//! (xfade), fades por preto e clipes **sem corte** (zoom/movimento contínuo). A métrica do critério da
//! Fase 4 é `(FP + FN) / N_anotado ≤ 5 %` (ADR-083). Sem ffmpeg: pula, salvo `CAPIA_REQUIRE_FFMPEG=1`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_intelligence::scenes::{
    Annotated, BoundaryKind, SceneEval, SceneParams, detect_media, evaluate,
};
use capia_media::{MediaConfig, MediaToolchain};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

// Só geradores DETERMINÍSTICOS (verificado: `gradients`/`sierpinski` sem cores/seed explícitos variam
// entre execuções do ffmpeg e foram descartados) — o corpus tem de ser reproduzível bit a bit.
const FPS: u32 = 25;
const SIZE: &str = "320x180";
const SOURCES: [&str; 6] = [
    "testsrc",
    "smptebars",
    "rgbtestsrc",
    "gradients=c0=0x203060:c1=0xe0b040:c2=0x20a070:nb_colors=3:seed=7",
    "mandelbrot",
    "yuvtestsrc",
];

/// Fontes **não usadas** no ajuste dos limiares (conjunto de validação separado).
const HELDOUT: [&str; 4] = [
    "pal100bars",
    "cellauto=random_seed=3",
    "life=random_seed=5",
    "gradients=c0=0x30c070:c1=0x802090:nb_colors=2:seed=3:speed=0.02:type=radial",
];

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        _ => {
            assert!(
                std::env::var("CAPIA_REQUIRE_FFMPEG").is_err(),
                "CAPIA_REQUIRE_FFMPEG=1 mas ffmpeg/ffprobe não foram encontrados"
            );
            None
        }
    }
}

/// `nome` ou `nome=opt` + opções comuns (o separador depende de já haver opção).
fn lavfi(source: &str, dur: f32) -> String {
    let sep = if source.contains('=') { ':' } else { '=' };
    let _ = dur;
    format!("{source}{sep}size={SIZE}:rate={FPS}")
}

/// Argumentos de entrada lavfi com duração por `-t` (nem todo gerador tem a opção `duration`).
fn input(source: &str, dur: f32) -> Vec<String> {
    vec![
        "-f".into(),
        "lavfi".into(),
        "-t".into(),
        format!("{dur}"),
        "-i".into(),
        lavfi(source, dur),
    ]
}

fn run(tc: &MediaToolchain, args: &[String], out: &Path) {
    let status = Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-nostdin"])
        .args(args)
        .args(["-pix_fmt", "yuv420p", "-c:v", "mpeg4", "-q:v", "2"])
        .arg(out)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg falhou ao gerar {out:?}");
}

/// `n` segmentos de `seg` s, concatenados com corte seco. Anotação: cortes em `k · seg · fps`.
fn hard_cuts(
    tc: &MediaToolchain,
    dir: &Path,
    n: usize,
    offset: usize,
) -> (PathBuf, Vec<Annotated>) {
    let seg = 2.4_f32;
    let mut a: Vec<String> = Vec::new();
    for i in 0..n {
        a.extend(input(SOURCES[(i + offset) % SOURCES.len()], seg));
    }
    let ins: String = (0..n).map(|i| format!("[{i}:v]")).collect();
    a.extend([
        "-filter_complex".into(),
        format!("{ins}concat=n={n}:v=1:a=0[v]"),
        "-map".into(),
        "[v]".into(),
    ]);
    let out = dir.join(format!("cuts_{n}_{offset}.mp4"));
    run(tc, &a, &out);
    let per = (seg * FPS as f32) as u64;
    let truth = (1..n as u64)
        .map(|k| Annotated {
            frame: k * per,
            kind: BoundaryKind::Cut,
            tolerance: 1,
        })
        .collect();
    (out, truth)
}

/// Dissolves (`xfade=fade`) de 1 s entre segmentos de 3 s. Anotação: meio da transição.
fn dissolves(
    tc: &MediaToolchain,
    dir: &Path,
    n: usize,
    offset: usize,
) -> (PathBuf, Vec<Annotated>) {
    let (seg, tr) = (3.0_f32, 1.0_f32);
    let mut a: Vec<String> = Vec::new();
    for i in 0..n {
        a.extend(input(SOURCES[(i + offset) % SOURCES.len()], seg));
    }
    let mut chain = String::new();
    let mut last = "[0:v]".to_owned();
    let mut truth = Vec::new();
    for i in 1..n {
        let off = i as f32 * (seg - tr);
        let label = format!("[x{i}]");
        chain.push_str(&format!(
            "{last}[{i}:v]xfade=transition=fade:duration={tr}:offset={off}{label};"
        ));
        last = label;
        truth.push(Annotated {
            frame: ((off + tr / 2.0) * FPS as f32) as u64,
            kind: BoundaryKind::Dissolve,
            tolerance: 8,
        });
    }
    chain.pop();
    a.extend(["-filter_complex".into(), chain, "-map".into(), last]);
    let out = dir.join(format!("dissolve_{n}_{offset}.mp4"));
    run(tc, &a, &out);
    (out, truth)
}

/// Fade-out + fade-in por preto (0,6 s cada) entre segmentos de 2,4 s.
fn fades(tc: &MediaToolchain, dir: &Path, n: usize, offset: usize) -> (PathBuf, Vec<Annotated>) {
    let seg = 2.4_f32;
    let mut a: Vec<String> = Vec::new();
    for i in 0..n {
        a.extend(input(SOURCES[(i + offset) % SOURCES.len()], seg));
    }
    let mut chain = String::new();
    for i in 0..n {
        chain.push_str(&format!(
            "[{i}:v]fade=t=in:st=0:d=0.6,fade=t=out:st=1.8:d=0.6[f{i}];"
        ));
    }
    let ins: String = (0..n).map(|i| format!("[f{i}]")).collect();
    chain.push_str(&format!("{ins}concat=n={n}:v=1:a=0[v]"));
    a.extend(["-filter_complex".into(), chain, "-map".into(), "[v]".into()]);
    let out = dir.join(format!("fade_{n}_{offset}.mp4"));
    run(tc, &a, &out);
    let per = (seg * FPS as f32) as u64;
    let truth = (1..n as u64)
        .map(|k| Annotated {
            frame: k * per,
            kind: BoundaryKind::Fade,
            tolerance: 18,
        })
        .collect();
    (out, truth)
}

/// Sem corte: um único plano com movimento contínuo.
fn no_cut(tc: &MediaToolchain, dir: &Path, name: &str, source: &str) -> (PathBuf, Vec<Annotated>) {
    let out = dir.join(format!("nocut_{name}.mp4"));
    run(tc, &input(source, 8.0), &out);
    (out, Vec::new())
}

fn scan(tc: &MediaToolchain, path: &Path) -> Vec<capia_intelligence::scenes::Boundary> {
    detect_media(
        tc,
        path,
        0,
        (FPS, 1),
        &SceneParams::default(),
        Duration::from_secs(120),
        &|| false,
    )
    .unwrap()
}

#[test]
fn annotated_corpus_meets_the_five_percent_cut_error_target() {
    let Some(tc) = toolchain() else { return };
    let dir = std::env::temp_dir().join(format!("capia-scene-corpus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut corpus = vec![
        hard_cuts(&tc, &dir, 6, 0),
        hard_cuts(&tc, &dir, 6, 2),
        hard_cuts(&tc, &dir, 5, 3),
        dissolves(&tc, &dir, 4, 0),
        dissolves(&tc, &dir, 4, 2),
        fades(&tc, &dir, 4, 0),
        fades(&tc, &dir, 4, 2),
        no_cut(&tc, &dir, "mandelbrot", "mandelbrot"),
        no_cut(
            &tc,
            &dir,
            "gradients",
            "gradients=c0=0xd03020:c1=0x2040e0:nb_colors=2:seed=21:speed=0.03",
        ),
        no_cut(&tc, &dir, "testsrc", "testsrc"),
    ];
    let (mut tp, mut fp, mut fnn, mut total) = (0usize, 0usize, 0usize, 0usize);
    let mut report = String::new();
    for (path, truth) in corpus.drain(..) {
        let det = scan(&tc, &path);
        let ev: SceneEval = evaluate(&truth, &det);
        report.push_str(&format!(
            "{}: truth={} det={} tp={} fp={} fn={} detected={:?}\n",
            path.file_name().unwrap().to_string_lossy(),
            ev.truth,
            ev.detected,
            ev.true_positives,
            ev.false_positives,
            ev.false_negatives,
            det.iter().map(|b| (b.frame, b.kind)).collect::<Vec<_>>()
        ));
        tp += ev.true_positives;
        fp += ev.false_positives;
        fnn += ev.false_negatives;
        total += ev.truth;
    }
    let error = (fp + fnn) as f64 / total as f64;
    eprintln!("{report}tp={tp} fp={fp} fn={fnn} total={total} error={error:.4}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(error <= 0.05, "erro de corte {error:.4} > 5%\n{report}");
}

#[test]
fn scanning_is_deterministic_for_the_same_file() {
    let Some(tc) = toolchain() else { return };
    let dir = std::env::temp_dir().join(format!("capia-scene-det-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (p, _) = hard_cuts(&tc, &dir, 4, 0);
    assert_eq!(scan(&tc, &p), scan(&tc, &p));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Validação com geradores que não participaram do ajuste: o desempenho aqui é o número honesto.
#[test]
fn heldout_sources_generalize() {
    let Some(tc) = toolchain() else { return };
    let dir = std::env::temp_dir().join(format!("capia-scene-held-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let seg = 2.4_f32;
    let n = HELDOUT.len();
    let mut a: Vec<String> = Vec::new();
    for s in HELDOUT {
        a.extend(input(s, seg));
    }
    let ins: String = (0..n).map(|i| format!("[{i}:v]")).collect();
    a.extend([
        "-filter_complex".into(),
        format!("{ins}concat=n={n}:v=1:a=0[v]"),
        "-map".into(),
        "[v]".into(),
    ]);
    let out = dir.join("held_cuts.mp4");
    run(&tc, &a, &out);
    let per = (seg * FPS as f32) as u64;
    let truth: Vec<Annotated> = (1..n as u64)
        .map(|k| Annotated {
            frame: k * per,
            kind: BoundaryKind::Cut,
            tolerance: 1,
        })
        .collect();
    let det = scan(&tc, &out);
    let ev = evaluate(&truth, &det);
    eprintln!("held-out: {ev:?} detected={det:?}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        ev.error_rate <= 0.25,
        "held-out error {:.3}: {det:?}",
        ev.error_rate
    );
}

/// Medição (não é gate): velocidade da varredura de cenas em 1080p. `cargo test --release -p
/// capia-intelligence --test scene_corpus -- --ignored --nocapture perf`.
#[test]
#[ignore = "medição"]
fn perf_scan_throughput_1080p() {
    let Some(tc) = toolchain() else { return };
    let dir = std::env::temp_dir().join(format!("capia-scene-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("p.mp4");
    let st = Command::new(tc.ffmpeg.as_ref().unwrap())
        .args([
            "-v", "error", "-y", "-nostdin", "-f", "lavfi", "-t", "120", "-i",
        ])
        .arg("testsrc2=size=1920x1080:rate=30")
        .args(["-pix_fmt", "yuv420p", "-c:v", "mpeg4", "-q:v", "5"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    let t0 = std::time::Instant::now();
    let b = detect_media(
        &tc,
        &out,
        0,
        (25, 1),
        &SceneParams::default(),
        Duration::from_secs(600),
        &|| false,
    )
    .unwrap();
    let el = t0.elapsed().as_secs_f64();
    eprintln!(
        "PERF scenes 1080p: 120 s de vídeo em {el:.2} s ⇒ {:.1}× tempo real; fronteiras={}",
        120.0 / el,
        b.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
