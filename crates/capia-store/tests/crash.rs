//! Testes de crash **reais**: um processo filho (este mesmo binário de teste) é encerrado de forma
//! abrupta — `process::abort` ou morto de fora com `kill` (SIGKILL/TerminateProcess) — antes do
//! `BEGIN`, no meio da transação, antes do `COMMIT` e logo depois dele. O pai reabre o arquivo e
//! exige: `integrity_check` ok, documento == estado anterior COMPLETO **ou** estado novo COMPLETO
//! (nunca um intermediário), log de operações coerente e idempotência intacta.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::{Actor, Engine, EngineConfig};
use capia_store::{ProjectStore, StoreOptions, Synchronous};
use common::*;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const TARGET_OP: &str = "crash-op";

/// A transação alvo: dois clips numa transação (para provar atomicidade multi-comando).
fn target_tx() -> capia_commands::Transaction {
    tx(
        "target",
        vec![
            insert(TARGET_OP, "V1", 100, "t1", 10),
            insert("crash-op-2", "V2", 100, "t2", 10),
        ],
    )
}

fn loop_tx(i: i64) -> capia_commands::Transaction {
    tx(
        "loop",
        vec![insert(
            &format!("loop-{i}"),
            "V1",
            1000 + i * 20,
            &format!("k{i}"),
            10,
        )],
    )
}

fn child_opts() -> StoreOptions {
    let every = std::env::var("CAPIA_SNAPSHOT_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    StoreOptions {
        synchronous: Synchronous::Full,
        snapshot_every: every,
        ..StoreOptions::default()
    }
}

/// Ponto de entrada do processo filho. Em execução normal (sem a variável) é um no-op.
#[test]
fn crash_child() {
    let Ok(scenario) = std::env::var("CAPIA_CRASH_CHILD") else {
        return;
    };
    let path = std::env::var("CAPIA_CRASH_PATH").unwrap();
    let mut e = ProjectStore::open_engine(
        Path::new(&path),
        &child_opts(),
        key(),
        EngineConfig::default(),
    )
    .unwrap();
    match scenario.as_str() {
        "one_commit" => {
            // com a feature `failpoints`, o processo morre/estaciona dentro desta chamada
            let r = e.execute(&user(), target_tx(), 5);
            println!("CHILD_RETURNED {}", r.is_ok());
        }
        "loop" => {
            for i in 0.. {
                e.execute(&user(), loop_tx(i), 5).unwrap();
                println!("COMMITTED {i}");
            }
        }
        "import" => {
            // mesmo mecanismo do `Project::import_asset`: efeito do catálogo na transação do commit
            let media = std::env::var("CAPIA_CRASH_MEDIA").unwrap();
            let (store, state) = ProjectStore::open(Path::new(&path), &child_opts()).unwrap();
            let pending = store.pending_catalog();
            let mut e = store
                .into_engine(state, key(), EngineConfig::default())
                .unwrap();
            let r = import_tx(Path::new(&media));
            pending.lock().unwrap().push(r.0);
            let out = e.execute(&user(), r.1, 5);
            println!("CHILD_RETURNED {}", out.is_ok());
        }
        "migrate" => {
            // abrir um projeto de schema 1 com este software migra para o 2 — o failpoint
            // `in_migration` mata o processo com o DDL aplicado e NADA commitado
            let opened = ProjectStore::open(Path::new(&path), &child_opts());
            println!("CHILD_RETURNED {}", opened.is_ok());
        }
        other => panic!("unknown scenario {other}"),
    }
}

/// Registro de catálogo + transação `register_asset` de um arquivo de mídia sintético.
fn import_tx(media: &Path) -> (capia_store::CatalogOp, capia_commands::Transaction) {
    let rec = capia_assets::prepare_import(media, None, &capia_assets::testing::StaticProbe, 1)
        .unwrap()
        .record;
    let asset = capia_model::Asset {
        id: rec.asset_id.clone(),
        name: rec.display_name.clone(),
        duration: rec.media.duration,
        has_video: true,
        has_audio: true,
        offline: false,
    };
    let t = tx(
        "import",
        vec![env(
            "import-op",
            capia_commands::Command::RegisterAsset { asset },
        )],
    );
    let op = capia_store::CatalogOp {
        record: rec,
        event: capia_store::CatalogEventKind::Import,
        detail: serde_json::json!({}),
        at_ms: 1,
    };
    (op, t)
}

fn spawn_child(
    path: &Path,
    scenario: &str,
    failpoint: Option<(&str, &str)>,
    snapshot_every: Option<u64>,
) -> Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env("CAPIA_CRASH_CHILD", scenario)
        .env("CAPIA_CRASH_PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some((name, mode)) = failpoint {
        cmd.env("CAPIA_FAILPOINT", name)
            .env("CAPIA_FAILPOINT_MODE", mode);
    }
    if let Some(m) = std::env::var_os("CAPIA_CRASH_MEDIA") {
        cmd.env("CAPIA_CRASH_MEDIA", m);
    }
    if let Some(n) = snapshot_every {
        cmd.env("CAPIA_SNAPSHOT_EVERY", n.to_string());
    }
    cmd.spawn().unwrap()
}

/// Lê o stdout do filho numa thread e entrega as linhas por canal (com timeout no consumidor).
fn lines_of(child: &mut Child) -> mpsc::Receiver<String> {
    let out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for(rx: &mpsc::Receiver<String>, needle: &str) -> String {
    loop {
        let line = rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|e| panic!("child did not print {needle:?}: {e}"));
        if line.contains(needle) {
            return line;
        }
    }
}

/// Projeto base: sequence S, tracks V1/V2 e um clip já commitado (estado "A").
fn base_project(path: &Path) {
    let mut e = create(path, &StoreOptions::default());
    e.execute(&user(), setup_tx("base"), 1).unwrap();
    e.execute(
        &user(),
        tx("seed", vec![insert("seed-op", "V1", 0, "seed", 10)]),
        2,
    )
    .unwrap();
}

/// Digest esperado após aplicar a transação alvo sobre o estado atual do arquivo (estado "B").
fn expected_after_target(path: &Path) -> (String, String) {
    let (_, state) = ProjectStore::open(path, &fast()).unwrap();
    let a = sem(&state.doc);
    let mut mem = Engine::new(state.doc, [0; 32]);
    mem.execute(&Actor::user("x"), target_tx(), 0).unwrap();
    (a, sem(mem.document()))
}

fn assert_file_is_sound(path: &Path) {
    let report = ProjectStore::validate_file(path);
    assert!(
        report.ok,
        "validate_file after the crash: {:?}",
        report.issues
    );
}

#[test]
fn killing_the_process_at_every_commit_stage_yields_exactly_the_old_or_the_new_state() {
    // (ponto de falha, o commit chegou ao disco?)
    let stages = [
        ("before_begin", false),
        ("in_tx_after_entry", false),
        ("in_tx_after_writes", false),
        ("before_commit", false),
        ("after_commit", true),
    ];
    for (stage, committed) in stages {
        for mode in ["abort", "park"] {
            let dir = TempDir::new("crash");
            let path = dir.file("p.capia");
            base_project(&path);
            let (state_a, state_b) = expected_after_target(&path);
            let mut child = spawn_child(&path, "one_commit", Some((stage, mode)), None);
            let status = if mode == "park" {
                let rx = lines_of(&mut child);
                wait_for(&rx, "FAILPOINT_REACHED");
                child.kill().unwrap(); // morto DE FORA, no meio da operação
                child.wait().unwrap()
            } else {
                child.wait().unwrap()
            };
            assert!(
                !status.success(),
                "{stage}/{mode}: the child must die abruptly, got {status}"
            );

            // ---- reabre o arquivo e prova o invariante A-ou-B ----
            assert_file_is_sound(&path);
            let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
            let got = sem(&state.doc);
            assert!(
                got == state_a || got == state_b,
                "{stage}/{mode}: intermediate state on disk!"
            );
            assert_eq!(
                got == state_b,
                committed,
                "{stage}/{mode}: wrong side of the commit"
            );
            // o log de operações acompanha o documento: nunca um sem o outro
            assert_eq!(
                store.has_operation_id(TARGET_OP).unwrap(),
                committed,
                "{stage}/{mode}"
            );
            assert_eq!(
                store.has_operation_id("crash-op-2").unwrap(),
                committed,
                "{stage}/{mode}"
            );
            let stats = store.stats().unwrap();
            assert_eq!(
                stats.history_entries,
                if committed { 3 } else { 2 },
                "{stage}/{mode}"
            );
            assert_eq!(
                stats.events,
                if committed { 3 } else { 2 },
                "{stage}/{mode}"
            );
            drop(store);

            // ---- idempotência depois da queda: reenviar a MESMA transação ----
            let mut e = open(&path, &fast());
            let r = e.execute(&user(), target_tx(), 9).unwrap();
            assert_eq!(
                r.replayed, committed,
                "{stage}/{mode}: replay iff it had been committed"
            );
            assert_eq!(
                sem(e.document()),
                state_b,
                "{stage}/{mode}: exactly one application"
            );
            assert_eq!(clip_ids(&e, "V1"), ["seed", "t1"]);
            drop(e);
            assert_file_is_sound(&path);
        }
    }
}

#[test]
fn killing_a_writer_at_random_moments_never_corrupts_or_half_applies_anything() {
    // loop de commits (fsync completo) interrompido por SIGKILL/TerminateProcess em instantes
    // variados — inclui commits que escrevem snapshots (snapshot_every pequeno)
    for round in 0..14u64 {
        let dir = TempDir::new("killloop");
        let path = dir.file("p.capia");
        base_project(&path);
        let mut child = spawn_child(&path, "loop", None, Some(1 + round % 4));
        let rx = lines_of(&mut child);
        let mut printed = 0usize;
        let want = 2 + usize::try_from(round % 7).unwrap();
        while printed < want {
            wait_for(&rx, "COMMITTED");
            printed += 1;
        }
        std::thread::sleep(Duration::from_millis((round * 7) % 23));
        child.kill().unwrap();
        child.wait().unwrap();
        while let Ok(line) = rx.try_recv() {
            if line.contains("COMMITTED") {
                printed += 1;
            }
        }

        assert_file_is_sound(&path);
        let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
        drop(store);
        // 2 revisões do pai (setup, seed) + n commits do filho
        let n = usize::try_from(state.doc.revision).unwrap() - 2;
        assert!(
            n >= printed,
            "round {round}: a commit reported as done was lost ({n} < {printed})"
        );
        assert!(
            n <= printed + 1,
            "round {round}: more commits on disk than the child could have done"
        );
        // o documento é EXATAMENTE o replay dos n primeiros commits em memória
        let (_, base_state) = ProjectStore::open(&dir.file("p.capia"), &fast()).unwrap();
        let _ = base_state;
        let mut mem = Engine::new(capia_model::Document::new(), [0; 32]);
        mem.execute(&user(), setup_tx("base"), 0).unwrap();
        mem.execute(
            &user(),
            tx("seed", vec![insert("seed-op", "V1", 0, "seed", 10)]),
            0,
        )
        .unwrap();
        for i in 0..i64::try_from(n).unwrap() {
            mem.execute(&user(), loop_tx(i), 0).unwrap();
        }
        assert_eq!(
            sem(&state.doc),
            sem(mem.document()),
            "round {round}: document != replay of {n} commits"
        );
        // o undo gravado também funciona depois da queda
        let mut e = open(&path, &fast());
        assert_eq!(e.applied_history().len(), 2 + n);
        e.undo(&user(), 0).unwrap();
        assert_file_is_sound(&path);
    }
}

#[test]
fn a_crash_leaves_a_wal_that_the_next_open_recovers_and_cleans() {
    let dir = TempDir::new("walrec");
    let path = dir.file("p.capia");
    base_project(&path);
    let mut child = spawn_child(&path, "one_commit", Some(("after_commit", "park")), None);
    let rx = lines_of(&mut child);
    wait_for(&rx, "FAILPOINT_REACHED");
    child.kill().unwrap();
    child.wait().unwrap();
    // o commit está só no -wal (sem checkpoint): o arquivo principal sozinho NÃO o contém
    assert!(
        dir.file("p.capia-wal").exists(),
        "the killed writer left its WAL behind"
    );
    let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
    assert!(
        store.has_operation_id(TARGET_OP).unwrap(),
        "recovered from the WAL"
    );
    assert_eq!(state.doc.revision, 3);
    store.close().unwrap();
    assert!(
        !dir.file("p.capia-wal").exists(),
        "a clean close removes the sidecars"
    );
}

#[test]
fn every_failpoint_is_inert_without_the_environment() {
    // sem CAPIA_FAILPOINT o mesmo caminho de código completa normalmente
    let dir = TempDir::new("inert");
    let path = dir.file("p.capia");
    base_project(&path);
    let mut e = open(&path, &fast());
    let r = e.execute(&user(), target_tx(), 5).unwrap();
    assert!(!r.replayed);
    assert_file_is_sound(&path);
}

/// Import = documento + catálogo na MESMA transação: depois de um processo morto em qualquer ponto,
/// o asset existe nas duas camadas (e no log de operações) ou em nenhuma.
#[test]
fn killing_an_import_at_every_stage_is_all_or_nothing() {
    let stages = [
        ("before_begin", false),
        ("in_tx_after_entry", false),
        ("in_tx_after_writes", false),
        ("before_commit", false),
        ("after_commit", true),
    ];
    for (stage, committed) in stages {
        for mode in ["abort", "park"] {
            let dir = TempDir::new("crash-import");
            let path = dir.file("p.capia");
            base_project(&path);
            let media = dir.file("clip.mp4");
            std::fs::write(&media, b"synthetic media bytes").unwrap();
            // CAPIA_CRASH_MEDIA é lido pelo filho (herdado do ambiente do pai neste ponto)
            let mut cmd_env = Command::new(std::env::current_exe().unwrap());
            cmd_env
                .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
                .env("CAPIA_CRASH_CHILD", "import")
                .env("CAPIA_CRASH_PATH", &path)
                .env("CAPIA_CRASH_MEDIA", &media)
                .env("CAPIA_FAILPOINT", stage)
                .env("CAPIA_FAILPOINT_MODE", mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut child = cmd_env.spawn().unwrap();
            let status = if mode == "park" {
                let rx = lines_of(&mut child);
                wait_for(&rx, "FAILPOINT_REACHED");
                child.kill().unwrap();
                child.wait().unwrap()
            } else {
                child.wait().unwrap()
            };
            assert!(!status.success(), "{stage}/{mode}: must die abruptly");

            assert_file_is_sound(&path);
            let rec =
                capia_assets::prepare_import(&media, None, &capia_assets::testing::StaticProbe, 1)
                    .unwrap()
                    .record;
            let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
            let in_doc = state.doc.asset(&rec.asset_id).is_some();
            let in_log = store.has_operation_id("import-op").unwrap();
            let catalog =
                capia_store::Catalog::open(&path, std::time::Duration::from_secs(5)).unwrap();
            let in_catalog = catalog.get(&rec.asset_id).unwrap().is_some();
            let events = catalog.events(&rec.asset_id).unwrap().len();
            assert_eq!(
                (in_doc, in_log, in_catalog, events),
                (committed, committed, committed, usize::from(committed)),
                "{stage}/{mode}: document, operation log and catalog must agree"
            );
            drop(store);
        }
    }
}

/// Queda no MEIO da migration v1→v2 (DDL já executado, nada commitado): o arquivo continua um
/// projeto v1 íntegro, com o histórico completo; a abertura seguinte migra normalmente.
#[test]
fn killing_the_process_in_the_middle_of_a_migration_leaves_a_sound_v1_project() {
    for mode in ["abort", "park"] {
        let dir = TempDir::new("crash-migrate");
        let path = dir.file("p.capia");
        {
            let (store, state) = ProjectStore::create_at_schema(&path, &fast(), 1).unwrap();
            let mut e = store
                .into_engine(state, key(), EngineConfig::default())
                .unwrap();
            e.execute(&user(), setup_tx("base"), 1).unwrap();
            e.execute(
                &user(),
                tx("seed", vec![insert("seed-op", "V1", 0, "seed", 10)]),
                2,
            )
            .unwrap();
        }
        let before = ProjectStore::validate_file(&path);
        assert!(before.ok, "{:?}", before.issues);
        let digest = before.info.unwrap().digest;

        let mut child = spawn_child(&path, "migrate", Some(("in_migration", mode)), None);
        let status = if mode == "park" {
            let rx = lines_of(&mut child);
            wait_for(&rx, "FAILPOINT_REACHED");
            child.kill().unwrap();
            child.wait().unwrap()
        } else {
            child.wait().unwrap()
        };
        assert!(!status.success(), "{mode}: the child must die abruptly");

        // ainda é schema 1, sem tabelas do catálogo, íntegro e com o mesmo documento
        let c = rusqlite::Connection::open(&path).unwrap();
        let v: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(
            v, 1,
            "{mode}: the half-applied migration must not be visible"
        );
        let tables: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('media_assets','asset_events')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "{mode}");
        drop(c);
        let report = ProjectStore::validate_file(&path);
        assert!(report.ok, "{mode}: {:?}", report.issues);
        assert_eq!(report.info.unwrap().digest, digest, "{mode}");

        // a próxima abertura migra de verdade, com a história e a idempotência intactas
        let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
        assert_eq!(store.schema_version(), capia_store::CURRENT_SCHEMA_VERSION);
        assert_eq!(sem(&state.doc), {
            let mut d = state.doc.clone();
            d.revision = 0;
            capia_commands::document_digest(&d)
        });
        assert!(store.has_operation_id("seed-op").unwrap());
        assert_eq!(state.history.len(), 2);
    }
}
