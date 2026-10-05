//! Schema 4 (registros de IA + uso) e o app DB global: migrations, versionamento, recuperação de
//! tarefas interrompidas, segredos fora do disco, arquivos alheios/futuros intocados.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_store::{
    APP_APPLICATION_ID, AiStore, AppDb, CURRENT_SCHEMA_VERSION, ProjectStore, StoreErrorCode,
    UsageRow,
};
use common::*;
use rusqlite::Connection;
use serde_json::json;
use std::time::Duration;

const BUSY: Duration = Duration::from_millis(2_000);

fn project(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.file("p.capia");
    let (store, _s) = ProjectStore::create(&path, &fast()).unwrap();
    assert_eq!(store.schema_version(), CURRENT_SCHEMA_VERSION);
    store.close().unwrap();
    path
}

#[test]
fn an_old_project_migrates_to_the_current_schema_with_empty_ai_tables() {
    let dir = TempDir::new("ai-mig");
    let path = dir.file("old.capia");
    let (store, _s) = ProjectStore::create_at_schema(&path, &fast(), 3).unwrap();
    assert_eq!(store.schema_version(), 3);
    store.close().unwrap();
    let (store, _) = ProjectStore::open(&path, &fast()).unwrap();
    assert_eq!(store.schema_version(), 5);
    store.close().unwrap();
    let c = Connection::open(&path).unwrap();
    for t in [
        "ai_records",
        "ai_usage",
        "ai_runs",
        "ai_run_stages",
        "ai_run_events",
        "ai_side_effects",
        "ai_provenance",
        "ai_memory",
        "ai_memory_log",
        "ai_budget_ledger",
    ] {
        let n: i64 = c
            .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{t}");
    }
    assert!(
        dir.file("old.capia.v3.bak").exists(),
        "backup antes de migrar"
    );
}

#[test]
fn records_are_versioned_listed_and_latest_wins() {
    let dir = TempDir::new("ai-rec");
    let s = AiStore::open(&project(&dir), BUSY).unwrap();
    s.put(
        "demand_spec",
        "main",
        1,
        None,
        1,
        &json!({"objective": "v1"}),
        10,
    )
    .unwrap();
    s.put(
        "demand_spec",
        "main",
        2,
        None,
        1,
        &json!({"objective": "v2"}),
        20,
    )
    .unwrap();
    s.put(
        "transcript",
        "asset_a|fp1",
        0,
        Some("asset_a"),
        1,
        &json!({"segments": []}),
        30,
    )
    .unwrap();
    assert_eq!(
        s.latest("demand_spec", "main").unwrap().unwrap().json["objective"],
        "v2"
    );
    assert_eq!(
        s.get("demand_spec", "main", 1).unwrap().unwrap().json["objective"],
        "v1"
    );
    assert!(s.get("demand_spec", "main", 9).unwrap().is_none());
    assert_eq!(s.list("demand_spec", None, false, 10).unwrap().len(), 2);
    assert_eq!(s.list("demand_spec", None, true, 10).unwrap().len(), 1);
    assert_eq!(
        s.list("transcript", Some("asset_a"), true, 10)
            .unwrap()
            .len(),
        1
    );
    assert!(
        s.list("transcript", Some("outro"), true, 10)
            .unwrap()
            .is_empty()
    );
    // upsert da mesma versão atualiza
    s.put(
        "demand_spec",
        "main",
        2,
        None,
        1,
        &json!({"objective": "v2b"}),
        40,
    )
    .unwrap();
    assert_eq!(
        s.latest("demand_spec", "main").unwrap().unwrap().json["objective"],
        "v2b"
    );
    assert_eq!(s.delete("demand_spec", "main").unwrap(), 2);
    // limites
    assert_eq!(
        s.put("", "x", 0, None, 1, &json!({}), 1).unwrap_err().code,
        StoreErrorCode::InvalidArgument
    );
    assert_eq!(
        s.put("k", "x", 0, None, 0, &json!({}), 1).unwrap_err().code,
        StoreErrorCode::InvalidArgument
    );
}

#[test]
fn registered_secrets_never_reach_the_project_file() {
    let dir = TempDir::new("ai-secret");
    let path = project(&dir);
    let secret = "CNRY-store-canary-0123456789ab";
    capia_secrets::register_global(secret);
    let s = AiStore::open(&path, BUSY).unwrap();
    s.put(
        "ai_task",
        "t1",
        0,
        None,
        1,
        &json!({"state": "done", "note": format!("leaked {secret}!")}),
        1,
    )
    .unwrap();
    s.add_usage(&UsageRow {
        request_id: "r".into(),
        task_id: Some("t1".into()),
        provider_id: "p".into(),
        endpoint_id: "p:m".into(),
        model_id: "m".into(),
        capability: "TextGeneration".into(),
        purpose: None,
        input_tokens: 1,
        output_tokens: 1,
        cached_tokens: 0,
        synthetic: false,
        cost_known: false,
        cost_micros: 0,
        currency: None,
        latency_ms: 1,
        attempt: 1,
        status: "failed".into(),
        error_code: Some(format!("echo {secret}")),
        pricing_date: None,
        timestamp_ms: 1,
    })
    .unwrap();
    drop(s);
    let bytes = std::fs::read(&path).unwrap();
    let wal = std::fs::read(dir.file("p.capia-wal")).unwrap_or_default();
    for (name, b) in [("db", bytes), ("wal", wal)] {
        assert!(
            !b.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "segredo em {name}"
        );
    }
    let s = AiStore::open(&path, BUSY).unwrap();
    assert!(
        s.latest("ai_task", "t1").unwrap().unwrap().json["note"]
            .as_str()
            .unwrap()
            .contains("[REDACTED]")
    );
}

#[test]
fn interrupted_tasks_are_never_left_running_or_completed() {
    let dir = TempDir::new("ai-recover");
    let s = AiStore::open(&project(&dir), BUSY).unwrap();
    for (id, st) in [
        ("a", "running"),
        ("b", "queued"),
        ("c", "completed"),
        ("d", "failed"),
    ] {
        s.put(
            "ai_task",
            id,
            0,
            None,
            1,
            &json!({"state": st, "title": id}),
            1,
        )
        .unwrap();
    }
    assert_eq!(s.recover_tasks(99).unwrap(), 2);
    let state = |id: &str| {
        s.latest("ai_task", id).unwrap().unwrap().json["state"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(state("a"), "interrupted");
    assert_eq!(state("b"), "interrupted");
    assert_eq!(state("c"), "completed", "o que já terminou não é tocado");
    assert_eq!(state("d"), "failed");
    assert_eq!(s.recover_tasks(100).unwrap(), 0, "idempotente");
}

#[test]
fn usage_summary_keeps_unknown_cost_separate_from_zero() {
    let dir = TempDir::new("ai-usage");
    let s = AiStore::open(&project(&dir), BUSY).unwrap();
    let row = |status: &str, known: bool, micros: u64| UsageRow {
        request_id: "r".into(),
        task_id: Some("t".into()),
        provider_id: "p".into(),
        endpoint_id: "p:m".into(),
        model_id: "m".into(),
        capability: "TextGeneration".into(),
        purpose: None,
        input_tokens: 100,
        output_tokens: 10,
        cached_tokens: 0,
        synthetic: false,
        cost_known: known,
        cost_micros: micros,
        currency: known.then(|| "USD".to_owned()),
        latency_ms: 5,
        attempt: 1,
        status: status.into(),
        error_code: None,
        pricing_date: None,
        timestamp_ms: 1,
    };
    s.add_usage(&row("ok", true, 500)).unwrap();
    s.add_usage(&row("ok", false, 0)).unwrap();
    s.add_usage(&row("failed", false, 0)).unwrap();
    s.add_usage(&row("cache_hit", true, 0)).unwrap();
    let sum = s.usage_summary(Some("t")).unwrap();
    assert_eq!(
        (
            sum.calls,
            sum.failed_calls,
            sum.cache_hits,
            sum.unknown_cost_calls
        ),
        (2, 1, 1, 1)
    );
    assert_eq!(
        (sum.known_cost_micros, sum.currency.as_deref()),
        (500, Some("USD"))
    );
    assert_eq!(sum.input_tokens, 200);
    assert_eq!(s.usage_summary(Some("outra")).unwrap().calls, 0);
    assert_eq!(s.usage_for_task("t").unwrap().len(), 4);
}

#[test]
fn app_db_roundtrip_overwrite_and_reopen() {
    let dir = TempDir::new("app-rt");
    let p = dir.file("sub/app.db");
    let db = AppDb::open(&p, BUSY).unwrap();
    assert_eq!(db.schema_version().unwrap(), 1);
    assert!(db.get("ai", "registry").unwrap().is_none());
    db.put("ai", "registry", &json!({"providers": {}}), 1)
        .unwrap();
    db.put("ai", "registry", &json!({"providers": {"a": 1}}), 2)
        .unwrap();
    db.put("ai", "budgets", &json!({"x": 1}), 3).unwrap();
    drop(db);
    let db = AppDb::open(&p, BUSY).unwrap();
    assert_eq!(
        db.get("ai", "registry").unwrap().unwrap()["providers"]["a"],
        1
    );
    assert_eq!(db.list_keys("ai").unwrap(), vec!["budgets", "registry"]);
    assert!(db.delete("ai", "budgets").unwrap());
    assert!(!db.delete("ai", "budgets").unwrap());
    let c = Connection::open(&p).unwrap();
    let id: i64 = c
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    assert_eq!(id, APP_APPLICATION_ID);
}

#[test]
fn app_db_refuses_future_schema_foreign_sqlite_and_garbage_without_touching_them() {
    let dir = TempDir::new("app-bad");
    // 1) schema futuro
    let fut = dir.file("future.db");
    drop(AppDb::open(&fut, BUSY).unwrap());
    {
        let c = Connection::open(&fut).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
        c.pragma_update(None, "journal_mode", "DELETE").unwrap();
    }
    let before = std::fs::read(&fut).unwrap();
    let e = AppDb::open(&fut, BUSY).unwrap_err();
    assert_eq!(e.code, StoreErrorCode::UnsupportedSchemaVersion, "{e}");
    assert_eq!(
        std::fs::read(&fut).unwrap(),
        before,
        "o arquivo não foi tocado"
    );
    // 2) SQLite alheio
    let foreign = dir.file("foreign.db");
    {
        let c = Connection::open(&foreign).unwrap();
        c.execute_batch("CREATE TABLE t (x); PRAGMA application_id = 12345; PRAGMA user_version = 1; PRAGMA journal_mode = DELETE;").unwrap();
    }
    let before = std::fs::read(&foreign).unwrap();
    assert_eq!(
        AppDb::open(&foreign, BUSY).unwrap_err().code,
        StoreErrorCode::NotACapiaProject
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), before);
    // 3) lixo
    let junk = dir.file("junk.db");
    std::fs::write(&junk, b"isto nao e sqlite").unwrap();
    assert_eq!(
        AppDb::open(&junk, BUSY).unwrap_err().code,
        StoreErrorCode::NotACapiaProject
    );
    assert_eq!(std::fs::read(&junk).unwrap(), b"isto nao e sqlite");
    // 4) vazio
    let empty = dir.file("empty.db");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(
        AppDb::open(&empty, BUSY).unwrap_err().code,
        StoreErrorCode::NotACapiaProject
    );
    // 5) truncado no meio: erro estruturado, nunca pânico
    let trunc = dir.file("trunc.db");
    {
        let db = AppDb::open(&trunc, BUSY).unwrap();
        db.put("ai", "k", &json!({"big": "x".repeat(50_000)}), 1)
            .unwrap();
    }
    {
        let c = Connection::open(&trunc).unwrap();
        c.pragma_update(None, "wal_checkpoint", "TRUNCATE").ok();
    }
    let bytes = std::fs::read(&trunc).unwrap();
    std::fs::write(&trunc, &bytes[..bytes.len() / 3]).unwrap();
    let r = AppDb::open(&trunc, BUSY).and_then(|db| db.get("ai", "k").map(|_| ()));
    if let Err(e) = r {
        assert!(
            matches!(
                e.code,
                StoreErrorCode::ProjectCorrupted
                    | StoreErrorCode::NotACapiaProject
                    | StoreErrorCode::StoreIoError
                    | StoreErrorCode::TransactionFailed
            ),
            "{e}"
        );
    }
}

#[test]
fn app_db_migrates_forward_with_a_backup() {
    use capia_store::Migration;
    fn v2(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE extra (x INTEGER);")
    }
    let dir = TempDir::new("app-mig");
    let p = dir.file("app.db");
    {
        let db = AppDb::open(&p, BUSY).unwrap();
        db.put("ai", "k", &json!({"keep": true}), 1).unwrap();
    }
    let mut list = capia_store::APP_MIGRATIONS.to_vec();
    list.push(Migration {
        version: 2,
        name: "extra",
        up: v2,
    });
    let db = AppDb::open_with(&p, BUSY, &list, 2).unwrap();
    assert_eq!(db.schema_version().unwrap(), 2);
    assert_eq!(db.get("ai", "k").unwrap().unwrap()["keep"], true);
    assert!(dir.file("app.db.v1.bak").exists());
    drop(db);
    // o build antigo (que só entende v1) recusa o arquivo migrado, sem tocá-lo
    assert_eq!(
        AppDb::open(&p, BUSY).unwrap_err().code,
        StoreErrorCode::UnsupportedSchemaVersion
    );
}

#[test]
fn app_db_redacts_registered_secrets() {
    let dir = TempDir::new("app-secret");
    let p = dir.file("app.db");
    let secret = "CNRY-app-canary-9f8e7d6c5b4a";
    capia_secrets::register_global(secret);
    let db = AppDb::open(&p, BUSY).unwrap();
    db.put(
        "ai",
        "registry",
        &json!({"note": secret, "nested": {"x": [secret]}}),
        1,
    )
    .unwrap();
    drop(db);
    for f in ["app.db", "app.db-wal"] {
        let b = std::fs::read(dir.file(f)).unwrap_or_default();
        assert!(
            !b.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{f}"
        );
    }
}

#[test]
fn an_app_db_left_by_a_killed_process_reopens_with_its_data() {
    // O cabeçalho do arquivo principal só é atualizado no checkpoint: depois de um kill, o
    // `application_id` e os dados ainda estão no `-wal`. Reabrir NÃO pode tomar isso por "arquivo
    // alheio" (regressão achada pelo E2E de autonomia: app reaberto após kill -9).
    let dir = TempDir::new("appdb-kill");
    let p = dir.file("app.db");
    let db = AppDb::open(&p, BUSY).unwrap();
    db.put("ns", "k", &json!({"v": 1}), 1).unwrap();
    std::mem::forget(db); // sem fechar: nada de checkpoint (como um processo morto)
    let again = AppDb::open(&p, BUSY).expect("a killed app db must reopen");
    assert_eq!(again.get("ns", "k").unwrap(), Some(json!({"v": 1})));
}
