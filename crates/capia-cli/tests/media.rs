//! CLI de jobs/cache/derivados de ponta a ponta (binário real, FFmpeg real).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU64 = AtomicU64::new(0);

struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "capia-cli-media-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn fixture(&self, name: &str, to: &str) -> PathBuf {
        let dst = self.0.join(to);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/media")
                .join(name),
            &dst,
        )
        .unwrap();
        dst
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn cmd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_capia"));
    c.args(args)
        .args(["--sync", "normal"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn capia(args: &[&str]) -> Out {
    let o = cmd(args).output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn json(o: &Out) -> Value {
    serde_json::from_str(o.stdout.lines().last().unwrap_or(""))
        .unwrap_or_else(|e| panic!("not JSON ({e}): {} / {}", o.stdout, o.stderr))
}

fn have_ffmpeg() -> bool {
    let ok = ["ffprobe", "ffmpeg"].iter().all(|b| {
        Command::new(b)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    assert!(
        ok || std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
        "CAPIA_REQUIRE_FFMPEG is set but ffmpeg/ffprobe are unavailable"
    );
    if !ok {
        eprintln!("SKIP (no ffmpeg)");
    }
    ok
}

macro_rules! need {
    () => {
        if !have_ffmpeg() {
            return;
        }
    };
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn setup(d: &Dir, fixtures: &[(&str, &str)]) -> (PathBuf, Vec<String>) {
    let p = d.file("p.capia");
    assert_eq!(capia(&["create", s(&p)]).code, 0);
    let mut ids = Vec::new();
    for (fx, to) in fixtures {
        let f = d.fixture(fx, to);
        let r = capia(&["asset", "import", s(&p), s(&f), "--json"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        ids.push(json(&r)["asset_id"].as_str().unwrap().to_owned());
    }
    (p, ids)
}

#[test]
fn index_frame_waveform_proxy_job_and_cache_commands() {
    need!();
    let d = Dir::new();
    let (p, ids) = setup(
        &d,
        &[
            ("cfr_gop.mp4", "m/cfr.mp4"),
            ("tone_44k.wav", "m/tone.wav"),
            ("video_audio.mp4", "m/va.mp4"),
        ],
    );
    let (vid, aud, av) = (&ids[0], &ids[1], &ids[2]);
    // índice
    let r = capia(&["media", "index", s(&p), vid, "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v = json(&r);
    assert_eq!(
        (v["frames"].as_u64(), v["keyframes"].as_u64()),
        (Some(50), Some(1))
    );
    assert_eq!(v["result"]["hit"], false);
    let again = json(&capia(&["media", "index", s(&p), vid, "--json"]));
    assert_eq!(again["result"]["hit"], true, "second run is a cache hit");
    // quadro exato (por índice e por tempo) + arquivo PPM
    let ppm = d.file("f.ppm");
    let r = capia(&[
        "media",
        "frame",
        s(&p),
        vid,
        "--index",
        "30",
        "--out",
        s(&ppm),
        "--json",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let f = json(&r);
    assert_eq!(f["index"], 30);
    assert_eq!(
        (f["width"].as_u64(), f["height"].as_u64()),
        (Some(64), Some(48))
    );
    assert_eq!(f["pixel_format"], "rgba8");
    assert!(
        std::fs::read(&ppm)
            .unwrap()
            .starts_with(b"P6\n64 48\n255\n")
    );
    let by_time = json(&capia(&[
        "media",
        "frame",
        s(&p),
        vid,
        "--at",
        "1.2",
        "--json",
    ]));
    assert_eq!(
        by_time["sha256"], f["sha256"],
        "t = 1.2 s is frame 30 at 25 fps"
    );
    // waveform
    let r = capia(&["media", "waveform", s(&p), aud, "--buckets", "8", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let w = json(&r);
    assert_eq!(w["total_samples"], 44_100);
    assert!(w["peaks"].as_array().unwrap().len() <= 8);
    // proxy
    let r = capia(&[
        "media",
        "proxy",
        s(&p),
        av,
        "--max-width",
        "32",
        "--max-height",
        "32",
        "--json",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let proxy = PathBuf::from(json(&r)["result"]["path"].as_str().unwrap());
    assert!(proxy.is_file());
    // jobs persistidos
    let r = capia(&["job", "list", s(&p), "--state", "completed", "--json"]);
    assert_eq!(r.code, 0);
    let jobs: Value = serde_json::from_str(r.stdout.trim()).unwrap();
    assert!(jobs.as_array().unwrap().len() >= 4);
    let jid = jobs[0]["id"].as_str().unwrap();
    let st = json(&capia(&["job", "status", s(&p), jid, "--json"]));
    assert_eq!(st["state"], "completed");
    // cache info/clean
    let info = json(&capia(&["cache", "info", s(&p), "--json"]));
    assert!(info["files"].as_u64().unwrap() >= 3);
    assert_eq!(capia(&["cache", "clean", s(&p), "--all"]).code, 0);
    assert_eq!(
        json(&capia(&["cache", "info", s(&p), "--json"]))["files"],
        0
    );
    // o projeto continua válido
    assert_eq!(capia(&["validate", s(&p)]).code, 0);
    // erros estruturados
    let r = capia(&["media", "index", s(&p), "ast_nope", "--json"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("ASSET_NOT_FOUND"), "{}", r.stderr);
    let r = capia(&["job", "status", s(&p), "job_nope", "--json"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("JOB_NOT_FOUND"));
}

fn wait_for(mut f: impl FnMut() -> bool, secs: u64, what: &str) {
    let t0 = Instant::now();
    while !f() {
        assert!(t0.elapsed() < Duration::from_secs(secs), "timeout: {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn make_long(d: &Dir) -> PathBuf {
    let out = d.file("m/long.mkv");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let st = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=240",
        ])
        .args(["-c:v", "mpeg4", "-q:v", "10"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

#[test]
fn a_job_cancelled_from_another_process_dies_and_leaves_no_cache() {
    need!();
    let d = Dir::new();
    let p = d.file("p.capia");
    assert_eq!(capia(&["create", s(&p)]).code, 0);
    let long = make_long(&d);
    let r = capia(&["asset", "import", s(&p), s(&long), "--json"]);
    let id = json(&r)["asset_id"].as_str().unwrap().to_owned();
    // processo 1: proxy longo (JPEG de alta qualidade)
    let child: Child = cmd(&[
        "media",
        "proxy",
        s(&p),
        &id,
        "--max-width",
        "640",
        "--max-height",
        "360",
        "--quality",
        "2",
        "--no-audio",
        "--json",
    ])
    .spawn()
    .unwrap();
    // processo 2: acha o job em execução e cancela
    let mut job = String::new();
    wait_for(
        || {
            let r = capia(&["job", "list", s(&p), "--state", "running", "--json"]);
            let v: Value = serde_json::from_str(r.stdout.trim()).unwrap_or(Value::Null);
            if let Some(j) = v.as_array().and_then(|a| a.first()) {
                job = j["id"].as_str().unwrap().to_owned();
                true
            } else {
                false
            }
        },
        30,
        "a running job",
    );
    let c = capia(&["job", "cancel", s(&p), &job]);
    assert_eq!(c.code, 0, "{}", c.stderr);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "the cancelled command fails");
    assert!(String::from_utf8_lossy(&out.stderr).contains("JOB_CANCELLED"));
    let st = json(&capia(&["job", "status", s(&p), &job, "--json"]));
    assert_eq!(st["state"], "cancelled");
    assert_eq!(
        json(&capia(&["cache", "info", s(&p), "--json"]))["files"],
        0
    );
}

#[test]
fn async_import_prints_the_ticket_then_finalizes() {
    need!();
    let d = Dir::new();
    let p = d.file("p.capia");
    assert_eq!(capia(&["create", s(&p)]).code, 0);
    let f = d.fixture("video_audio.mp4", "m/a.mp4");
    let r = capia(&["asset", "import", s(&p), s(&f), "--async", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let lines: Vec<Value> = r
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["ticket"]["state"], "pending");
    assert!(
        lines[0]["ticket"]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("fp1:")
    );
    assert!(
        lines[1]["finalized"]["asset_id"]
            .as_str()
            .unwrap()
            .starts_with("ast_")
    );
    let list = capia(&["asset", "list", s(&p), "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(list.stdout.trim())
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn force_relink_and_relink_folder_commands() {
    need!();
    let d = Dir::new();
    let (p, ids) = setup(
        &d,
        &[
            ("cfr_gop.mp4", "m/long.mp4"),
            ("video_audio.mp4", "m/va.mp4"),
        ],
    );
    // force-relink: dry-run não grava; real troca o conteúdo
    let other = d.fixture("video_only.mp4", "n/short.mp4");
    let dry = capia(&[
        "asset",
        "force-relink",
        s(&p),
        &ids[0],
        s(&other),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "{}", dry.stderr);
    assert_eq!(json(&dry)["dry_run"], true);
    let real = capia(&["asset", "force-relink", s(&p), &ids[0], s(&other), "--json"]);
    assert_eq!(real.code, 0, "{}", real.stderr);
    let v = json(&real);
    assert_eq!(v["asset_id"], ids[0]);
    assert_ne!(v["old_hash"], v["new_hash"]);
    // relink em lote: o vídeo+áudio some e reaparece numa pasta com OUTRO nome
    std::fs::remove_file(d.file("m/va.mp4")).unwrap();
    d.fixture("video_audio.mp4", "lib/deep/renamed.mov");
    let r = capia(&["asset", "relink-folder", s(&p), s(&d.file("lib")), "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v = json(&r);
    assert_eq!(v["applied"].as_array().unwrap().len(), 1);
    assert_eq!(v["applied"][0]["asset_id"], ids[1]);
    // limites inválidos são erro de uso
    assert_eq!(
        capia(&[
            "asset",
            "relink-folder",
            s(&p),
            s(&d.file("lib")),
            "--max-depth",
            "999"
        ])
        .code,
        2
    );
}
