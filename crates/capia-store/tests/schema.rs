//! Migrations explícitas, versões futuras e arquivos inválidos/corrompidos.
//! Regra: entrada externa nunca causa `panic`; o erro é estruturado e o arquivo não é alterado
//! quando é incompatível.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::hash::sha256_hex;
use capia_store::{CURRENT_SCHEMA_VERSION, MIGRATIONS, Migration, ProjectStore, StoreErrorCode};
use common::*;
use rusqlite::{Connection, Transaction};
use std::path::{Path, PathBuf};

/// Projeto REAL de schema 1 (o que a M06 produzia), com histórico e ramo de redo.
fn project_with_history(dir: &TempDir) -> PathBuf {
    let path = dir.file("p.capia");
    let (store, state) = ProjectStore::create_at_schema(&path, &fast(), 1).unwrap();
    assert_eq!(store.schema_version(), 1);
    let mut e = store
        .into_engine(state, key(), capia_commands::EngineConfig::default())
        .unwrap();
    e.execute(&user(), setup_tx("a"), 1).unwrap();
    e.execute(
        &user(),
        tx(
            "c",
            vec![
                insert("c1", "V1", 0, "a", 10),
                insert("c2", "V2", 4, "b", 8),
            ],
        ),
        2,
    )
    .unwrap();
    e.undo(&user(), 3).unwrap();
    drop(e);
    path
}

fn snapshot_of_files(dir: &TempDir) -> Vec<(String, String)> {
    let mut v: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.path().is_file())
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                sha256_hex(&std::fs::read(e.path()).unwrap()),
            )
        })
        .collect();
    v.sort();
    v
}

fn raw(path: &Path) -> Connection {
    let c = Connection::open(path).unwrap();
    // para fabricar corrupção/versões: sem checagens nem triggers de proteção
    c.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    c
}

// ---- migrations --------------------------------------------------------------------------------

fn m2_ok(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE extra (id INTEGER PRIMARY KEY NOT NULL) STRICT; INSERT INTO meta(key,value) VALUES ('m2','done');")
}

fn m2_fails_midway(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE half_done (id INTEGER) STRICT;")?;
    tx.execute_batch("THIS IS NOT SQL")
}

fn m3_fails(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE never (id INTEGER) STRICT; SELECT * FROM table_that_does_not_exist;",
    )
}

fn with(extra: &[Migration]) -> Vec<Migration> {
    let mut v = MIGRATIONS.to_vec();
    v.extend_from_slice(extra);
    v
}

#[test]
fn the_current_schema_is_two_and_migrations_are_contiguous() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 2);
    for (i, m) in MIGRATIONS.iter().enumerate() {
        assert_eq!(
            m.version,
            u32::try_from(i + 1).unwrap(),
            "migration versions must be 1..N"
        );
    }
    assert_eq!(MIGRATIONS.last().unwrap().version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn an_older_project_is_migrated_forward_with_a_backup_and_keeps_its_history() {
    let dir = TempDir::new("migrate");
    let path = project_with_history(&dir);
    let digest_before = ProjectStore::inspect(&path).unwrap().digest;
    let m3 = Migration {
        version: 3,
        name: "add extra table",
        up: m2_ok,
    };
    let list = with(&[m3]);
    // v1 → v2 (REAL, catálogo de mídia) → v3 (sintética), na mesma abertura
    let (store, state) = ProjectStore::open_with_migrations(&path, &fast(), &list, 3).unwrap();
    assert_eq!(store.schema_version(), 3);
    assert_eq!(
        capia_commands::document_digest(&state.doc),
        digest_before,
        "data survives the migration"
    );
    assert!(
        state.cursor < state.history.len(),
        "the redo branch survives too"
    );
    // operation_ids intactos (a idempotência durável atravessa a migration)
    for op in ["a-seq", "a-V1", "a-V2", "c1", "c2"] {
        assert!(
            store.has_operation_id(op).unwrap(),
            "{op} lost in the migration"
        );
    }
    store.close().unwrap();
    // e a pilha de undo/redo continua funcionando depois de migrar (1 aplicada + 1 ramo de redo)
    {
        let (store, state) = ProjectStore::open_with_migrations(&path, &fast(), &list, 3).unwrap();
        let mut e = store
            .into_engine(state, key(), capia_commands::EngineConfig::default())
            .unwrap();
        assert_eq!(
            e.applied_history().len(),
            1,
            "history survived the migration"
        );
        e.redo(&user(), 9).unwrap();
        assert_eq!(e.applied_history().len(), 2);
        e.undo(&user(), 9).unwrap();
        e.undo(&user(), 9).unwrap();
        assert_eq!(e.applied_history().len(), 0);
    }

    let c = Connection::open(&path).unwrap();
    let v: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 3);
    let versions: Vec<i64> = c
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(versions, [1, 2, 3]);
    // a migration real criou o catálogo COMPLETO (vazio: nada do projeto v1 foi tocado)
    let media: i64 = c
        .query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(media, 0);
    let objects: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE name IN ('media_assets','asset_events','media_assets_content_hash','asset_events_asset','asset_events_no_update','asset_events_no_delete') ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        objects.len(),
        6,
        "table, index and append-only triggers: {objects:?}"
    );
    // o catálogo nasce com as proteções: hash único e trilha de eventos append-only
    c.execute("INSERT INTO media_assets VALUES ('a','video','sha256:0000000000000000000000000000000000000000000000000000000000000000',1,'n','{}','[]','{}','online',0,0)", []).unwrap();
    assert!(c.execute("INSERT INTO media_assets VALUES ('b','video','sha256:0000000000000000000000000000000000000000000000000000000000000000',1,'n','{}','[]','{}','online',0,0)", []).is_err(), "unique content hash");
    c.execute(
        "INSERT INTO asset_events(asset_id,kind,detail_json,at_ms) VALUES ('a','import','{}',0)",
        [],
    )
    .unwrap();
    assert!(
        c.execute("UPDATE asset_events SET kind='verify'", [])
            .is_err(),
        "append-only"
    );
    assert!(
        c.execute("DELETE FROM asset_events", []).is_err(),
        "append-only"
    );
    c.execute("DELETE FROM media_assets", []).ok();
    let marker: String = c
        .query_row("SELECT value FROM meta WHERE key='m2'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(marker, "done");

    // o backup pré-migração é um projeto v1 íntegro e abre com o software "antigo"
    let backup = dir.file("p.capia.v1.bak");
    assert!(backup.exists(), "backup is mandatory before migrating");
    // (o software "antigo" é simulado com a lista de migrations só até a versão 1)
    let (old, old_state) =
        ProjectStore::open_with_migrations(&backup, &fast(), &MIGRATIONS[..1], 1).unwrap();
    assert_eq!(old.schema_version(), 1);
    assert_eq!(
        capia_commands::document_digest(&old_state.doc),
        digest_before
    );
}

#[test]
fn a_failing_migration_rolls_back_and_leaves_the_project_usable() {
    let dir = TempDir::new("migfail");
    let path = project_with_history(&dir);
    let list = with(&[Migration {
        version: 3,
        name: "broken",
        up: m2_fails_midway,
    }]);
    let err = ProjectStore::open_with_migrations(&path, &fast(), &list, 3).unwrap_err();
    assert_eq!(err.code, StoreErrorCode::MigrationFailed);
    assert_eq!(err.details.as_ref().unwrap()["failed_version"], 3);
    assert!(
        err.cause.is_some(),
        "the SQLite cause is kept for diagnosis"
    );
    // nada ficou pela metade: a migration real (2) ficou aplicada, a falha (3) foi desfeita
    let c = Connection::open(&path).unwrap();
    let v: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 2);
    let tables: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='half_done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    drop(c);
    assert!(
        ProjectStore::open(&path, &fast()).is_ok(),
        "the software at schema 2 still opens it"
    );
    assert!(dir.file("p.capia.v1.bak").exists());
}

#[test]
fn each_migration_is_its_own_transaction() {
    let dir = TempDir::new("migsteps");
    let path = project_with_history(&dir);
    let list = with(&[
        Migration {
            version: 3,
            name: "ok",
            up: m2_ok,
        },
        Migration {
            version: 4,
            name: "broken",
            up: m3_fails,
        },
    ]);
    let err = ProjectStore::open_with_migrations(&path, &fast(), &list, 4).unwrap_err();
    assert_eq!(err.code, StoreErrorCode::MigrationFailed);
    assert_eq!(
        err.details.as_ref().unwrap()["applied"],
        serde_json::json!([2, 3])
    );
    let c = Connection::open(&path).unwrap();
    let v: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 3, "migrations 2 and 3 stay applied; only 4 rolled back");
    let never: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='never'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(never, 0);
}

#[test]
fn a_gap_in_the_migration_list_is_refused() {
    let dir = TempDir::new("migrgap");
    let path = project_with_history(&dir);
    let list = with(&[Migration {
        version: 4,
        name: "skips 3",
        up: m2_ok,
    }]);
    let err = ProjectStore::open_with_migrations(&path, &fast(), &list, 4).unwrap_err();
    assert_eq!(err.code, StoreErrorCode::MigrationFailed);
    let v: i64 = Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
}

#[test]
fn a_newer_schema_is_rejected_and_the_file_is_not_touched() {
    let dir = TempDir::new("future");
    let path = project_with_history(&dir);
    {
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
    }
    let before = snapshot_of_files(&dir);
    for result in [
        ProjectStore::open(&path, &fast()).map(|_| ()),
        ProjectStore::inspect(&path).map(|_| ()),
    ] {
        let e = result.unwrap_err();
        assert_eq!(e.code, StoreErrorCode::UnsupportedSchemaVersion);
        assert_eq!(e.details.as_ref().unwrap()["found"], 99);
        assert_eq!(e.details.as_ref().unwrap()["supported"], 2);
    }
    let report = ProjectStore::validate_file(&path);
    assert!(!report.ok);
    assert_eq!(
        report.issues[0].code,
        StoreErrorCode::UnsupportedSchemaVersion
    );
    assert_eq!(
        snapshot_of_files(&dir),
        before,
        "no byte changed, no sidecar created"
    );
}

#[test]
fn a_project_migrated_by_a_newer_build_is_refused_by_the_older_one() {
    let dir = TempDir::new("downgrade");
    let path = project_with_history(&dir);
    let list = with(&[Migration {
        version: 3,
        name: "add extra table",
        up: m2_ok,
    }]);
    ProjectStore::open_with_migrations(&path, &fast(), &list, 3)
        .unwrap()
        .0
        .close()
        .unwrap();
    let before = snapshot_of_files(&dir);
    let e = ProjectStore::open(&path, &fast()).unwrap_err();
    assert_eq!(e.code, StoreErrorCode::UnsupportedSchemaVersion);
    assert_eq!(snapshot_of_files(&dir), before);
}

// ---- arquivos que não são projetos -----------------------------------------------------------------

#[test]
fn non_projects_are_rejected_with_structured_errors_and_never_modified() {
    let dir = TempDir::new("nonproj");
    let missing = dir.file("missing.capia");
    assert_eq!(
        ProjectStore::open(&missing, &fast()).unwrap_err().code,
        StoreErrorCode::ProjectNotFound
    );

    let empty = dir.file("empty.capia");
    std::fs::write(&empty, b"").unwrap();
    let text = dir.file("text.capia");
    std::fs::write(
        &text,
        b"hello, this is definitely not a database\n".repeat(200),
    )
    .unwrap();
    let noise = dir.file("noise.capia");
    std::fs::write(
        &noise,
        (0..70_000u32)
            .map(|i| u8::try_from((i * 7919 + 13) % 251).unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let plain_sqlite = dir.file("plain.capia");
    {
        let c = Connection::open(&plain_sqlite).unwrap();
        c.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT); INSERT INTO notes(body) VALUES ('hi');").unwrap();
    }
    let other_app = dir.file("other.capia");
    {
        let c = Connection::open(&other_app).unwrap();
        c.execute_batch(
            "CREATE TABLE t (x); PRAGMA application_id = 12345; PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let no_version = dir.file("noversion.capia");
    {
        let c = Connection::open(&no_version).unwrap();
        c.execute_batch(&format!(
            "CREATE TABLE t (x); PRAGMA application_id = {};",
            capia_store::APPLICATION_ID
        ))
        .unwrap();
    }
    let a_dir = dir.file("folder.capia");
    std::fs::create_dir(&a_dir).unwrap();

    let before = snapshot_of_files(&dir);
    for (path, want) in [
        (&empty, StoreErrorCode::NotACapiaProject),
        (&text, StoreErrorCode::NotACapiaProject),
        (&noise, StoreErrorCode::NotACapiaProject),
        (&plain_sqlite, StoreErrorCode::NotACapiaProject),
        (&other_app, StoreErrorCode::NotACapiaProject),
        (&no_version, StoreErrorCode::ProjectCorrupted),
        (&a_dir, StoreErrorCode::NotACapiaProject),
    ] {
        let e = ProjectStore::open(path, &fast()).unwrap_err();
        assert_eq!(e.code, want, "{}: {e}", path.display());
        assert_eq!(ProjectStore::inspect(path).unwrap_err().code, want);
        let report = ProjectStore::validate_file(path);
        assert!(!report.ok && report.issues[0].code == want);
    }
    // nem o `open` nem o `validate` criam schema em arquivo alheio (nem arquivos auxiliares)
    let after: Vec<_> = snapshot_of_files(&dir)
        .into_iter()
        .filter(|(n, _)| n != "folder.capia")
        .collect();
    assert_eq!(
        after,
        before
            .into_iter()
            .filter(|(n, _)| n != "folder.capia")
            .collect::<Vec<_>>()
    );
    // criar em cima de um arquivo existente também é recusado
    assert_eq!(
        ProjectStore::create(&plain_sqlite, &fast())
            .unwrap_err()
            .code,
        StoreErrorCode::ProjectAlreadyExists
    );
}

// ---- corrupção dentro de um projeto CapIA ---------------------------------------------------------

fn expect_corrupted(path: &Path, what: &str) {
    let e = match ProjectStore::open(path, &fast()) {
        Err(e) => e,
        Ok(_) => panic!("{what}: open should have failed"),
    };
    assert_eq!(e.code, StoreErrorCode::ProjectCorrupted, "{what}: {e}");
    let report = ProjectStore::validate_file(path);
    assert!(!report.ok, "{what}: validate_file must flag it");
    assert!(
        report
            .issues
            .iter()
            .all(|i| i.code == StoreErrorCode::ProjectCorrupted),
        "{what}: {:?}",
        report.issues
    );
}

#[test]
fn corrupted_blobs_and_inconsistent_rows_are_detected() {
    type Tamper = fn(&Connection);
    let cases: &[(&str, Tamper)] = &[
        ("snapshot is not json", |c| {
            c.execute("UPDATE snapshots SET document_json = '{not json'", [])
                .unwrap();
        }),
        ("snapshot has the wrong shape", |c| {
            c.execute("UPDATE snapshots SET document_json = '{\"a\":1}'", [])
                .unwrap();
        }),
        ("snapshot digest mismatch", |c| {
            c.execute("UPDATE snapshots SET digest = 'deadbeef'", [])
                .unwrap();
        }),
        ("snapshot revision mismatch", |c| {
            c.execute("UPDATE snapshots SET revision = revision + 5", [])
                .unwrap();
        }),
        ("history entry fails its checksum", |c| {
            c.execute_batch("DROP TRIGGER history_entries_no_update; UPDATE history_entries SET entry_json = replace(entry_json, '\"label\":\"c\"', '\"label\":\"x\"') WHERE id = 2;").unwrap();
        }),
        ("history entry is not json", |c| {
            c.execute_batch("DROP TRIGGER history_entries_no_update; UPDATE history_entries SET entry_json = 'garbage', entry_sha256 = '" .to_owned().as_str()).ok();
            c.execute_batch(&format!(
                "UPDATE history_entries SET entry_json = 'garbage', entry_sha256 = '{}' WHERE id = 2;",
                sha256_hex(b"garbage")
            ))
            .unwrap();
        }),
        ("commit result fails its checksum", |c| {
            c.execute_batch("DROP TRIGGER commit_results_no_update; UPDATE commit_results SET result_json = replace(result_json, '\"replayed\":false', '\"replayed\":true') WHERE entry_id = 1;").unwrap();
        }),
        ("event sequence has a gap", |c| {
            c.execute_batch("DROP TRIGGER events_no_delete; DELETE FROM events WHERE seq = 2;")
                .unwrap();
        }),
        ("event references a missing entry", |c| {
            c.execute("INSERT INTO events(seq, kind, entry_id, revision_after, timestamp_ms, actor_json) VALUES (99, 'commit', 4242, 99, 0, '{\"kind\":\"user\",\"id\":\"x\"}')", []).unwrap();
        }),
        ("redo event that contradicts the stack", |c| {
            // revisão contígua, mas o undo aponta para uma entrada que não está no topo da pilha
            c.execute("INSERT INTO events(seq, kind, entry_id, revision_after, timestamp_ms, actor_json) VALUES (4, 'redo', 1, 4, 0, '{\"kind\":\"user\",\"id\":\"x\"}')", []).unwrap();
        }),
        ("event with a negative entry id", |c| {
            c.execute_batch(
                "DROP TRIGGER events_no_update; UPDATE events SET entry_id = -1 WHERE seq = 3;",
            )
            .unwrap();
        }),
        ("operation points to a missing result", |c| {
            c.execute_batch("INSERT INTO history_entries(id, revision_before, revision_after, label, actor_json, timestamp_ms, affected_json, entry_json, entry_sha256) VALUES (77, 0, 1, 'x', '{}', 0, '[]', '{}', 'x'); INSERT INTO applied_operations(operation_id, payload_hash, entry_id, applied_at_ms, actor_json) VALUES ('orphan', 'h', 77, 0, '{\"kind\":\"user\",\"id\":\"x\"}');").unwrap();
        }),
    ];
    for (what, tamper) in cases {
        let dir = TempDir::new("corrupt");
        let path = project_with_history(&dir);
        {
            let c = raw(&path);
            tamper(&c);
        }
        expect_corrupted(&path, what);
    }
}

#[test]
fn a_stored_document_that_breaks_an_invariant_is_rejected() {
    let dir = TempDir::new("invariant");
    let path = project_with_history(&dir);
    // adultera o snapshot para sobrepor dois clips da mesma track, recalculando o digest
    let (text, rev): (String, i64) = {
        let c = Connection::open(&path).unwrap();
        c.query_row("SELECT document_json, revision FROM snapshots", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
    };
    let mut doc: capia_model::Document = serde_json::from_str(&text).unwrap();
    let _ = rev;
    // monta um documento válido de saída: sequence S com dois clips sobrepostos via JSON
    let mut v = serde_json::to_value(&doc).unwrap();
    let clips = serde_json::json!({
        "k1": {"id":"k1","track":"V1","start":0,"duration":23520000,"content":{"type":"solid","color":"#000"}},
        "k2": {"id":"k2","track":"V1","start":0,"duration":23520000,"content":{"type":"solid","color":"#000"}},
    });
    if v["sequences"].is_null()
        || v["sequences"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
    {
        // snapshot 0 é vazio: cria a sequence manualmente
        v["sequences"] = serde_json::json!({"S": {
            "header": {"name":"S","frame_rate":"30","sample_rate":48000},
            "tracks": [{"id":"V1","kind":"visual"}],
            "clips": clips,
        }});
    }
    doc = serde_json::from_value(v).unwrap();
    let new_text = capia_commands::hash::canonical_json(&doc);
    let digest = capia_commands::document_digest(&doc);
    {
        let c = raw(&path);
        c.execute(
            "UPDATE snapshots SET document_json = ?1, digest = ?2",
            rusqlite::params![new_text, digest],
        )
        .unwrap();
    }
    expect_corrupted(&path, "document with overlapping clips");
}

#[test]
fn random_damage_never_panics_and_never_silently_changes_the_document() {
    let dir = TempDir::new("fuzz");
    let path = project_with_history(&dir);
    let original = std::fs::read(&path).unwrap();
    let want = ProjectStore::inspect(&path).unwrap().digest;
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let (mut errors, mut clean) = (0, 0);
    for round in 0..300 {
        let mut bytes = original.clone();
        match round % 3 {
            0 => {
                let i = usize::try_from(next()).unwrap() % bytes.len();
                bytes[i] ^= 1 << (next() % 8);
            }
            1 => {
                let cut = usize::try_from(next()).unwrap() % bytes.len();
                bytes.truncate(cut);
            }
            _ => {
                let i = usize::try_from(next()).unwrap() % bytes.len();
                let n = (usize::try_from(next()).unwrap() % 64).min(bytes.len() - i);
                for b in &mut bytes[i..i + n] {
                    *b = u8::try_from(next() % 256).unwrap();
                }
            }
        }
        let victim = dir.file(&format!("v{round}.capia"));
        std::fs::write(&victim, &bytes).unwrap();
        match ProjectStore::open(&victim, &fast()) {
            Err(e) => {
                errors += 1;
                assert!(
                    e.code != StoreErrorCode::TransactionFailed || e.cause.is_some(),
                    "round {round}: {e}"
                );
            }
            Ok((store, state)) => {
                // abriu: então os dados que importam têm que ser idênticos (dano em página livre/slack)
                let got = capia_commands::document_digest(&state.doc);
                assert_eq!(got, want, "round {round}: opened with a DIFFERENT document");
                clean += 1;
                drop(store);
            }
        }
        let _ = ProjectStore::validate_file(&victim);
        let _ = ProjectStore::inspect(&victim);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.file(&format!("v{round}.capia{suffix}")));
        }
    }
    assert!(
        errors > 50,
        "the damage should usually be detected ({errors} errors, {clean} clean)"
    );
}
