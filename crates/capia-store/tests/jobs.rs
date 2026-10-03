//! Persistência de jobs e tickets (schema 3): round-trip, recuperação e cancelamento entre
//! processos.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_jobs::{JobError, JobId, JobKind, JobSink, JobSnapshot, JobState, Priority, Progress};
use capia_store::{JobStore, ProjectStore, TicketRow, TicketState};
use common::*;
use serde_json::json;
use std::time::Duration;

fn open(dir: &TempDir) -> (std::path::PathBuf, JobStore) {
    let path = dir.file("p.capia");
    let (store, _state) = ProjectStore::create(&path, &fast()).unwrap();
    drop(store);
    let js = JobStore::open(&path, Duration::from_secs(3)).unwrap();
    (path, js)
}

fn snap(id: &str, state: JobState) -> JobSnapshot {
    JobSnapshot {
        id: JobId(id.into()),
        kind: JobKind::FrameIndex,
        priority: Priority::Normal,
        state,
        progress: Progress { done: 5, total: 10 },
        dedup_key: Some("k".into()),
        label: "index".into(),
        params: json!({"asset": "ast_1"}),
        result: None,
        error: None,
        created_ms: 100,
        started_ms: Some(110),
        finished_ms: None,
    }
}

#[test]
fn snapshots_round_trip_through_the_sink() {
    let dir = TempDir::new("jobs-rt");
    let (_p, js) = open(&dir);
    let mut s = snap("job_1", JobState::Running);
    js.record(&s);
    assert_eq!(js.get(&s.id).unwrap().unwrap(), s);
    s.state = JobState::Failed;
    s.error = Some(JobError::new("MEDIA_DECODE_FAILED", "boom"));
    s.finished_ms = Some(200);
    js.record(&s);
    assert_eq!(js.get(&s.id).unwrap().unwrap(), s);
    s.state = JobState::Completed;
    s.error = None;
    s.result = Some(json!({"path": "x"}));
    js.record(&s);
    let back = js.get(&s.id).unwrap().unwrap();
    assert_eq!(back.result, Some(json!({"path": "x"})));
    assert_eq!(js.list(None, 10).unwrap().len(), 1);
    assert_eq!(js.list(Some(JobState::Failed), 10).unwrap().len(), 0);
    assert!(js.get(&JobId("nope".into())).unwrap().is_none());
}

#[test]
fn recovery_turns_running_and_queued_into_interrupted_never_completed() {
    let dir = TempDir::new("jobs-rec");
    let (path, js) = open(&dir);
    for (id, st) in [
        ("q", JobState::Queued),
        ("r", JobState::Running),
        ("c", JobState::Completed),
        ("f", JobState::Failed),
        ("x", JobState::Cancelled),
    ] {
        js.record(&snap(id, st));
    }
    js.put_ticket(&TicketRow {
        ticket_id: "t1".into(),
        path: "/a/b.mp4".into(),
        size_bytes: 9,
        fingerprint: Some("fp1:9:aa".into()),
        state: TicketState::Pending,
        job_id: Some("r".into()),
        asset_id: None,
        outcome: None,
        error: None,
        created_ms: 1,
        updated_ms: 1,
    })
    .unwrap();
    drop(js);
    // "reabrir o projeto"
    let js = JobStore::open(&path, Duration::from_secs(3)).unwrap();
    let r = js.recover(500).unwrap();
    assert_eq!((r.jobs_interrupted, r.tickets_interrupted), (2, 1));
    let state = |id: &str| js.get(&JobId(id.into())).unwrap().unwrap().state;
    assert_eq!(state("q"), JobState::Interrupted);
    assert_eq!(state("r"), JobState::Interrupted);
    assert_eq!(state("c"), JobState::Completed);
    assert_eq!(state("f"), JobState::Failed);
    assert_eq!(state("x"), JobState::Cancelled);
    assert_eq!(
        js.get_ticket("t1").unwrap().unwrap().state,
        TicketState::Interrupted
    );
    // idempotente
    let again = js.recover(600).unwrap();
    assert_eq!((again.jobs_interrupted, again.tickets_interrupted), (0, 0));
}

#[test]
fn cancel_requests_work_across_connections_and_only_for_live_jobs() {
    let dir = TempDir::new("jobs-cancel");
    let (path, js) = open(&dir);
    js.record(&snap("live", JobState::Running));
    js.record(&snap("done", JobState::Completed));
    let other = JobStore::open(&path, Duration::from_secs(3)).unwrap();
    assert!(other.request_cancel(&JobId("live".into())).unwrap());
    assert!(!other.request_cancel(&JobId("done".into())).unwrap());
    assert!(!other.request_cancel(&JobId("missing".into())).unwrap());
    assert_eq!(js.pending_cancels().unwrap(), vec![JobId("live".into())]);
    // a flag sobrevive a novas gravações de estado do executor
    js.record(&snap("live", JobState::Running));
    assert_eq!(js.pending_cancels().unwrap().len(), 1);
    let mut fin = snap("live", JobState::Cancelled);
    fin.finished_ms = Some(300);
    js.record(&fin);
    assert!(js.pending_cancels().unwrap().is_empty());
}

#[test]
fn tickets_round_trip_and_filter_by_state() {
    let dir = TempDir::new("jobs-tickets");
    let (_p, js) = open(&dir);
    let mut t = TicketRow {
        ticket_id: "tk_1".into(),
        path: "/m/a.mp4".into(),
        size_bytes: 42,
        fingerprint: None,
        state: TicketState::Pending,
        job_id: None,
        asset_id: None,
        outcome: None,
        error: None,
        created_ms: 10,
        updated_ms: 10,
    };
    js.put_ticket(&t).unwrap();
    t.state = TicketState::Finalized;
    t.asset_id = Some("ast_x".into());
    t.outcome = Some(json!({"outcome": "created"}));
    t.updated_ms = 20;
    js.put_ticket(&t).unwrap();
    assert_eq!(js.get_ticket("tk_1").unwrap().unwrap(), t);
    assert_eq!(
        js.list_tickets(Some(TicketState::Pending)).unwrap().len(),
        0
    );
    assert_eq!(js.list_tickets(None).unwrap().len(), 1);
}
