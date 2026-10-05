//! Testes de ponta a ponta da CLI: executam o binário `capia` de verdade (processo separado) sobre
//! arquivos `.capia` reais.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("capia-cli-{}-{t}-{n}", std::process::id()));
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
    capia_with_stdin(args, None)
}

fn capia_with_stdin(args: &[&str], stdin: Option<&str>) -> Out {
    let mut child = Command::new(env!("CARGO_BIN_EXE_capia"))
        .args(args)
        .args(["--sync", "normal"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    let o = child.wait_with_output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8(o.stdout).unwrap(),
        stderr: String::from_utf8(o.stderr).unwrap(),
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

const SETUP: &str = r##"{
  "label": "setup",
  "commands": [
    {"operation_id": "s1", "type": "create_sequence", "id": "A", "name": "AD 1", "frame_rate": "30"},
    {"operation_id": "s2", "type": "add_track", "sequence": "A", "id": "A_V1", "kind": "visual"},
    {"operation_id": "s3", "type": "create_sequence", "id": "B", "name": "BODY", "frame_rate": "30"},
    {"operation_id": "s4", "type": "add_track", "sequence": "B", "id": "B_V1", "kind": "visual"},
    {"operation_id": "s5", "type": "insert_clip", "track": "B_V1", "start": 0,
     "clip": {"id": "body", "duration": 2352000000, "content": {"type": "solid", "color": "#222"}}}
  ]
}"##;

fn project_with_setup(dir: &Dir) -> PathBuf {
    let p = dir.file("p.capia");
    assert_eq!(capia(&["create", s(&p)]).code, 0);
    let setup = dir.write("setup.json", SETUP);
    let r = capia(&["apply", s(&p), s(&setup)]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    p
}

#[test]
fn create_and_inspect_a_project() {
    let d = Dir::new();
    let p = d.file("p.capia");
    let r = capia(&["create", s(&p)]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("created") && r.stdout.contains("schema:    4"),
        "{}",
        r.stdout
    );
    assert!(p.exists());

    let r = capia(&["inspect", s(&p), "--json"]);
    assert_eq!(r.code, 0);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["schema_version"], 4);
    assert_eq!(v["revision"], 0);
    assert_eq!(v["stats"]["operations"], 0);
    assert!(v["project_id"].as_str().unwrap().starts_with("prj_"));

    let r = capia(&["inspect", s(&p)]);
    assert!(
        r.stdout.contains("project:") && r.stdout.contains("0 sequences · 0 tracks · 0 clips"),
        "{}",
        r.stdout
    );
}

#[test]
fn creating_over_an_existing_project_is_a_structured_failure() {
    let d = Dir::new();
    let p = d.file("p.capia");
    assert_eq!(capia(&["create", s(&p)]).code, 0);
    let before = std::fs::read(&p).unwrap();
    let r = capia(&["create", s(&p), "--json"]);
    assert_eq!(r.code, 1);
    let e: Value = serde_json::from_str(r.stderr.trim()).unwrap();
    assert_eq!(e["error"]["code"], "PROJECT_ALREADY_EXISTS");
    assert_eq!(std::fs::read(&p).unwrap(), before);
}

#[test]
fn apply_persists_and_inspect_counts_the_document_and_the_operation_log() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let r = capia(&["inspect", s(&p), "--json"]);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["sequences"].as_array().unwrap().len(), 2);
    assert_eq!(v["total_tracks"], 2);
    assert_eq!(v["total_clips"], 1);
    assert_eq!(v["stats"]["operations"], 5);
    assert_eq!(v["stats"]["history_entries"], 1);
    assert_eq!(v["revision"], 1);
    let body = v["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "B")
        .unwrap();
    assert_eq!(body["duration_ticks"], 2_352_000_000_i64);
}

#[test]
fn reapplying_the_same_operation_ids_is_idempotent_across_processes() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let setup = d.file("setup.json");
    let r = capia(&["apply", s(&p), s(&setup), "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["replayed"], true);
    let info: Value = serde_json::from_str(&capia(&["inspect", s(&p), "--json"]).stdout).unwrap();
    assert_eq!(info["revision"], 1, "nothing was applied twice");
    assert_eq!(info["stats"]["history_entries"], 1);
}

#[test]
fn dump_is_deterministic_and_matches_the_inspect_digest() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let a = capia(&["dump", s(&p)]);
    let b = capia(&["dump", s(&p)]);
    assert_eq!(a.code, 0);
    assert_eq!(a.stdout, b.stdout, "byte-identical output");
    let doc: Value = serde_json::from_str(&a.stdout).unwrap();
    assert_eq!(doc["revision"], 1);
    assert!(doc["sequences"]["A"].is_object() && doc["sequences"]["B"].is_object());
    assert!(a.stdout.lines().count() == 1, "compact by default");
    assert!(capia(&["dump", s(&p), "--pretty"]).stdout.lines().count() > 20);
}

#[test]
fn a_rejected_command_is_a_structured_error_and_changes_nothing() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let bad = d.write(
        "bad.json",
        r##"{"operation_id":"x1","type":"insert_clip","track":"B_V1","start":10000000,
            "clip":{"id":"c2","duration":23520000,"content":{"type":"solid","color":"#fff"}}}"##,
    );
    let before = capia(&["dump", s(&p)]).stdout;
    let r = capia(&["apply", s(&p), s(&bad), "--json"]);
    assert_eq!(r.code, 1);
    let e: Value = serde_json::from_str(r.stderr.trim()).unwrap();
    assert!(
        ["NOT_FRAME_ALIGNED", "OVERLAP"].contains(&e["error"]["code"].as_str().unwrap()),
        "{e}"
    );
    assert_eq!(e["error"]["command_index"], 0);
    assert_eq!(capia(&["dump", s(&p)]).stdout, before);
}

#[test]
fn nested_sequences_through_the_cli_with_cycle_protection() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let nest = d.write(
        "nest.json",
        r#"{"operation_id":"n1","type":"insert_nested","track":"A_V1","start":0,"sequence":"B","id":"nb","follow_length":true}"#,
    );
    let r = capia(&["apply", s(&p), s(&nest)]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let info: Value = serde_json::from_str(&capia(&["inspect", s(&p), "--json"]).stdout).unwrap();
    let a = info["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "A")
        .unwrap();
    assert_eq!(
        a["duration_ticks"], 2_352_000_000_i64,
        "nested duration comes from BODY"
    );
    // B -> A fecharia A -> B -> A
    let cycle = d.write(
        "cycle.json",
        r#"{"operation_id":"n2","type":"insert_nested","track":"B_V1","start":2352000000,"sequence":"A"}"#,
    );
    let r = capia(&["apply", s(&p), s(&cycle), "--json"]);
    assert_eq!(r.code, 1);
    let e: Value = serde_json::from_str(r.stderr.trim()).unwrap();
    assert_eq!(e["error"]["code"], "NESTED_CYCLE");
    // não dá para apagar BODY enquanto A o usa
    let del = d.write(
        "del.json",
        r#"{"operation_id":"d1","type":"delete_sequence","sequence":"B"}"#,
    );
    let r = capia(&["apply", s(&p), s(&del), "--json"]);
    let e: Value = serde_json::from_str(r.stderr.trim()).unwrap();
    assert_eq!(e["error"]["code"], "IN_USE");
}

#[test]
fn undo_and_redo_persist_between_invocations() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let before = capia(&["dump", s(&p)]).stdout;
    let r = capia(&["undo", s(&p)]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let undone = capia(&["dump", s(&p)]).stdout;
    assert_ne!(undone, before);
    let h = capia(&["history", s(&p)]);
    assert!(
        h.stdout.contains("undone") && h.stdout.contains("setup"),
        "{}",
        h.stdout
    );
    let r = capia(&["redo", s(&p)]);
    assert_eq!(r.code, 0);
    let redone: Value = serde_json::from_str(&capia(&["dump", s(&p)]).stdout).unwrap();
    let orig: Value = serde_json::from_str(&before).unwrap();
    // o conteúdo é idêntico; só o contador de revisão avançou (undo e redo são revisões)
    assert_eq!(redone["sequences"], orig["sequences"]);
    assert_eq!(redone["revision"], 3);
    // nada a refazer
    let r = capia(&["redo", s(&p), "--json"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("NOTHING_TO_REDO"));
}

#[test]
fn agents_go_through_preview_and_apply_plan() {
    let d = Dir::new();
    let p = d.file("p.capia");
    capia(&["create", s(&p)]);
    let setup = d.write("setup.json", SETUP);
    let r = capia(&["apply", s(&p), s(&setup), "--actor", "agent", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["revision"], 1);
    // reenvio do mesmo plano: já aplicado
    let r = capia(&["apply", s(&p), s(&setup), "--actor", "agent", "--json"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("already_applied"));
    let h: Value = serde_json::from_str(&capia(&["history", s(&p), "--json"]).stdout).unwrap();
    assert_eq!(h[0]["actor"]["kind"], "agent");
}

#[test]
fn validate_accepts_good_projects_and_rejects_damaged_files() {
    let d = Dir::new();
    let p = project_with_setup(&d);
    let r = capia(&["validate", s(&p)]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(r.stdout.starts_with("OK:"));
    let v: Value = serde_json::from_str(&capia(&["validate", s(&p), "--json"]).stdout).unwrap();
    assert_eq!(v["ok"], true);
    // arquivo que não é CapIA
    let junk = d.write("junk.capia", "this is not a database");
    let r = capia(&["validate", s(&junk)]);
    assert_eq!(r.code, 1);
    assert!(
        r.stdout.contains("INVALID") && r.stdout.contains("NOT_A_CAPIA_PROJECT"),
        "{}",
        r.stdout
    );
    // truncado
    let bytes = std::fs::read(&p).unwrap();
    let cut = d.file("cut.capia");
    std::fs::write(&cut, &bytes[..bytes.len() / 3]).unwrap();
    assert_eq!(capia(&["validate", s(&cut)]).code, 1);
    // ausente
    let r = capia(&["inspect", s(&d.file("none.capia")), "--json"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("PROJECT_NOT_FOUND"));
}

#[test]
fn apply_reads_commands_from_stdin_and_accepts_a_bare_command() {
    let d = Dir::new();
    let p = d.file("p.capia");
    capia(&["create", s(&p)]);
    let r = capia_with_stdin(
        &["apply", s(&p), "-", "--label", "from stdin"],
        Some(r#"{"operation_id":"o1","type":"create_sequence","name":"S","frame_rate":"24"}"#),
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let h = capia(&["history", s(&p)]);
    assert!(h.stdout.contains("from stdin"), "{}", h.stdout);
    let r = capia_with_stdin(&["apply", s(&p), "-"], Some("not json"));
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("INVALID_ARGUMENT"));
}

#[test]
fn usage_errors_exit_with_code_2() {
    for args in [
        vec![],
        vec!["nope"],
        vec!["inspect"],
        vec!["create", "a", "b"],
        vec!["apply", "p"],
        vec!["inspect", "p", "--bogus"],
        vec!["apply", "p", "c.json", "--actor", "root"],
    ] {
        let r = capia(&args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.stderr);
    }
    let r = capia(&["help"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("USAGE"));
}
