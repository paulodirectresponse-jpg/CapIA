//! CLI de assets e mídia, ponta a ponta, com o ffprobe REAL e as fixtures versionadas.
//! `CAPIA_REQUIRE_FFMPEG=1` (CI) transforma a ausência do binário em falha.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "capia-cli-assets-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], envs: &[(&str, &str)]) -> Out {
    let mut c = Command::new(env!("CARGO_BIN_EXE_capia"));
    c.args(args)
        .args(["--sync", "normal"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn capia(args: &[&str]) -> Out {
    run(args, &[])
}

fn json(o: &Out) -> Value {
    serde_json::from_str(&o.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", o.stdout))
}

fn have_ffprobe() -> bool {
    let ok = Command::new("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    assert!(
        ok || std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
        "CAPIA_REQUIRE_FFMPEG is set but ffprobe is unavailable"
    );
    ok
}

#[test]
fn media_probe_prints_normalized_json_and_structured_errors() {
    if !have_ffprobe() {
        eprintln!("SKIP (no ffprobe)");
        return;
    }
    let v = fixture("video_audio.mp4");
    let o = capia(&["media", "probe", v.to_str().unwrap(), "--json"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let j = json(&o);
    assert_eq!(j["media"]["kind"], "video");
    assert_eq!(j["media"]["default_video"], 0);
    assert_eq!(j["media"]["default_audio"], 1);
    assert_eq!(j["media"]["duration"], 705_600_000);
    assert!(
        j["backend"]["ffprobe"]
            .as_str()
            .unwrap()
            .contains("ffprobe")
    );
    // determinismo: duas execuções, mesma saída
    let again = capia(&["media", "probe", v.to_str().unwrap(), "--json"]);
    assert_eq!(o.stdout, again.stdout);

    let bad = fixture("invalid.mp4");
    let o = capia(&["media", "probe", bad.to_str().unwrap(), "--json"]);
    assert_eq!(o.code, 1);
    let e: Value = serde_json::from_str(o.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(e["error"]["code"], "MEDIA_PROBE_FAILED");
    // texto humano
    let o = capia(&["media", "probe", fixture("audio.wav").to_str().unwrap()]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("kind:       audio"), "{}", o.stdout);
}

#[test]
fn missing_backend_is_a_structured_error_not_a_panic() {
    let o = run(
        &[
            "media",
            "probe",
            fixture("audio.wav").to_str().unwrap(),
            "--json",
            "--ffprobe",
            "/no/such/ffprobe",
        ],
        &[],
    );
    assert_eq!(o.code, 1);
    let e: Value = serde_json::from_str(o.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(e["error"]["code"], "MEDIA_BACKEND_NOT_FOUND");
    // sem PATH nenhum e sem caminho configurado
    let o = run(
        &[
            "media",
            "probe",
            fixture("audio.wav").to_str().unwrap(),
            "--json",
        ],
        &[("PATH", ""), ("CAPIA_FFPROBE", "")],
    );
    assert_eq!(o.code, 1, "{}", o.stderr);
    let e: Value = serde_json::from_str(o.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(e["error"]["code"], "MEDIA_BACKEND_NOT_FOUND");
}

#[test]
fn asset_lifecycle_import_list_inspect_offline_relink_verify() {
    if !have_ffprobe() {
        eprintln!("SKIP (no ffprobe)");
        return;
    }
    let d = Dir::new();
    let proj = d.file("p.capia");
    let p = proj.to_str().unwrap();
    assert_eq!(capia(&["create", p]).code, 0);

    // copia as fixtures para um lugar que podemos mover/alterar
    let media = d.file("media");
    std::fs::create_dir_all(&media).unwrap();
    let video = media.join("hero.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &video).unwrap();
    let image = media.join("logo.png");
    std::fs::copy(fixture("image_alpha.png"), &image).unwrap();
    let wav = media.join("voice.wav");
    std::fs::copy(fixture("audio.wav"), &wav).unwrap();

    let imp = |f: &Path| capia(&["asset", "import", p, f.to_str().unwrap(), "--json"]);
    let o = imp(&video);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let j = json(&o);
    assert_eq!(j["outcome"], "created");
    let id = j["asset_id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("ast_"));
    assert_eq!(imp(&image).code, 0);
    assert_eq!(imp(&wav).code, 0);

    // idempotente por conteúdo (e depois de reabrir: cada chamada é um processo novo)
    let again = json(&imp(&video));
    assert_eq!(
        (again["outcome"].as_str(), again["asset_id"].as_str()),
        (Some("existing"), Some(id.as_str()))
    );
    let copy = media.join("hero-copy.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &copy).unwrap();
    let alias = json(&imp(&copy));
    assert_eq!(
        (alias["outcome"].as_str(), alias["asset_id"].as_str()),
        (Some("aliased"), Some(id.as_str()))
    );

    let list = json(&capia(&["asset", "list", p, "--json"]));
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().all(|a| a["catalog"]["status"] == "online"));
    let o = capia(&["asset", "list", p]);
    assert!(o.stdout.contains("3 asset(s)"), "{}", o.stdout);

    let ins = json(&capia(&["asset", "inspect", p, &id, "--json"]));
    assert_eq!(
        ins["asset"]["catalog"]["content_hash"]
            .as_str()
            .unwrap()
            .len(),
        71
    );
    assert_eq!(ins["asset"]["catalog"]["media"]["kind"], "video");
    assert_eq!(ins["events"].as_array().unwrap().len(), 2, "import + alias");

    // verify ok
    let v = capia(&["asset", "verify", p, &id, "--json"]);
    assert_eq!(v.code, 0);
    assert_eq!(json(&v)["status"], "online");

    // some do disco (e a cópia também): projeto abre, asset fica offline
    std::fs::remove_file(&video).unwrap();
    std::fs::remove_file(&copy).unwrap();
    let list = json(&capia(&["asset", "list", p, "--json"]));
    let hero = list
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["asset_id"] == id.as_str())
        .unwrap();
    assert_eq!(hero["catalog"]["status"], "offline");
    let v = capia(&["asset", "verify", p, &id, "--json"]);
    assert_eq!(v.code, 1);
    assert_eq!(json(&v)["status"], "offline");

    // relink com arquivo diferente: rejeição estruturada
    let wrong = media.join("other.mp4");
    std::fs::copy(fixture("video_only.mp4"), &wrong).unwrap();
    let o = capia(&["asset", "relink", p, &id, wrong.to_str().unwrap(), "--json"]);
    assert_eq!(o.code, 1);
    let e: Value = serde_json::from_str(o.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(e["error"]["code"], "ASSET_HASH_MISMATCH");
    assert_ne!(
        e["error"]["details"]["expected_hash"],
        e["error"]["details"]["found_hash"]
    );

    // relink com o mesmo conteúdo em outro caminho: funciona
    let moved = media.join("renamed-hero.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &moved).unwrap();
    let o = capia(&["asset", "relink", p, &id, moved.to_str().unwrap(), "--json"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v = capia(&["asset", "verify", p, &id, "--json"]);
    assert_eq!(
        (v.code, json(&v)["status"].as_str().map(str::to_owned)),
        (0, Some("online".into()))
    );

    // arquivo alterado: verify detecta
    let mut bytes = std::fs::read(&moved).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff; // mesmo tamanho, outro conteúdo
    std::fs::write(&moved, bytes).unwrap();
    let v = capia(&["asset", "verify", p, &id, "--json"]);
    assert_eq!(v.code, 1);
    assert_eq!(json(&v)["status"], "modified");

    // o projeto continua válido
    let o = capia(&["validate", p, "--json"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(json(&o)["info"]["media_assets"], 3);
}

#[test]
fn import_errors_are_structured_and_leave_the_project_untouched() {
    if !have_ffprobe() {
        eprintln!("SKIP (no ffprobe)");
        return;
    }
    let d = Dir::new();
    let proj = d.file("p.capia");
    let p = proj.to_str().unwrap();
    assert_eq!(capia(&["create", p]).code, 0);
    let before = json(&capia(&["inspect", p, "--json"]));
    for (file, code) in [
        (fixture("invalid.mp4"), "MEDIA_PROBE_FAILED"),
        (d.file("missing.mp4"), "ASSET_FILE_NOT_FOUND"),
        (d.0.clone(), "ASSET_NOT_REGULAR_FILE"),
    ] {
        let o = capia(&["asset", "import", p, file.to_str().unwrap(), "--json"]);
        assert_eq!(o.code, 1, "{file:?}");
        let e: Value = serde_json::from_str(o.stderr.lines().last().unwrap()).unwrap();
        assert_eq!(e["error"]["code"], code, "{file:?}");
    }
    let empty = d.file("empty.mp4");
    std::fs::write(&empty, b"").unwrap();
    let o = capia(&["asset", "import", p, empty.to_str().unwrap(), "--json"]);
    let e: Value = serde_json::from_str(o.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(e["error"]["code"], "ASSET_EMPTY_FILE");
    let after = json(&capia(&["inspect", p, "--json"]));
    assert_eq!(before["digest"], after["digest"]);
    assert_eq!(before["revision"], after["revision"]);
    assert_eq!(after["media_assets"], 0);
}

#[test]
fn thumbnail_goes_to_the_cache_beside_the_project() {
    if !have_ffprobe() {
        eprintln!("SKIP (no ffprobe)");
        return;
    }
    let ffmpeg_ok = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ffmpeg_ok {
        assert!(
            std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
            "ffmpeg required"
        );
        return;
    }
    let d = Dir::new();
    let proj = d.file("p.capia");
    let p = proj.to_str().unwrap();
    assert_eq!(capia(&["create", p]).code, 0);
    let v = d.file("v.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &v).unwrap();
    let id = json(&capia(&[
        "asset",
        "import",
        p,
        v.to_str().unwrap(),
        "--json",
    ]))["asset_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let o = capia(&[
        "asset",
        "thumbnail",
        p,
        &id,
        "--at",
        "0.5",
        "--size",
        "32",
        "--json",
    ]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let path = PathBuf::from(json(&o)["path"].as_str().unwrap());
    assert!(path.exists());
    assert!(path.starts_with(d.file("p.capia-cache")), "{path:?}");
    // o cache é descartável: apagar tudo e o projeto continua íntegro e regenera
    std::fs::remove_dir_all(d.file("p.capia-cache")).unwrap();
    assert_eq!(capia(&["validate", p]).code, 0);
    assert_eq!(
        capia(&["asset", "thumbnail", p, &id, "--at", "0.5", "--size", "32"]).code,
        0
    );
    // áudio não tem miniatura
    let w = d.file("a.wav");
    std::fs::copy(fixture("audio.wav"), &w).unwrap();
    let aid = json(&capia(&[
        "asset",
        "import",
        p,
        w.to_str().unwrap(),
        "--json",
    ]))["asset_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(capia(&["asset", "thumbnail", p, &aid, "--json"]).code, 1);
}

#[test]
fn usage_errors_exit_with_2() {
    for args in [
        &["asset"][..],
        &["asset", "bogus", "x.capia"],
        &["asset", "import", "x.capia"],
        &["media"],
        &["media", "probe"],
        &["media", "probe", "a", "b"],
        &[
            "asset",
            "list",
            "x.capia",
            "--timeout-ms",
            "abc",
            "--ffprobe",
        ],
    ] {
        assert_eq!(capia(args).code, 2, "{args:?}");
    }
}
