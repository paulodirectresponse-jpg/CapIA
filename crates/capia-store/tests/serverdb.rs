//! Banco do servidor (Fase 6): tokens só com hash, idempotência, eventos→entregas atômicos,
//! recuperação após queda, schema futuro rejeitado sem tocar o arquivo.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::TempDir;

use capia_store::{
    ExportRow, IdemBegin, ProjectRow, SERVER_APPLICATION_ID, ServerDb, StoreErrorCode, TokenRow,
    WebhookRow,
};
use serde_json::json;
use std::time::Duration;

fn open(dir: &std::path::Path) -> ServerDb {
    ServerDb::open(&dir.join("server.db"), Duration::from_secs(5)).unwrap()
}

fn tok(id: &str, hash: &str) -> TokenRow {
    TokenRow {
        id: id.into(),
        name: "t".into(),
        secret_hash: hash.into(),
        prefix: "capia_ab".into(),
        scopes: vec!["project:read".into()],
        created_ms: 10,
        expires_ms: None,
        last_used_ms: None,
        revoked_ms: None,
        rotated_from: None,
    }
}

fn hash(c: char) -> String {
    c.to_string().repeat(64)
}

#[test]
fn tokens_are_found_by_hash_revoked_and_rotated_atomically() {
    let d = TempDir::new("serverdb");
    let db = open(d.path());
    db.token_insert(&tok("a", &hash('a'))).unwrap();
    assert_eq!(db.token_by_hash(&hash('a')).unwrap().unwrap().id, "a");
    assert!(db.token_by_hash(&hash('b')).unwrap().is_none());
    // o hash é único
    assert!(db.token_insert(&tok("dup", &hash('a'))).is_err());
    let mut n = tok("b", &hash('b'));
    n.rotated_from = Some("a".into());
    assert!(db.token_rotate("a", &n, 50).unwrap());
    assert_eq!(db.token_get("a").unwrap().unwrap().revoked_ms, Some(50));
    assert!(db.token_get("b").unwrap().unwrap().revoked_ms.is_none());
    // rotacionar de novo o já revogado não cria outro token
    assert!(!db.token_rotate("a", &tok("c", &hash('c')), 60).unwrap());
    assert!(db.token_get("c").unwrap().is_none());
    assert_eq!(db.token_count_active(100).unwrap(), 1);
}

#[test]
fn idempotency_distinguishes_replay_mismatch_in_progress_and_indeterminate() {
    let d = TempDir::new("serverdb");
    let db = open(d.path());
    assert_eq!(
        db.idem_begin("t", "k", "runs.create", "h1", 1000, 60_000)
            .unwrap(),
        IdemBegin::New
    );
    assert_eq!(
        db.idem_begin("t", "k", "runs.create", "h1", 1001, 60_000)
            .unwrap(),
        IdemBegin::InProgress
    );
    db.idem_finish("t", "k", 202, "{\"ok\":true}").unwrap();
    assert_eq!(
        db.idem_begin("t", "k", "runs.create", "h1", 1002, 60_000)
            .unwrap(),
        IdemBegin::Replay {
            status: 202,
            body: "{\"ok\":true}".into()
        }
    );
    assert_eq!(
        db.idem_begin("t", "k", "runs.create", "OTHER", 1003, 60_000)
            .unwrap(),
        IdemBegin::Mismatch
    );
    assert_eq!(
        db.idem_begin("t", "k", "projects.create", "h1", 1003, 60_000)
            .unwrap(),
        IdemBegin::Mismatch
    );
    // outro token: namespace independente
    assert_eq!(
        db.idem_begin("u", "k", "runs.create", "h1", 1004, 60_000)
            .unwrap(),
        IdemBegin::New
    );
    // pending antigo = processo caiu no meio
    assert_eq!(
        db.idem_begin("u", "k", "runs.create", "h1", 1004 + 120_000, 60_000)
            .unwrap(),
        IdemBegin::Indeterminate
    );
    // falha antes de qualquer efeito libera a chave
    db.idem_release("u", "k").unwrap();
    assert_eq!(
        db.idem_begin("u", "k", "runs.create", "h1", 5, 60_000)
            .unwrap(),
        IdemBegin::New
    );
}

fn hook(id: &str, events: &[&str]) -> WebhookRow {
    WebhookRow {
        id: id.into(),
        url: "http://127.0.0.1:1/x".into(),
        events: events.iter().map(|s| (*s).to_owned()).collect(),
        secret_ref: format!("webhook:{id}"),
        enabled: true,
        description: String::new(),
        created_ms: 1,
        updated_ms: 1,
    }
}

#[test]
fn an_event_enqueues_one_delivery_per_subscribed_webhook_and_is_idempotent() {
    let d = TempDir::new("serverdb");
    let db = open(d.path());
    db.webhook_insert(&hook("w1", &["run.completed"])).unwrap();
    db.webhook_insert(&hook("w2", &["*"])).unwrap();
    db.webhook_insert(&hook("w3", &["export.completed"]))
        .unwrap();
    let seq = db
        .event_append(
            "e1",
            "run.completed",
            100,
            Some("p"),
            Some("r"),
            None,
            &json!({"a":1}),
            None,
        )
        .unwrap()
        .unwrap();
    // o mesmo event_id nunca vira evento novo
    assert!(
        db.event_append(
            "e1",
            "run.completed",
            101,
            None,
            None,
            None,
            &json!({}),
            None
        )
        .unwrap()
        .is_none()
    );
    let due = db.deliveries_claim_due(100, 10).unwrap();
    let mut hooks: Vec<_> = due.iter().map(|x| x.webhook_id.clone()).collect();
    hooks.sort();
    assert_eq!(hooks, ["w1", "w2"]);
    assert!(due.iter().all(|x| x.event_seq == seq));
    // reservadas: não saem de novo
    assert!(db.deliveries_claim_due(100, 10).unwrap().is_empty());
    // queda: o que estava `delivering` volta a `retrying`
    assert_eq!(db.deliveries_recover(200).unwrap(), 2);
    assert_eq!(db.deliveries_claim_due(200, 10).unwrap().len(), 2);
    let ev = db.events_after(0, 10).unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].data, json!({"a":1}));
}

#[test]
fn delivery_state_machine_persists_attempts_and_allows_redelivery() {
    let d = TempDir::new("serverdb");
    let db = open(d.path());
    db.webhook_insert(&hook("w", &["*"])).unwrap();
    db.event_append("e", "run.failed", 5, None, None, None, &json!({}), None)
        .unwrap();
    let del = db.deliveries_claim_due(5, 1).unwrap().remove(0);
    db.delivery_finish(
        del.id,
        "retrying",
        1,
        9_000,
        Some(503),
        Some(12),
        Some("boom"),
        6,
    )
    .unwrap();
    assert!(db.deliveries_claim_due(8_999, 1).unwrap().is_empty());
    let again = db.deliveries_claim_due(9_000, 1).unwrap();
    assert_eq!(again[0].attempt, 1);
    assert_eq!(again[0].last_status, Some(503));
    db.delivery_finish(del.id, "dead", 8, 0, None, None, Some("gave up"), 7)
        .unwrap();
    assert_eq!(db.delivery_get(del.id).unwrap().unwrap().state, "dead");
    assert!(db.delivery_requeue(del.id, 10).unwrap());
    assert_eq!(db.delivery_get(del.id).unwrap().unwrap().attempt, 0);
    // apagar o webhook apaga as entregas
    assert!(db.webhook_delete("w").unwrap());
    assert!(db.delivery_get(del.id).unwrap().is_none());
}

#[test]
fn exports_and_projects_roundtrip_and_interrupted_exports_are_failed_on_reopen() {
    let d = TempDir::new("serverdb");
    {
        let db = open(d.path());
        db.project_insert(&ProjectRow {
            id: "p1".into(),
            name: "P".into(),
            path: "/x/p1.capia".into(),
            created_ms: 1,
            last_opened_ms: None,
        })
        .unwrap();
        assert!(
            db.project_insert(&ProjectRow {
                id: "p2".into(),
                name: "Q".into(),
                path: "/x/p1.capia".into(),
                created_ms: 1,
                last_opened_ms: None,
            })
            .is_err(),
            "paths are unique"
        );
        db.export_insert(&ExportRow {
            id: "x1".into(),
            project_id: "p1".into(),
            batch: "batch-1".into(),
            sequence: "seq".into(),
            preset: "h264-mp4".into(),
            state: "running".into(),
            path: Some("/e/x1.mp4".into()),
            report: None,
            error: None,
            created_ms: 2,
            finished_ms: None,
        })
        .unwrap();
        db.export_update("x1", "running", None, None, 3).unwrap();
    }
    let db = open(d.path());
    assert_eq!(db.exports_recover(99).unwrap(), 1);
    let e = db.export_get("x1").unwrap().unwrap();
    assert_eq!(e.state, "failed");
    assert_eq!(e.error.unwrap()["code"], "INTERRUPTED");
    assert_eq!(db.project_by_path("/x/p1.capia").unwrap().unwrap().id, "p1");
    db.export_update("x1", "completed", Some(&json!({"frames": 3})), None, 100)
        .unwrap();
    assert_eq!(
        db.export_get("x1").unwrap().unwrap().report.unwrap()["frames"],
        3
    );
}

#[test]
fn a_foreign_or_future_database_is_refused_without_being_touched() {
    let d = TempDir::new("serverdb");
    let p = d.path().join("server.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE x(a); PRAGMA user_version = 1;")
            .unwrap();
    }
    let before = std::fs::read(&p).unwrap();
    let e = ServerDb::open(&p, Duration::from_secs(1)).unwrap_err();
    assert_eq!(e.code, StoreErrorCode::NotACapiaProject);
    assert_eq!(std::fs::read(&p).unwrap(), before);
    // futuro
    let p2 = d.path().join("future.db");
    {
        let c = rusqlite::Connection::open(&p2).unwrap();
        c.pragma_update(None, "application_id", SERVER_APPLICATION_ID)
            .unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
        c.execute_batch("CREATE TABLE x(a);").unwrap();
    }
    let before = std::fs::read(&p2).unwrap();
    let e = ServerDb::open(&p2, Duration::from_secs(1)).unwrap_err();
    assert_eq!(e.code, StoreErrorCode::UnsupportedSchemaVersion);
    assert_eq!(std::fs::read(&p2).unwrap(), before);
}

#[test]
fn a_db_left_by_a_killed_process_reopens_with_its_data() {
    let d = TempDir::new("serverdb");
    let p = d.path().join("server.db");
    let exe = std::env::current_exe().unwrap();
    // o filho abre, grava e morre sem fechar (SIGKILL/abort): o cabeçalho fica no -wal
    let st = std::process::Command::new(&exe)
        .args([
            "--exact",
            "child_writer_then_abort",
            "--nocapture",
            "--ignored",
        ])
        .env("CAPIA_SERVERDB_CHILD", &p)
        .status()
        .unwrap();
    assert!(!st.success());
    let db = ServerDb::open(&p, Duration::from_secs(5)).unwrap();
    assert!(db.token_get("t1").unwrap().is_some());
}

#[test]
#[ignore = "helper: roda só como processo filho do teste de queda"]
fn child_writer_then_abort() {
    let Some(p) = std::env::var_os("CAPIA_SERVERDB_CHILD") else {
        return;
    };
    let db = ServerDb::open(std::path::Path::new(&p), Duration::from_secs(5)).unwrap();
    db.token_insert(&tok("t1", &hash('d'))).unwrap();
    std::process::abort();
}

#[test]
fn a_test_event_is_delivered_only_to_the_target_webhook() {
    let d = TempDir::new("serverdb-test-event");
    let db = open(d.path());
    db.webhook_insert(&hook("w1", &["*"])).unwrap();
    let mut off = hook("w2", &["*"]);
    off.enabled = false;
    db.webhook_insert(&off).unwrap();
    db.event_append(
        "t1",
        "webhook.test",
        1,
        None,
        None,
        None,
        &json!({}),
        Some("w2"),
    )
    .unwrap();
    let due = db.deliveries_claim_due(10, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].webhook_id, "w2");
}

#[test]
fn the_audit_log_is_trimmed_to_the_most_recent_entries() {
    let d = TempDir::new("serverdb-audit");
    let db = open(d.path());
    for i in 0..50u64 {
        db.audit_append(&capia_store::AuditRow {
            at_ms: i,
            request_id: format!("r{i}"),
            surface: "rest".into(),
            op: "x".into(),
            outcome: "ok".into(),
            ..Default::default()
        })
        .unwrap();
    }
    assert_eq!(db.audit_trim(10).unwrap(), 40);
    let rows = db.audit_list(0, 100).unwrap();
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[0].request_id, "r40");
    assert_eq!(db.audit_trim(10).unwrap(), 0);
}
