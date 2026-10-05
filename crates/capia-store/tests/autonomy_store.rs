//! Schema 5 (autonomia): cursor atômico com CAS, efeitos idempotentes, livro-razão de orçamento
//! sem corrida, recuperação de stages interrompidos, migração a partir do schema 4.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_store::{AutonomyStore, Claim, ProjectStore, RunUpdate, StageRow, StoreErrorCode};
use common::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const BUSY: Duration = Duration::from_millis(5_000);

fn project(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.file("p.capia");
    let (store, _s) = ProjectStore::create(&path, &fast()).unwrap();
    store.close().unwrap();
    path
}

fn stage(run: &str, seq: u64, name: &str, key: &str, status: &str) -> StageRow {
    StageRow {
        run_id: run.into(),
        seq,
        stage: name.into(),
        attempt: 0,
        idem_key: key.into(),
        input_digest: "in".into(),
        output_digest: (status == "completed").then(|| "out".to_owned()),
        status: status.into(),
        started_ms: 1,
        ended_ms: (status != "started").then_some(2),
        json: json!({}),
    }
}

#[test]
fn the_cursor_moves_only_with_the_expected_revision_and_atomically() {
    let dir = TempDir::new("auto-cas");
    let s = AutonomyStore::open(&project(&dir), BUSY).unwrap();
    s.create_run("r1", "pending", "understand", None, None, &json!({"a": 1}), 1)
        .unwrap();
    assert!(s.create_run("r1", "pending", "understand", None, None, &json!({}), 1).is_err());
    let upd = RunUpdate {
        status: "running".into(),
        stage: "plan".into(),
        json: json!({"a": 2}),
    };
    let rev = s
        .advance(
            "r1",
            0,
            &upd,
            Some(&stage("r1", 1, "understand", "r1:understand:0", "completed")),
            &[("stage_completed".into(), json!({"stage": "understand"}))],
            5,
        )
        .unwrap();
    assert_eq!(rev, 1);
    // um segundo escritor com revisão velha perde — e nada dele é gravado
    let err = s
        .advance(
            "r1",
            0,
            &RunUpdate {
                status: "failed".into(),
                stage: "plan".into(),
                json: json!({"a": 3}),
            },
            Some(&stage("r1", 2, "plan", "r1:plan:0", "completed")),
            &[("x".into(), json!({}))],
            6,
        )
        .unwrap_err();
    assert_eq!(err.code, StoreErrorCode::StoreConflict);
    let run = s.get_run("r1").unwrap().unwrap();
    assert_eq!((run.status.as_str(), run.stage.as_str(), run.revision), ("running", "plan", 1));
    assert_eq!(s.list_stages("r1").unwrap().len(), 1, "no partial stage row");
    assert_eq!(s.events_after("r1", 0, 100).unwrap().len(), 1, "no partial event");
    assert_eq!(s.stage_by_key("r1:understand:0").unwrap().unwrap().seq, 1);
    assert_eq!(s.next_stage_seq("r1").unwrap(), 2);
}

#[test]
fn started_stages_become_interrupted_never_completed() {
    let dir = TempDir::new("auto-int");
    let s = AutonomyStore::open(&project(&dir), BUSY).unwrap();
    s.create_run("r1", "running", "edit", None, None, &json!({}), 1).unwrap();
    s.put_stage(&stage("r1", 1, "plan", "k1", "completed")).unwrap();
    s.put_stage(&stage("r1", 2, "edit", "k2", "started")).unwrap();
    assert_eq!(s.interrupt_started_stages(9).unwrap(), 1);
    let st = s.list_stages("r1").unwrap();
    assert_eq!(st[0].status, "completed");
    assert_eq!(st[1].status, "interrupted");
    assert_eq!(s.interrupt_started_stages(10).unwrap(), 0, "idempotent");
}

#[test]
fn only_the_first_claim_of_an_effect_executes() {
    let dir = TempDir::new("auto-eff");
    let s = Arc::new(AutonomyStore::open(&project(&dir), BUSY).unwrap());
    let first = s.claim_effect("gen:abc", "r1", "generation", &json!({"p": 1}), 1).unwrap();
    assert!(matches!(first, Claim::New(_)));
    s.update_effect("gen:abc", "submitted", Some("job-7"), None, 2).unwrap();
    match s.claim_effect("gen:abc", "r1", "generation", &json!({"p": 2}), 3).unwrap() {
        Claim::Existing(e) => {
            assert_eq!(e.state, "submitted");
            assert_eq!(e.external_id.as_deref(), Some("job-7"));
            assert_eq!(e.json["p"], 1, "the original intent is kept");
        }
        Claim::New(_) => panic!("duplicate claim must not be new"),
    }
    // corrida real: 8 threads, exatamente uma vence
    let wins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hs: Vec<_> = (0..8)
        .map(|_| {
            let (s, w) = (s.clone(), wins.clone());
            std::thread::spawn(move || {
                if let Claim::New(_) = s.claim_effect("race", "r1", "x", &json!({}), 1).unwrap() {
                    w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(wins.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(s.effects_for_run("r1").unwrap().len(), 2);
}

#[test]
fn parallel_reservations_never_exceed_the_limit() {
    let dir = TempDir::new("auto-led");
    let s = Arc::new(AutonomyStore::open(&project(&dir), BUSY).unwrap());
    let ok = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hs: Vec<_> = (0..12)
        .map(|n| {
            let (s, ok) = (s.clone(), ok.clone());
            std::thread::spawn(move || {
                if s
                    .ledger_reserve("r1", &format!("res{n}"), 300, Some(1_000), &json!({}), 1)
                    .unwrap()
                {
                    ok.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(ok.load(std::sync::atomic::Ordering::SeqCst), 3, "3 x 300 <= 1000 < 4 x 300");
    assert_eq!(s.ledger_committed("r1").unwrap(), 900);
    // idempotente: repetir a mesma reserva não gasta de novo
    assert!(s.ledger_reserve("r1", "res0", 300, Some(1_000), &json!({}), 2).unwrap() || true);
    assert_eq!(s.ledger_committed("r1").unwrap(), 900);
}

#[test]
fn settle_and_release_reconcile_reserved_with_actual() {
    let dir = TempDir::new("auto-set");
    let s = AutonomyStore::open(&project(&dir), BUSY).unwrap();
    assert!(s.ledger_reserve("r1", "a", 500, Some(1_000), &json!({}), 1).unwrap());
    assert!(s.ledger_reserve("r1", "b", 400, Some(1_000), &json!({}), 1).unwrap());
    assert!(!s.ledger_reserve("r1", "c", 200, Some(1_000), &json!({}), 1).unwrap());
    s.ledger_settle("r1", "a", 120, &json!({}), 2).unwrap();
    s.ledger_settle("r1", "a", 999, &json!({}), 3).unwrap(); // idempotente: o primeiro vale
    s.ledger_release("r1", "b", 4).unwrap();
    assert_eq!(s.ledger_committed("r1").unwrap(), 120);
    assert!(s.ledger_reserve("r1", "c", 200, Some(1_000), &json!({}), 5).unwrap());
    assert_eq!(s.ledger_committed("r1").unwrap(), 320);
}

#[test]
fn provenance_memory_and_events_round_trip_and_redact_registered_secrets() {
    let dir = TempDir::new("auto-misc");
    let path = project(&dir);
    let s = AutonomyStore::open(&path, BUSY).unwrap();
    capia_secrets::register_global("sk-CANARY-autonomy-store-0001");
    s.put_provenance(
        "asset1",
        Some("r1"),
        "generated",
        "sha256:aa",
        &json!({"note": "key sk-CANARY-autonomy-store-0001 leaked?"}),
        1,
    )
    .unwrap();
    let p = s.get_provenance("asset1").unwrap().unwrap();
    assert!(!p.json.to_string().contains("sk-CANARY-autonomy-store-0001"));
    assert_eq!(s.provenance_by_hash("sha256:aa").unwrap().unwrap().asset_id, "asset1");
    s.put_memory("m1", "proposed", &json!({"content": "x"}), 1).unwrap();
    s.log_memory("m1", "proposed", "agent", Some("r1"), &json!({}), 1).unwrap();
    assert_eq!(s.list_memory(Some("proposed")).unwrap().len(), 1);
    assert_eq!(s.list_memory(Some("active")).unwrap().len(), 0);
    assert_eq!(s.memory_log(Some("m1")).unwrap().len(), 1);
    s.create_run("r1", "running", "plan", None, Some("g1"), &json!({}), 1).unwrap();
    for n in 0..3 {
        s.append_event("r1", "tick", &json!({"n": n}), 1).unwrap();
    }
    let ev = s.events_after("r1", 1, 10).unwrap();
    assert_eq!(ev.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
    assert_eq!(s.runs_in_group("g1").unwrap().len(), 1);
    // nada do segredo no arquivo inteiro
    drop(s);
    let bytes = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("sk-CANARY-autonomy-store-0001"));
}

#[test]
fn a_schema_4_project_migrates_and_keeps_its_content() {
    let dir = TempDir::new("auto-mig4");
    let path = dir.file("old.capia");
    let (store, _s) = ProjectStore::create_at_schema(&path, &fast(), 4).unwrap();
    assert_eq!(store.schema_version(), 4);
    store.close().unwrap();
    let (store, _) = ProjectStore::open(&path, &fast()).unwrap();
    assert_eq!(store.schema_version(), 5);
    store.close().unwrap();
    let s = AutonomyStore::open(&path, BUSY).unwrap();
    assert!(s.list_runs(None, 10).unwrap().is_empty());
}
