//! Kill REAL (SIGKILL no processo-filho estacionado no failpoint) e retomada em outro processo
//! (PHASE5_AUTONOMY_TESTS): o filho cria projeto+Run e estaciona (`CAPIA_FAILPOINT_MODE=park`); o
//! pai mata de fora, reabre o `.capia` e prova que `recover()`+`resume()` terminam a Run sem
//! duplicar edição nem download. O "filho" é este mesmo binário de teste (`kill_child`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::gateway::{
    Candidate, CatalogEntry, LicenseStatus, ReplayCatalogAdapter,
};
use capia_intelligence::autonomy::machine::RunStatus;
use capia_intelligence::autonomy::plan::AssetKind;
use common::auto::*;
use common::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn candidate(id: &str, license: LicenseStatus, price: Option<u64>) -> Candidate {
    Candidate {
        id: id.into(),
        adapter: "cat".into(),
        title: "pessoa usando cafe na cozinha".into(),
        description: "b-roll produto cozinha".into(),
        kind: AssetKind::Video,
        duration_ms: Some(4000),
        width: None,
        height: None,
        license,
        license_text: Some("test license".into()),
        price_micros: price,
        source_uri: Some(format!("https://stock.example/{id}")),
        score: 0.0,
        score_components: vec![],
    }
}

fn need(id: &str, required: bool, priority: &[&str]) -> Value {
    json!({"id": id, "kind": "video", "purpose": "b-roll produto", "description": "pessoa usando cafe na cozinha",
           "required": required, "source_priority": priority, "target_duration_ms": 4000})
}

fn beats_with_need(asset: &str, need_id: &str) -> Value {
    json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "estimated_duration_ms": 7000, "beats": [
        {"id": "hook", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": asset, "source_in_ms": 0},
         "overlays": [{"text": "Compre agora", "start_offset_ms": 0, "duration_ms": 1500}]},
        {"id": "broll", "role": "broll", "duration_ms": 3000, "asset": {"need_id": need_id, "source_in_ms": 0}},
        {"id": "cta", "role": "cta", "duration_ms": 1000, "asset": {"asset_id": asset, "source_in_ms": 7000}}]})
}

fn world_for(
    scenario: &str,
    reopen: Option<PathBuf>,
) -> Option<(AutoWorld, Option<Arc<ReplayCatalogAdapter>>)> {
    let gateway = scenario == "download";
    let a = auto_world_at("kill", reopen, |asset| {
        Script::new(
            || demand_json(false),
            move || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    if gateway {
                        json!([need("n1", true, &["gateway"])])
                    } else {
                        json!([])
                    },
                )
            },
            move |_d, _n| {
                if gateway {
                    beats_with_need(&asset.lock().unwrap(), "n1")
                } else {
                    edit_json(&asset.lock().unwrap(), json!([]))
                }
            },
            |_n| json!({"findings": []}),
        )
    })?;
    let cat = if gateway {
        let tc = ffmpeg()?;
        let payload = broll_bytes(&tc, &a.w.dir, "stock.mp4", 4);
        let c = Arc::new(ReplayCatalogAdapter::new(
            "cat",
            vec![CatalogEntry {
                candidate: candidate("ok", LicenseStatus::KnownAllowed, Some(0)),
                payload,
            }],
        ));
        a.gateways.register_shared(c.clone());
        Some(c)
    } else {
        None
    };
    Some((a, cat))
}

/// Processo-filho: só roda quando o pai o invoca (`CAPIA_KILL_CHILD`); senão é no-op.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kill_child() {
    let Ok(scenario) = std::env::var("CAPIA_KILL_CHILD") else {
        return;
    };
    let (a, _cat) = world_for(&scenario, None).expect("fixtures");
    let run = a.create(a.inputs(), auto_policy());
    println!("RUN {}", run.id);
    a.orch.start(&run.id).unwrap();
    // o failpoint (modo park) estaciona o driver; o pai mata este processo
    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
}

async fn kill_and_resume(scenario: &str, point: &str) {
    if ffmpeg().is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("capia-kill-{point}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "kill_child", "--nocapture", "--test-threads=1"])
        .env("CAPIA_KILL_CHILD", scenario)
        .env("CAPIA_TEST_DIR", &dir)
        .env("CAPIA_FAILPOINT", point)
        .env("CAPIA_FAILPOINT_MODE", "park")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let out = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let mut run_id = String::new();
    let t0 = std::time::Instant::now();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(l) if l.contains("RUN ") => {
                run_id = l[l.find("RUN ").unwrap() + 4..].trim().to_owned()
            }
            Ok(l) if l.contains("FAILPOINT_REACHED") => break,
            _ => {}
        }
        assert!(
            t0.elapsed().as_secs() < 120,
            "the child never reached {point}"
        );
    }
    child.kill().unwrap(); // SIGKILL: sem destrutores, sem flush
    child.wait().unwrap();
    assert!(!run_id.is_empty());

    // outro "processo": reabre o projeto
    let (a, cat) = world_for(scenario, Some(dir.clone())).unwrap();
    let rec = a.orch.recover().unwrap();
    assert!(rec.iter().any(|r| r.run_id == run_id), "{rec:?}");
    let r = a.orch.load(&run_id).unwrap();
    assert_eq!(r.status, RunStatus::Paused, "never auto-resumed");
    a.orch.resume(&run_id).unwrap();
    let done = a.wait(&run_id, |r| r.status == RunStatus::Completed).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    // nenhuma operação duplicada no histórico
    let h = a.w.ctx.engine.read("history.list", json!({})).unwrap();
    let ops: Vec<String> = h["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            e["actor"]["id"]
                .as_str()
                .is_some_and(|x| x.starts_with("run:"))
        })
        .map(|e| e["operation_id"].as_str().unwrap_or("").to_owned())
        .collect();
    let mut u = ops.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), ops.len(), "duplicated operations: {ops:?}");
    assert!(!ops.is_empty());
    if let Some(c) = cat {
        assert_eq!(
            c.fetch_count.load(Ordering::SeqCst),
            0,
            "the verified staged download was reused"
        );
    }
    let _: Value = Value::Null;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sigkill_while_the_provider_answer_is_unsaved_resumes_in_another_process() {
    kill_and_resume("plain", "autonomy_understand_after_provider").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sigkill_after_the_download_resumes_without_downloading_again() {
    kill_and_resume("download", "autonomy_acquire_after_download").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sigkill_right_after_the_apply_resumes_without_applying_twice() {
    kill_and_resume("plain", "autonomy_edit_after_apply").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sigkill_right_before_the_apply_resumes_and_applies_once() {
    kill_and_resume("plain", "autonomy_edit_before_apply").await;
}
