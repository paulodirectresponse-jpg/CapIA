//! **Teste de aceitação da Fase 2** (CLI ponta a ponta, binário real, FFmpeg real):
//! cria `.capia`, importa mídia real, monta `HOOK_A`, `HOOK_B` e `BODY_MASTER` (nested), aplica
//! transações, desfaz/refaz, fecha/reabre (cada chamada é um processo), renderiza, exporta MP4(s),
//! valida com ffprobe (duração, fps, quadros, áudio, sincronismo A/V) e valida o projeto.
//!
//! H.264: usa o encoder aprovado disponível (NVENC/QSV/AMF/Media Foundation/OpenH264). Sem nenhum,
//! o pedido H.264 FALHA de forma estruturada (sem fallback) e o fluxo é provado com o MP4 de
//! referência `mpeg4-reference` (pedido de forma explícita).

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
            "capia-phase2-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.file(name);
        std::fs::write(&p, text).unwrap();
        p
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

fn capia(args: &[&str]) -> Out {
    let o = Command::new(env!("CARGO_BIN_EXE_capia"))
        .args(args)
        .args(["--sync", "normal"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn ok_json(args: &[&str]) -> Value {
    let o = capia(args);
    assert_eq!(o.code, 0, "capia {args:?}\n{}\n{}", o.stdout, o.stderr);
    serde_json::from_str(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", o.stdout))
}

fn have_ffmpeg() -> bool {
    let ok = ["ffmpeg", "ffprobe"].iter().all(|b| {
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

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn make_av(d: &Dir, name: &str, pattern: &str, secs: u32, tone: u32) -> PathBuf {
    let out = d.file(name);
    let st = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("{pattern}=size=64x48:rate=30"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!(
            "aevalsrc=0.6*sin(2*PI*t*{tone})|0.6*sin(2*PI*t*({tone}+100)):s=48000:c=stereo"
        ))
        .args([
            "-t",
            &secs.to_string(),
            "-c:v",
            "mpeg4",
            "-g",
            "12",
            "-qscale:v",
            "3",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-shortest",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

fn ffprobe(file: &Path) -> Value {
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-count_packets", "-show_entries"])
        .arg("format=format_name,duration:stream=codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,sample_rate,channels,duration,nb_read_packets")
        .args(["-of", "json"])
        .arg(file)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn stream<'a>(v: &'a Value, kind: &str) -> &'a Value {
    v["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["codec_type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} stream in {v}"))
}

fn secs(v: &Value) -> f64 {
    v.as_str().unwrap().parse::<f64>().unwrap()
}

/// Valida um MP4 exportado: contêiner, codec, tamanho, fps, nº de quadros, áudio e sincronismo.
fn check_mp4(file: &Path, codec: &str, frames: u64) {
    let v = ffprobe(file);
    assert!(
        v["format"]["format_name"].as_str().unwrap().contains("mp4"),
        "{v}"
    );
    let vs = stream(&v, "video");
    assert_eq!(vs["codec_name"], codec);
    assert_eq!(
        (vs["width"].as_u64(), vs["height"].as_u64()),
        (Some(64), Some(48))
    );
    assert_eq!(vs["avg_frame_rate"], "30/1");
    assert_eq!(
        vs["nb_read_packets"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap(),
        frames
    );
    let as_ = stream(&v, "audio");
    assert_eq!(as_["sample_rate"], "48000");
    assert_eq!(as_["channels"].as_u64(), Some(2));
    let (vd, ad) = (secs(&vs["duration"]), secs(&as_["duration"]));
    let want = frames as f64 / 30.0;
    assert!(
        (vd - want).abs() <= 1.0 / 30.0,
        "video duration {vd} vs {want}"
    );
    assert!(
        (vd - ad).abs() <= 1.0 / 30.0,
        "A/V drift {} s",
        (vd - ad).abs()
    );
}

const FRAME: i64 = 23_520_000;

#[test]
fn phase2_acceptance_cli_end_to_end() {
    if !have_ffmpeg() {
        return;
    }
    let d = Dir::new();
    let p = d.file("campanha.capia");
    // 1. cria o projeto
    assert_eq!(capia(&["create", s(&p)]).code, 0);

    // 2. importa mídia REAL (3 clipes de 2 s: hook A, hook B, corpo)
    let a = make_av(&d, "hook_a.mp4", "testsrc2", 2, 440);
    let b = make_av(&d, "hook_b.mp4", "mandelbrot", 2, 660);
    let c = make_av(&d, "body.mp4", "rgbtestsrc", 3, 330);
    let mut ids = Vec::new();
    for f in [&a, &b, &c] {
        let v = ok_json(&["asset", "import", s(&p), s(f), "--json"]);
        ids.push(v["asset_id"].as_str().unwrap().to_owned());
    }
    let (ia, ib, ic) = (&ids[0], &ids[1], &ids[2]);

    // 3. sequences HOOK_A, HOOK_B e BODY_MASTER (nested) em transações
    let n1 = FRAME * 30; // 1 s
    let n2 = FRAME * 60; // 2 s
    let media = |id: &str, asset: &str, track: &str, start: i64, dur: i64, v: bool, au: bool| {
        format!(
            r#"{{"operation_id":"{id}","type":"insert_clip","track":"{track}","start":{start},
              "clip":{{"id":"{id}","duration":{dur},"content":{{"type":"media","asset":"{asset}","has_video":{v},"has_audio":{au}}}}}}}"#
        )
    };
    let setup = format!(
        r#"{{"label":"hooks","commands":[
          {{"operation_id":"sa","type":"create_sequence","id":"HOOK_A","name":"HOOK_A","frame_rate":"30"}},
          {{"operation_id":"sa1","type":"add_track","sequence":"HOOK_A","id":"A_V","kind":"visual"}},
          {{"operation_id":"sa2","type":"add_track","sequence":"HOOK_A","id":"A_A","kind":"audio"}},
          {{"operation_id":"sb","type":"create_sequence","id":"HOOK_B","name":"HOOK_B","frame_rate":"30"}},
          {{"operation_id":"sb1","type":"add_track","sequence":"HOOK_B","id":"B_V","kind":"visual"}},
          {{"operation_id":"sb2","type":"add_track","sequence":"HOOK_B","id":"B_A","kind":"audio"}},
          {media_a_v},{media_a_a},{media_b_v},{media_b_a}
        ]}}"#,
        media_a_v = media("hav", ia, "A_V", 0, n1, true, false),
        media_a_a = media("haa", ia, "A_A", 0, n1, false, true),
        media_b_v = media("hbv", ib, "B_V", 0, n1, true, false),
        media_b_a = media("hba", ib, "B_A", 0, n1, false, true),
    );
    let setup = d.write("setup.json", &setup);
    ok_json(&["apply", s(&p), s(&setup), "--json"]);
    let body = format!(
        r#"{{"label":"body","commands":[
          {{"operation_id":"sm","type":"create_sequence","id":"BODY_MASTER","name":"BODY_MASTER","frame_rate":"30"}},
          {{"operation_id":"sm1","type":"add_track","sequence":"BODY_MASTER","id":"M_V","kind":"visual"}},
          {{"operation_id":"sm2","type":"add_track","sequence":"BODY_MASTER","id":"M_A","kind":"audio"}},
          {{"operation_id":"nh","type":"insert_nested","track":"M_V","start":0,"sequence":"HOOK_A","id":"nest_a","duration":{n1},"follow_length":false}},
          {mv},{ma}
        ]}}"#,
        mv = media("bv", ic, "M_V", n1, n2, true, false),
        ma = media("ba", ic, "M_A", n1, n2, false, true),
    );
    let body = d.write("body.json", &body);
    ok_json(&["apply", s(&p), s(&body), "--json"]);

    // 4. troca de hook: desfazer/refazer (a 2ª transação põe HOOK_B por cima, 50%)
    // o documento sem `revision` (undo/redo também incrementam a revisão)
    let dump = |p: &Path| {
        let mut v: Value = serde_json::from_str(&capia(&["dump", s(p)]).stdout).unwrap();
        v.as_object_mut().unwrap().remove("revision");
        v
    };
    let before = dump(&p);
    let overlay = d.write(
        "overlay.json",
        &format!(
            r#"{{"label":"overlay hook B","commands":[
              {{"operation_id":"ov1","type":"add_track","sequence":"BODY_MASTER","id":"M_V2","kind":"visual"}},
              {{"operation_id":"ov2","type":"insert_nested","track":"M_V2","start":0,"sequence":"HOOK_B","id":"nest_b","duration":{n1},"follow_length":false}},
              {{"operation_id":"ov3","type":"set_property","clip":"nest_b","prop":"opacity","value":0.5}}
            ]}}"#
        ),
    );
    ok_json(&["apply", s(&p), s(&overlay), "--json"]);
    let after = dump(&p);
    assert_ne!(before, after);
    ok_json(&["undo", s(&p), "--json"]);
    assert_eq!(dump(&p), before, "undo restores the document");
    ok_json(&["redo", s(&p), "--json"]);
    assert_eq!(dump(&p), after, "redo reapplies it");

    // 5. render: um quadro do BODY_MASTER == o mesmo quadro no export intermediário (manifesto)
    let f5 = ok_json(&[
        "render",
        "frame",
        s(&p),
        "--sequence",
        "BODY_MASTER",
        "--frame",
        "5",
        "--width",
        "64",
        "--height",
        "48",
        "--json",
    ]);
    let inter = d.file("inter");
    let rep = ok_json(&[
        "export",
        "intermediate",
        s(&p),
        "--sequence",
        "BODY_MASTER",
        "--out",
        s(&inter),
        "--width",
        "64",
        "--height",
        "48",
        "--json",
    ]);
    assert_eq!(rep["frames"], 90);
    assert_eq!(
        rep["frame_digests"][5], f5["digest"],
        "render and export agree"
    );
    assert!(inter.join("manifest.json").exists() && inter.join("audio.wav").exists());

    // 6. encoders detectados e a política
    let enc = ok_json(&["media", "encoders", "--json"]);
    let list = enc["encoders"].as_array().unwrap();
    for x in list {
        if x["ffmpeg_name"].as_str().unwrap().starts_with("libx26") {
            assert_eq!(x["available"], false, "GPL encoder must never be available");
        }
    }
    let h264_ok = list
        .iter()
        .any(|x| x["codec"] == "h264" && x["available"] == true);
    eprintln!("H.264 approved encoder available on this host: {h264_ok}");

    // 7. MP4(s)
    let mp4 = d.file("body_master.mp4");
    if h264_ok {
        let r = ok_json(&[
            "export",
            "mp4",
            s(&p),
            "--sequence",
            "BODY_MASTER",
            "--out",
            s(&mp4),
            "--width",
            "64",
            "--height",
            "48",
            "--json",
        ]);
        assert_eq!(r["codec"], "h264");
        check_mp4(&mp4, "h264", 90);
    } else {
        // sem encoder aprovado: erro estruturado, nada publicado, SEM fallback silencioso
        let o = capia(&[
            "export",
            "mp4",
            s(&p),
            "--sequence",
            "BODY_MASTER",
            "--out",
            s(&mp4),
            "--width",
            "64",
            "--height",
            "48",
            "--json",
        ]);
        assert_eq!(o.code, 1, "{}", o.stdout);
        let e: Value = serde_json::from_str(o.stderr.trim()).unwrap();
        assert_eq!(e["error"]["code"], "MEDIA_ENCODER_UNAVAILABLE", "{e}");
        assert!(!mp4.exists());
        // referência explícita (MPEG-4 parte 2, não H.264): prova mux, quadros e sincronismo
        let r = ok_json(&[
            "export",
            "mp4",
            s(&p),
            "--sequence",
            "BODY_MASTER",
            "--out",
            s(&mp4),
            "--width",
            "64",
            "--height",
            "48",
            "--codec",
            "mpeg4-reference",
            "--json",
        ]);
        assert_eq!(r["codec"], "mpeg4");
        check_mp4(&mp4, "mpeg4", 90);
    }
    // GPL por nome: recusado, nada publicado
    let gpl = d.file("gpl.mp4");
    let o = capia(&[
        "export",
        "mp4",
        s(&p),
        "--sequence",
        "BODY_MASTER",
        "--out",
        s(&gpl),
        "--encoder",
        "libx264",
        "--json",
    ]);
    assert_eq!(o.code, 1);
    assert!(
        o.stderr.contains("MEDIA_ENCODER_PROHIBITED"),
        "{}",
        o.stderr
    );
    assert!(!gpl.exists());
    // segundo MP4: o HOOK_B sozinho (1 s)
    let hb = d.file("hook_b_export.mp4");
    let codec = if h264_ok { "h264" } else { "mpeg4-reference" };
    ok_json(&[
        "export",
        "mp4",
        s(&p),
        "--sequence",
        "HOOK_B",
        "--out",
        s(&hb),
        "--width",
        "64",
        "--height",
        "48",
        "--codec",
        codec,
        "--json",
    ]);
    check_mp4(&hb, if h264_ok { "h264" } else { "mpeg4" }, 30);

    // 8. reabre (processos novos) e valida o projeto + os assets continuam online
    let v = ok_json(&["validate", s(&p), "--json"]);
    assert_eq!(v["ok"], true, "{v}");
    let hist = ok_json(&["history", s(&p), "--json"]);
    assert!(hist.to_string().contains("overlay hook B"));
}
