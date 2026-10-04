//! A API do editor de ponta a ponta (JSON in/out), com projeto real e FFmpeg real (quando houver).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_editor_api::{Reply, Session, SessionConfig};
use capia_media::{MediaConfig, MediaToolchain};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn media_ok() -> bool {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => true,
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "CAPIA_REQUIRE_FFMPEG is set but ffmpeg is unavailable: {other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
            false
        }
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("capia-api-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn json_of(r: Reply) -> Value {
    match r {
        Reply::Json(v) => v,
        Reply::Binary { .. } => panic!("expected JSON"),
    }
}

fn call(s: &mut Session, m: &str, p: Value) -> Value {
    json_of(s.call(m, p).unwrap_or_else(|e| panic!("{m}: {e}")))
}

fn setup(name: &str) -> (Session, PathBuf) {
    let dir = tmp(name);
    let mut s = Session::new(SessionConfig::default());
    call(
        &mut s,
        "project.create",
        json!({ "path": dir.join("p.capia").display().to_string() }),
    );
    (s, dir)
}

fn cmds(s: &mut Session, label: &str, commands: Value) -> Value {
    call(
        s,
        "command.execute",
        json!({ "label": label, "commands": commands }),
    )
}

const FRAME: i64 = 23_520_000;

#[test]
fn edits_produce_patches_undo_redo_and_history() {
    let (mut s, _d) = setup("edit");
    let r = cmds(
        &mut s,
        "setup",
        json!([
            {"operation_id":"o1","type":"create_sequence","id":"s","name":"Main","frame_rate":"30","width":1080,"height":1920},
            {"operation_id":"o2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
            {"operation_id":"o3","type":"insert_clip","track":"v","start":0,
             "clip":{"id":"c1","duration":FRAME*30,"content":{"type":"solid","color":"#336699"}}}
        ]),
    );
    assert_eq!(r["can_undo"], true);
    assert_eq!(r["can_redo"], false);
    let patches = r["patches"].as_array().unwrap();
    assert!(patches.iter().any(|p| p["op"] == "clip" && p["id"] == "c1"));
    assert!(
        patches
            .iter()
            .any(|p| p["op"] == "sequence" && p["id"] == "s")
    );
    let summ = &r["sequence_summaries"][0];
    assert_eq!(
        (summ["width"].as_i64(), summ["height"].as_i64()),
        (Some(1080), Some(1920))
    );

    let snap = call(&mut s, "project.snapshot", json!({}));
    assert_eq!(snap["sequences"][0]["clip_count"], 1);
    let seq = call(&mut s, "sequence.get", json!({"sequence":"s"}));
    assert!(seq["clips"]["c1"].is_object());

    // split → 2 clips; undo restaura; redo reaplica
    cmds(
        &mut s,
        "split",
        json!([{"operation_id":"o4","type":"split_clip","clip":"c1","at":FRAME*10}]),
    );
    assert_eq!(
        call(&mut s, "sequence.get", json!({"sequence":"s"}))["clips"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    let u = call(&mut s, "command.undo", json!({}));
    assert_eq!(u["can_redo"], true);
    // o undo devolve patches inversos (new = estado anterior)
    assert!(
        u["patches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["op"] == "clip" && p["new"].is_null())
    );
    assert_eq!(
        call(&mut s, "sequence.get", json!({"sequence":"s"}))["clips"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    call(&mut s, "command.redo", json!({}));
    assert_eq!(
        call(&mut s, "sequence.get", json!({"sequence":"s"}))["clips"]
            .as_object()
            .unwrap()
            .len(),
        2
    );

    let h = call(&mut s, "history.list", json!({}));
    assert_eq!(h["entries"].as_array().unwrap().len(), 2);
    assert_eq!(h["cursor"], 2);
    assert_eq!(h["entries"][1]["label"], "split");
}

#[test]
fn errors_are_structured_and_do_not_change_the_document() {
    let (mut s, _d) = setup("errors");
    cmds(
        &mut s,
        "setup",
        json!([
            {"operation_id":"a1","type":"create_sequence","id":"s","name":"M","frame_rate":"30"},
            {"operation_id":"a2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
            {"operation_id":"a3","type":"insert_clip","track":"v","start":0,
             "clip":{"id":"c1","duration":FRAME*30,"content":{"type":"solid","color":"#fff"}}}
        ]),
    );
    let err = s
        .call("command.execute", json!({"commands":[{"operation_id":"b1","type":"insert_clip","track":"v","start":FRAME*10,
            "clip":{"id":"c2","duration":FRAME*30,"content":{"type":"solid","color":"#fff"}}}]}))
        .unwrap_err();
    assert_eq!(err.code, "OVERLAP");
    assert!(err.details.as_ref().unwrap()["hint"]["free_ranges"].is_array());
    let rev = call(&mut s, "project.snapshot", json!({}))["revision"].clone();
    let _ = s.call("command.undo", json!({})).unwrap();
    assert_ne!(call(&mut s, "project.snapshot", json!({}))["revision"], rev);
    // sem projeto
    s.call("project.close", json!({})).unwrap();
    assert_eq!(
        s.call("project.snapshot", json!({})).unwrap_err().code,
        "NO_PROJECT_OPEN"
    );
    assert_eq!(
        s.call("nope", json!({})).unwrap_err().code,
        "UNKNOWN_METHOD"
    );
    assert!(
        !s.call(
            "project.open",
            json!({"path": "/definitely/not/here.capia"})
        )
        .unwrap_err()
        .code
        .is_empty()
    );
}

fn poll_until(s: &mut Session, what: &str, mut pred: impl FnMut(&Value) -> bool) -> Vec<Value> {
    let start = Instant::now();
    let mut all = Vec::new();
    while start.elapsed() < Duration::from_secs(60) {
        let ev = call(s, "events.poll", json!({}));
        for e in ev["events"].as_array().unwrap() {
            all.push(e.clone());
        }
        if all.iter().any(&mut pred) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timeout waiting for {what}: {all:?}");
}

#[test]
fn import_render_thumbnail_and_export_work_end_to_end() {
    if !media_ok() {
        return;
    }
    let (mut s, d) = setup("media");
    let info = call(&mut s, "engine.info", json!({}));
    assert_eq!(info["media_available"], true);

    // import em background → evento + patch do documento
    let imp = call(
        &mut s,
        "assets.import",
        json!({"paths":[fixture("video_audio.mp4").display().to_string()]}),
    );
    assert_eq!(imp["tickets"].as_array().unwrap().len(), 1, "{imp}");
    let evs = poll_until(&mut s, "import", |e| e["kind"] == "document_changed");
    let fin = evs
        .iter()
        .find(|e| e["kind"] == "import_finalized")
        .expect("import_finalized");
    let asset_id = fin["result"]["asset_id"].as_str().unwrap().to_owned();
    let changed = evs
        .iter()
        .find(|e| e["kind"] == "document_changed")
        .unwrap();
    assert!(
        changed["change"]["patches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["op"] == "asset")
    );
    let assets = call(&mut s, "assets.list", json!({}));
    assert_eq!(assets.as_array().unwrap().len(), 1);

    // montar sequence com a mídia e renderizar um quadro
    cmds(
        &mut s,
        "build",
        json!([
            {"operation_id":"m1","type":"create_sequence","id":"s","name":"M","frame_rate":"30","width":320,"height":180},
            {"operation_id":"m2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
            {"operation_id":"m3","type":"add_track","sequence":"s","id":"t","kind":"visual"},
            {"operation_id":"m4","type":"insert_clip","track":"v","start":0,
             "clip":{"id":"c1","duration":FRAME*30,"content":{"type":"media","asset":asset_id,"has_video":true,"has_audio":true}}},
            {"operation_id":"m5","type":"insert_clip","track":"t","start":0,
             "clip":{"id":"txt","duration":FRAME*30,"content":{"type":"text","text":"Olá CapIA",
                     "style":{"size_permille":120,"weight":700}}}}
        ]),
    );
    let Reply::Binary { mime, bytes, meta } = s
        .call(
            "render.frame",
            json!({"sequence":"s","at":FRAME*5,"width":320,"height":180}),
        )
        .unwrap()
    else {
        panic!("expected a binary frame");
    };
    assert_eq!(mime, "application/x-rgba");
    assert_eq!(bytes.len(), 320 * 180 * 4);
    assert_eq!(meta["width"], 320);
    let white = bytes
        .chunks(4)
        .filter(|p| p[0] > 250 && p[1] > 250 && p[2] > 250)
        .count();
    assert!(
        white > 20,
        "white text painted ({white} px); warnings: {}",
        meta["warnings"]
    );

    let Reply::Binary { mime, bytes, .. } = s
        .call("media.thumbnail", json!({"asset": asset_id, "max_dim": 96}))
        .unwrap()
    else {
        panic!("expected a thumbnail");
    };
    assert!(mime == "image/jpeg" || mime == "image/png", "{mime}");
    assert!(bytes.len() > 100);

    let wf = call(
        &mut s,
        "media.peaks",
        json!({"asset": asset_id, "buckets": 32}),
    );
    let n = wf["peaks"].as_array().unwrap().len();
    assert!(
        n > 0 && n.is_multiple_of(2) && n <= 64,
        "{n} values (min,max pairs)"
    );

    // export H.264 (se houver encoder aprovado) ou intermediário (sempre)
    let out = d.join("out.rgba-dir");
    let started = call(
        &mut s,
        "export.start",
        json!({"items":[
            {"id":"i1","sequence":"s","preset":"intermediate","path":out.display().to_string()}
        ]}),
    );
    assert!(started["batch"].as_str().unwrap().starts_with("batch-"));
    let evs = poll_until(&mut s, "export", |e| e["kind"] == "export_batch_finished");
    let fin = evs
        .iter()
        .find(|e| e["kind"] == "export_item_finished")
        .unwrap();
    assert_eq!(fin["ok"], true, "{fin}");
    assert!(out.exists());
}

#[test]
fn export_can_be_cancelled_and_never_publishes_a_partial_output() {
    if !media_ok() {
        return;
    }
    let (mut s, d) = setup("cancel");
    cmds(
        &mut s,
        "build",
        json!([
            {"operation_id":"x1","type":"create_sequence","id":"s","name":"M","frame_rate":"30","width":640,"height":360},
            {"operation_id":"x2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
            {"operation_id":"x3","type":"insert_clip","track":"v","start":0,
             "clip":{"id":"c1","duration":FRAME*3000,"content":{"type":"solid","color":"#336699"}}}
        ]),
    );
    let out = d.join("long.out");
    let r = call(
        &mut s,
        "export.start",
        json!({"items":[
            {"id":"i1","sequence":"s","preset":"intermediate","path":out.display().to_string()}
        ]}),
    );
    let batch = r["batch"].as_str().unwrap().to_owned();
    std::thread::sleep(Duration::from_millis(300));
    call(&mut s, "export.cancel", json!({"id": batch}));
    let evs = poll_until(&mut s, "cancel", |e| e["kind"] == "export_batch_finished");
    let fin = evs
        .iter()
        .find(|e| e["kind"] == "export_item_finished")
        .unwrap();
    assert_eq!(fin["ok"], false);
    assert_eq!(fin["cancelled"], true);
    assert!(!out.exists(), "no partial output may be published");
}
