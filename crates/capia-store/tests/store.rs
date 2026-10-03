//! Integração do store: create → commit → close → reopen, undo/redo após reopen, idempotência
//! durável, rollback, escritor obsoleto/ocupado e determinismo do digest.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::{Actor, Command, Engine};
use capia_model::ErrorCode;
use capia_store::{
    APPLICATION_ID, CURRENT_SCHEMA_VERSION, ProjectStore, StoreErrorCode, StoreOptions, Synchronous,
};
use common::*;
use rusqlite::Connection;
use std::time::Duration;

fn count(path: &std::path::Path, table: &str) -> i64 {
    let c = Connection::open(path).unwrap();
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn create_then_reopen_yields_the_same_empty_project() {
    let dir = TempDir::new("create");
    let path = dir.file("p.capia");
    let (store, state) = ProjectStore::create(&path, &fast()).unwrap();
    let id = store.project_id().to_owned();
    assert!(id.starts_with("prj_") && id.len() == 36, "{id}");
    assert_eq!(store.schema_version(), CURRENT_SCHEMA_VERSION);
    assert_eq!(state.doc.revision, 0);
    store.close().unwrap();
    assert!(
        !dir.file("p.capia-wal").exists() && !dir.file("p.capia-shm").exists(),
        "no sidecars at rest"
    );

    let (store, state2) = ProjectStore::open(&path, &fast()).unwrap();
    assert_eq!(store.project_id(), id);
    assert_eq!(state2, state);
    // assinatura do formato
    let raw = Connection::open(&path).unwrap();
    let app: i64 = raw
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    let ver: i64 = raw
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(
        (app, ver),
        (APPLICATION_ID, i64::from(CURRENT_SCHEMA_VERSION))
    );
    let mode: String = raw
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn create_refuses_to_overwrite_and_leaves_no_temp_files() {
    let dir = TempDir::new("exists");
    let path = dir.file("p.capia");
    ProjectStore::create(&path, &fast())
        .unwrap()
        .0
        .close()
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let e = ProjectStore::create(&path, &fast()).unwrap_err();
    assert_eq!(e.code, StoreErrorCode::ProjectAlreadyExists);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the existing project is untouched"
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["p.capia"], "no .creating-* leftovers: {names:?}");
}

#[test]
fn committed_work_survives_close_and_reopen_with_full_history() {
    let dir = TempDir::new("reopen");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(
        &user(),
        tx(
            "clips",
            vec![
                insert("c1", "V1", 0, "a", 10),
                insert("c2", "V1", 20, "b", 10),
            ],
        ),
        2,
    )
    .unwrap();
    let digest = capia_commands::document_digest(e.document());
    let (rev, hist) = (e.revision(), e.history().len());
    drop(e);

    let e = open(&path, &fast());
    assert_eq!(
        capia_commands::document_digest(e.document()),
        digest,
        "document is bit-identical"
    );
    assert_eq!((e.revision(), e.history().len()), (rev, hist));
    assert_eq!(clip_ids(&e, "V1"), ["a", "b"]);
    assert!(e.can_undo() && !e.can_redo());
    assert_eq!(e.audit_log().len(), 2);
    assert!(e.applied_operation("c1").is_some() && e.applied_operation("a-seq").is_some());
    for (op, label) in [("c1", "clips"), ("a-seq", "setup")] {
        let entry = e
            .history()
            .iter()
            .find(|h| h.commands.iter().any(|c| c.operation_id == op))
            .unwrap();
        assert_eq!(entry.label, label);
        assert!(!entry.inverse_ops.is_empty());
    }
}

#[test]
fn undo_and_redo_work_after_reopen_and_persist_themselves() {
    let dir = TempDir::new("undo");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(&user(), tx("one", vec![insert("c1", "V1", 0, "a", 10)]), 2)
        .unwrap();
    e.execute(&user(), tx("two", vec![insert("c2", "V1", 20, "b", 10)]), 3)
        .unwrap();
    let after_two = sem(e.document());
    drop(e);

    // reabre e desfaz o último commit
    let mut e = open(&path, &fast());
    e.undo(&user(), 4).unwrap();
    assert_eq!(clip_ids(&e, "V1"), ["a"]);
    let after_undo = sem(e.document());
    drop(e);

    // reabre de novo: o undo persistiu; redo restaura exatamente o estado anterior
    let mut e = open(&path, &fast());
    assert_eq!(sem(e.document()), after_undo);
    assert!(e.can_redo());
    e.redo(&user(), 5).unwrap();
    assert_eq!(sem(e.document()), after_two);
    drop(e);

    // e desfazer ate o início depois de reabrir percorre toda a pilha gravada
    let mut e = open(&path, &fast());
    assert_eq!(sem(e.document()), after_two);
    while e.can_undo() {
        e.undo(&user(), 6).unwrap();
    }
    assert_eq!(e.document().sequence_count(), 0);
    drop(e);
    let e = open(&path, &fast());
    assert_eq!(e.document().sequence_count(), 0);
    assert!(!e.can_undo() && e.can_redo());
}

#[test]
fn a_new_edit_after_undo_drops_the_redo_branch_durably() {
    let dir = TempDir::new("branch");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(&user(), tx("x", vec![insert("c1", "V1", 0, "a", 10)]), 2)
        .unwrap();
    e.undo(&user(), 3).unwrap();
    e.execute(&user(), tx("y", vec![insert("c2", "V1", 50, "b", 10)]), 4)
        .unwrap();
    assert!(!e.can_redo());
    drop(e);
    let mut e = open(&path, &fast());
    assert!(!e.can_redo(), "the discarded branch does not come back");
    assert_eq!(clip_ids(&e, "V1"), ["b"]);
    assert_eq!(
        e.applied_history()
            .iter()
            .map(|h| h.label.as_str())
            .collect::<Vec<_>>(),
        ["setup", "y"]
    );
    // mas o operation_id do ramo descartado continua registrado (undo não libera ids)
    let again = e
        .execute(&user(), tx("x", vec![insert("c1", "V1", 0, "a", 10)]), 5)
        .unwrap();
    assert!(again.replayed);
    assert_eq!(clip_ids(&e, "V1"), ["b"]);
    // toda a auditoria (commit, undo, commit) foi gravada
    assert_eq!(count(&path, "events"), 4);
}

#[test]
fn operation_ids_are_durable_across_reopen() {
    let dir = TempDir::new("idem");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    let t = tx("clip", vec![insert("abc", "V1", 0, "a", 10)]);
    let first = e.execute(&user(), t.clone(), 2).unwrap();
    drop(e);

    let mut e = open(&path, &fast());
    let rev = e.revision();
    let again = e.execute(&user(), t, 3).unwrap();
    assert!(
        again.replayed,
        "same operation_id after reopen is not re-applied"
    );
    assert_eq!(again.entry_id, first.entry_id);
    assert_eq!(
        again.results, first.results,
        "the original result comes back"
    );
    assert_eq!(e.revision(), rev);
    assert_eq!(clip_ids(&e, "V1"), ["a"]);
    // mesmo id com comando diferente continua sendo rejeitado
    let reused = e
        .execute(
            &user(),
            tx("other", vec![insert("abc", "V1", 50, "z", 5)]),
            4,
        )
        .unwrap_err();
    assert_eq!(reused.code, ErrorCode::OperationIdReused);
    // e o undo não libera o id, nem depois de reabrir
    e.undo(&user(), 5).unwrap();
    drop(e);
    let mut e = open(&path, &fast());
    let r = e
        .execute(
            &user(),
            tx("clip", vec![insert("abc", "V1", 0, "a", 10)]),
            6,
        )
        .unwrap();
    assert!(r.replayed);
    assert!(clip_ids(&e, "V1").is_empty());
}

#[test]
fn the_store_exposes_the_durable_operation_log() {
    let dir = TempDir::new("oplog");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    let r = e
        .execute(
            &Actor::system(),
            tx("clip", vec![insert("abc", "V1", 0, "a", 10)]),
            9,
        )
        .unwrap();
    drop(e);
    let (store, _) = ProjectStore::open(&path, &fast()).unwrap();
    assert!(store.has_operation_id("abc").unwrap());
    assert!(!store.has_operation_id("nope").unwrap());
    let stored = store.load_operation_result("abc").unwrap().unwrap();
    assert_eq!(stored.result, r);
    assert_eq!(stored.applied.applied_at_ms, 9);
    assert_eq!(stored.applied.actor, Actor::system());
    assert!(store.load_operation_result("nope").unwrap().is_none());
    assert_eq!(store.stats().unwrap().operations, 4);
}

#[test]
fn the_semantic_digest_is_identical_before_and_after_save_and_reopen() {
    let dir = TempDir::new("digest");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(
        &user(),
        tx(
            "c",
            vec![
                insert("c1", "V1", 0, "a", 10),
                insert("c2", "V2", 3, "b", 7),
            ],
        ),
        2,
    )
    .unwrap();
    let a = capia_commands::document_digest(e.document());
    drop(e);
    let e = open(&path, &fast());
    let b = capia_commands::document_digest(e.document());
    assert_eq!(a, b);
    // metadados voláteis vivem fora do documento: reabrir em outro instante não muda nada
    std::thread::sleep(Duration::from_millis(20));
    drop(e);
    let e = open(&path, &fast());
    assert_eq!(capia_commands::document_digest(e.document()), a);
    // e o digest do arquivo bate com o do documento em memória
    let info = ProjectStore::inspect(&path).unwrap();
    assert_eq!(info.digest, a);
}

#[test]
fn snapshots_are_taken_on_cadence_and_reopen_replays_the_tail() {
    let dir = TempDir::new("snap");
    let path = dir.file("p.capia");
    let opts = StoreOptions {
        snapshot_every: 3,
        ..fast()
    };
    let mut e = create(&path, &opts);
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    for i in 0..7 {
        e.execute(
            &user(),
            tx(
                "c",
                vec![insert(&format!("c{i}"), "V1", i * 20, &format!("k{i}"), 10)],
            ),
            2,
        )
        .unwrap();
    }
    e.undo(&user(), 3).unwrap();
    let want = sem(e.document());
    let revision = e.revision();
    drop(e);
    let info = ProjectStore::inspect(&path).unwrap();
    assert!(info.stats.snapshot_seq >= 6, "{:?}", info.stats);
    assert!(info.stats.events_since_snapshot < 3);
    assert_eq!(info.stats.events, 9);
    assert_eq!(
        count(&path, "snapshots"),
        1,
        "only the latest snapshot is kept"
    );
    let mut e = open(&path, &opts);
    assert_eq!(sem(e.document()), want);
    assert_eq!(e.revision(), revision);
    // continua editando por cima do replay
    e.execute(&user(), tx("more", vec![insert("z", "V2", 0, "zz", 5)]), 4)
        .unwrap();
    drop(e);
    let e = open(&path, &opts);
    assert_eq!(clip_ids(&e, "V2"), ["zz"]);
}

#[test]
fn a_failure_inside_the_transaction_rolls_everything_back() {
    let dir = TempDir::new("rollback");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    // um registro "intruso" faz o INSERT em applied_operations falhar DEPOIS de a entrada de
    // histórico e o resultado terem sido gravados dentro da mesma transação
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute(
            "INSERT INTO applied_operations(operation_id, payload_hash, entry_id, applied_at_ms, actor_json) VALUES ('boom', 'x', 1, 0, '{\"kind\":\"user\",\"id\":\"x\"}')",
            [],
        )
        .unwrap();
    }
    let (entries, results, events, ops) = (
        count(&path, "history_entries"),
        count(&path, "commit_results"),
        count(&path, "events"),
        count(&path, "applied_operations"),
    );
    let before = e.document().clone();
    let err = e
        .execute(
            &user(),
            tx("boom", vec![insert("boom", "V1", 0, "a", 10)]),
            2,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PersistenceFailed);
    assert_eq!(
        err.hint.as_ref().unwrap()["store_code"],
        "TRANSACTION_FAILED"
    );
    assert_eq!(e.document(), &before, "memory is untouched");
    assert_eq!(e.history().len(), 1);
    // nada do que veio antes do ponto de falha ficou no arquivo
    assert_eq!(
        (
            count(&path, "history_entries"),
            count(&path, "commit_results"),
            count(&path, "events"),
            count(&path, "applied_operations")
        ),
        (entries, results, events, ops)
    );
    // o engine continua utilizável e persistindo
    e.execute(&user(), tx("fine", vec![insert("ok", "V1", 0, "a", 10)]), 3)
        .unwrap();
    drop(e);
    assert_eq!(clip_ids(&open(&path, &fast()), "V1"), ["a"]);
}

#[test]
fn a_stale_engine_cannot_write_and_nothing_is_stored() {
    let dir = TempDir::new("stale");
    let path = dir.file("p.capia");
    let mut a = create(&path, &fast());
    a.execute(&user(), setup_tx("a"), 1).unwrap();
    let mut b = open(&path, &fast());
    a.execute(
        &user(),
        tx("a-edit", vec![insert("a1", "V1", 0, "x", 10)]),
        2,
    )
    .unwrap();
    let events = count(&path, "events");
    let err = b
        .execute(
            &user(),
            tx("b-edit", vec![insert("b1", "V2", 0, "y", 10)]),
            3,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PersistenceFailed);
    assert_eq!(err.hint.as_ref().unwrap()["store_code"], "STORE_CONFLICT");
    assert_eq!(count(&path, "events"), events);
    assert!(
        clip_ids(&b, "V2").is_empty(),
        "the stale engine did not publish"
    );
    drop((a, b));
    // reabrir resolve: o histórico tem a edição de A, e B pode editar por cima
    let mut b = open(&path, &fast());
    assert_eq!(clip_ids(&b, "V1"), ["x"]);
    b.execute(
        &user(),
        tx("b-edit", vec![insert("b1", "V2", 0, "y", 10)]),
        4,
    )
    .unwrap();
}

#[test]
fn a_busy_database_returns_a_structured_error_promptly_and_recovers() {
    let dir = TempDir::new("busy");
    let path = dir.file("p.capia");
    let opts = StoreOptions {
        busy_timeout: Duration::from_millis(150),
        ..fast()
    };
    let mut e = create(&path, &opts);
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    // outro "escritor" segura o lock
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    let err = e
        .execute(&user(), tx("t", vec![insert("c1", "V1", 0, "a", 10)]), 2)
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "bounded wait");
    assert_eq!(err.code, ErrorCode::PersistenceFailed);
    assert_eq!(err.hint.as_ref().unwrap()["store_code"], "STORE_BUSY");
    assert!(clip_ids(&e, "V1").is_empty());
    raw.execute_batch("ROLLBACK").unwrap();
    // liberado: o mesmo engine (memória intacta) consegue gravar
    e.execute(&user(), tx("t", vec![insert("c1", "V1", 0, "a", 10)]), 3)
        .unwrap();
    drop((e, raw));
    assert_eq!(clip_ids(&open(&path, &opts), "V1"), ["a"]);
}

#[test]
fn concurrent_writers_never_corrupt_the_project() {
    let dir = TempDir::new("concurrent");
    let path = dir.file("p.capia");
    {
        let mut e = create(&path, &fast());
        e.execute(&user(), setup_tx("a"), 1).unwrap();
    }
    const THREADS: usize = 4;
    const PER_THREAD: usize = 6;
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let opts = StoreOptions {
                    busy_timeout: Duration::from_millis(2_000),
                    ..fast()
                };
                let mut retries = 0u32;
                for i in 0..PER_THREAD {
                    let op = format!("t{t}-{i}");
                    // abre → tenta → em STALE_HEAD/BUSY reabre e tenta de novo (mesmo operation_id)
                    loop {
                        let mut e = open(&path, &opts);
                        let track = if t % 2 == 0 { "V1" } else { "V2" };
                        let t_ = tx(
                            "w",
                            vec![insert(
                                &op,
                                track,
                                i64::try_from(t * 1000 + i * 20).unwrap(),
                                &op,
                                10,
                            )],
                        );
                        match e.execute(&user(), t_, 5) {
                            Ok(_) => break,
                            Err(err) if err.code == ErrorCode::PersistenceFailed => {
                                retries += 1;
                                assert!(retries < 5_000, "livelock");
                            }
                            Err(other) => panic!("unexpected error: {other}"),
                        }
                    }
                }
                retries
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let report = ProjectStore::validate_file(&path);
    assert!(report.ok, "{:?}", report.issues);
    let e = open(&path, &fast());
    assert_eq!(
        clip_ids(&e, "V1").len() + clip_ids(&e, "V2").len(),
        THREADS * PER_THREAD,
        "every write landed exactly once"
    );
    assert_eq!(
        e.revision(),
        1 + u64::try_from(THREADS * PER_THREAD).unwrap()
    );
    assert_eq!(
        count(&path, "events"),
        i64::try_from(1 + THREADS * PER_THREAD).unwrap()
    );
}

#[test]
fn full_synchronous_mode_is_the_default() {
    assert_eq!(StoreOptions::default().synchronous, Synchronous::Full);
    let dir = TempDir::new("sync");
    let path = dir.file("p.capia");
    let mut e = create(&path, &StoreOptions::default());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(&user(), tx("c", vec![insert("c1", "V1", 0, "a", 10)]), 2)
        .unwrap();
    drop(e);
    assert_eq!(
        clip_ids(&open(&path, &StoreOptions::default()), "V1"),
        ["a"]
    );
}

#[test]
fn engine_errors_are_unchanged_by_persistence() {
    // o mesmo comando inválido falha igual com e sem journal e não grava nada
    let dir = TempDir::new("errs");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(&user(), tx("c", vec![insert("c1", "V1", 0, "a", 10)]), 2)
        .unwrap();
    let events = count(&path, "events");
    let err = e
        .execute(&user(), tx("bad", vec![insert("c2", "V1", 5, "b", 10)]), 3)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Overlap);
    assert_eq!(count(&path, "events"), events);
    let _: Option<Engine> = None;
    let _ = Command::DeleteTrack { track: "V1".into() };
}

// ---- limites e entrada extrema (nunca panic) --------------------------------------------------------

#[test]
fn revision_counters_at_the_edge_fail_with_structured_errors_not_panics() {
    use capia_model::Document;
    let dir = TempDir::new("limits");
    // um documento já na revisão máxima que o store sabe gravar
    let mut doc = Document::new();
    doc.revision = i64::MAX as u64;
    let path = dir.file("edge.capia");
    let (store, state) = ProjectStore::create_with_document(&path, &doc, &fast()).unwrap();
    let mut e = store
        .into_engine(state, key(), capia_commands::EngineConfig::default())
        .unwrap();
    let err = e.execute(&user(), setup_tx("a"), 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::LimitExceeded, "{err}");
    drop(e);
    // e um arquivo cujo documento declara revisão u64::MAX é recusado como corrompido
    let path2 = dir.file("huge.capia");
    ProjectStore::create(&path2, &fast())
        .unwrap()
        .0
        .close()
        .unwrap();
    {
        let c = Connection::open(&path2).unwrap();
        let (text,): (String,) = c
            .query_row("SELECT document_json FROM snapshots", [], |r| {
                Ok((r.get(0)?,))
            })
            .unwrap();
        let mut d: Document = serde_json::from_str(&text).unwrap();
        d.revision = u64::MAX;
        let new = capia_commands::hash::canonical_json(&d);
        c.execute("PRAGMA ignore_check_constraints=ON", []).ok();
        c.execute(
            "UPDATE snapshots SET document_json = ?1, digest = ?2",
            rusqlite::params![new, capia_commands::document_digest(&d)],
        )
        .unwrap();
    }
    let e = ProjectStore::open(&path2, &fast()).unwrap_err();
    assert!(matches!(e.code, StoreErrorCode::ProjectCorrupted), "{e}");
}

#[test]
fn oversized_blobs_are_refused_instead_of_exhausting_memory() {
    let dir = TempDir::new("blob");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(&user(), tx("c", vec![insert("c1", "V1", 0, "a", 10)]), 2)
        .unwrap();
    drop(e);
    let tiny = StoreOptions {
        max_blob_bytes: 64,
        ..fast()
    };
    let err = ProjectStore::open(&path, &tiny).unwrap_err();
    assert_eq!(err.code, StoreErrorCode::ProjectCorrupted);
    assert!(err.message.contains("safety limit"), "{err}");
    // com o teto normal abre
    assert!(ProjectStore::open(&path, &fast()).is_ok());
    // entradas de histórico acima do teto também (o snapshot cabe, a entrada não)
    let snapshot_len: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT length(document_json) FROM snapshots", [], |r| {
            r.get(0)
        })
        .unwrap();
    let entry_max: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT MAX(length(entry_json)) FROM history_entries",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if entry_max > snapshot_len + 8 {
        let mid = StoreOptions {
            max_blob_bytes: snapshot_len + 8,
            ..fast()
        };
        assert_eq!(
            ProjectStore::open(&path, &mid).unwrap_err().code,
            StoreErrorCode::ProjectCorrupted
        );
    }
}

#[test]
fn identifiers_with_hostile_content_are_stored_and_returned_verbatim() {
    // ids/labels arbitrários (aspas, SQL, unicode, controle) passam por parâmetros, nunca por concatenação
    let dir = TempDir::new("hostile");
    let path = dir.file("p.capia");
    let mut e = create(&path, &fast());
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    let nasty = "x'); DROP TABLE history_entries; --\u{0}\u{202e}ç\n\"quoted\"";
    let t = capia_commands::Transaction {
        transaction_id: Some(nasty.into()),
        label: nasty.into(),
        base_revision: None,
        commands: vec![insert(nasty, "V1", 0, nasty, 10)],
        max_ops: None,
    };
    let r = e.execute(&user(), t.clone(), 2);
    // o id de operação pode ser recusado por validação (≤128 chars sempre é; o NUL é aceito pelo SQLite como texto)
    let r = r.unwrap();
    drop(e);
    let e2 = open(&path, &fast());
    assert_eq!(e2.history().last().unwrap().label, nasty);
    assert!(e2.applied_operation(nasty).is_some());
    assert_eq!(count(&path, "history_entries"), 2, "no table was dropped");
    let (store, _) = ProjectStore::open(&path, &fast()).unwrap();
    assert_eq!(
        store.load_operation_result(nasty).unwrap().unwrap().result,
        r
    );
    // reenvio idempotente com o id hostil
    let mut e3 = open(&path, &fast());
    assert!(e3.execute(&user(), t, 3).unwrap().replayed);
}
