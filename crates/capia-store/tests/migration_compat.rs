//! Compatibilidade de migração (Fase 6, Track D-1): projetos COM DADOS criados em cada schema
//! 1..=atual (Fase 2/3 ≤3, Fase 4 = 4, Fase 5 = 5) abrem no schema atual sem perda de dados;
//! a migração é só para frente; o backup pré-migração é um projeto íntegro no schema antigo; e uma
//! migração que falha (em qualquer degrau) deixa o arquivo na última versão concluída, recuperável.
//! Complementa `schema.rs` (que usa projetos sem linhas nas tabelas novas) e `crash.rs` (kill no
//! meio da migração): aqui o que se prova é "nenhuma linha se perde".

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::hash::sha256_hex;
use capia_store::{
    AutonomyStore, CURRENT_SCHEMA_VERSION, MIGRATIONS, Migration, ProjectStore, StoreErrorCode,
};
use common::*;
use rusqlite::types::Value;
use rusqlite::{Connection, Transaction, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

type Rows = Vec<Vec<String>>;
/// tabela → (colunas, linhas)
type Dump = BTreeMap<String, (Vec<String>, Rows)>;

fn raw(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

fn tables(c: &Connection) -> Vec<String> {
    c.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .unwrap()
    .query_map([], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn columns(c: &Connection, table: &str) -> Vec<String> {
    c.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn rows_of(c: &Connection, table: &str, cols: &[String]) -> Rows {
    let sql = format!("SELECT {} FROM {table} ORDER BY rowid", cols.join(", "));
    let mut st = c.prepare(&sql).unwrap();
    let n = cols.len();
    st.query_map([], |r| {
        (0..n)
            .map(|i| r.get::<_, Value>(i).map(|v| format!("{v:?}")))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Todas as linhas de todas as tabelas (menos a trilha de migrações, comparada à parte).
fn dump(path: &Path) -> Dump {
    let c = raw(path);
    tables(&c)
        .into_iter()
        .filter(|t| t != "schema_migrations")
        .map(|t| {
            let cols = columns(&c, &t);
            let rows = rows_of(&c, &t, &cols);
            (t, (cols, rows))
        })
        .collect()
}

/// `after` contém TUDO o que `before` tinha (mesmas linhas, nas colunas que já existiam).
fn assert_nothing_lost(before: &Dump, path: &Path, what: &str) {
    let c = raw(path);
    for (table, (cols, rows)) in before {
        let now = rows_of(&c, table, cols);
        assert_eq!(&now, rows, "{what}: table `{table}` lost or changed data");
    }
}

fn user_version(path: &Path) -> i64 {
    raw(path)
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn applied_versions(path: &Path) -> Vec<i64> {
    raw(path)
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn file_hash(path: &Path) -> String {
    sha256_hex(&std::fs::read(path).unwrap())
}

/// Linhas reais em TODAS as tabelas que existem no schema `v` (índice de dados = o que o produto
/// daquela fase gravava). Só SQL de DML: o DDL é o do próprio `create_at_schema`.
fn seed(path: &Path, v: u32) {
    let c = raw(path);
    let hash = |n: u8| format!("sha256:{}", format!("{n:02x}").repeat(32));
    if v >= 2 {
        let info = serde_json::to_string(&capia_assets::testing::synthetic_info(
            capia_media::MediaKind::Video,
            10,
        ))
        .unwrap();
        let loc = serde_json::to_string(&capia_assets::AssetLocation::from_path(
            Path::new("/m/x.mp4"),
            None,
        ))
        .unwrap();
        for (i, id) in ["ast_a", "ast_b"].into_iter().enumerate() {
            c.execute(
                "INSERT INTO media_assets(asset_id, kind, content_hash, size_bytes, display_name, location_json, known_paths_json, media_info_json, status, status_checked_ms, imported_ms) \
                 VALUES (?1, 'video', ?2, ?3, ?4, ?5, '[\"/old/x.mp4\"]', ?6, 'online', 5, 6)",
                params![id, hash(i as u8 + 1), 1000 + i as i64, format!("{id}.mp4"), loc, info],
            )
            .unwrap();
            c.execute(
                "INSERT INTO asset_events(asset_id, kind, detail_json, at_ms) VALUES (?1, 'import', '{\"n\":1}', 7)",
                [id],
            )
            .unwrap();
        }
    }
    if v >= 3 {
        c.execute(
            "UPDATE media_assets SET fingerprint = 'fp1:0011223344556677' WHERE asset_id = 'ast_a'",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO jobs(job_id, kind, priority, state, progress_done, progress_total, dedup_key, label, params_json, result_json, error_json, cancel_requested, created_ms, started_ms, finished_ms, updated_ms) \
             VALUES ('job1','index','normal','completed',3,3,'k1','idx','{}','{\"ok\":true}',NULL,0,1,2,3,3)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO import_tickets(ticket_id, path, size_bytes, fingerprint, state, job_id, asset_id, outcome_json, error_json, created_ms, updated_ms) \
             VALUES ('t1','/m/x.mp4',1000,'fp1:0011223344556677','finalized','job1','ast_a','{}',NULL,1,2)",
            [],
        )
        .unwrap();
    }
    if v >= 4 {
        for ver in 0..2 {
            c.execute(
                "INSERT INTO ai_records(kind, id, version, parent, schema_version, created_ms, updated_ms, json) VALUES ('demand_spec','d1',?1,NULL,1,1,2,'{\"brief\":\"x\"}')",
                [ver],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO ai_usage(request_id, task_id, provider_id, endpoint_id, model_id, capability, purpose, input_tokens, output_tokens, cached_tokens, synthetic, cost_known, cost_micros, currency, latency_ms, attempt, status, error_code, pricing_date, timestamp_ms) \
             VALUES ('rq1','tk1','replay','e','m','text','plan',10,20,0,1,1,500,'USD',12,0,'ok',NULL,NULL,9)",
            [],
        )
        .unwrap();
    }
    if v >= 5 {
        for r in ["run1", "run2"] {
            c.execute(
                "INSERT INTO ai_runs(run_id, status, stage, revision, parent_run_id, variant_group, created_ms, updated_ms, json) VALUES (?1,'completed','deliver',3,NULL,'g1',1,2,'{\"goal\":\"x\"}')",
                [r],
            )
            .unwrap();
            c.execute(
                "INSERT INTO ai_run_stages(run_id, seq, stage, attempt, idem_key, input_digest, output_digest, status, started_ms, ended_ms, json) VALUES (?1,1,'plan',0,?2,'i','o','completed',1,2,'{}')",
                params![r, format!("{r}:plan:0")],
            )
            .unwrap();
            c.execute(
                "INSERT INTO ai_run_events(run_id, seq, kind, ts_ms, json) VALUES (?1,1,'stage_started',1,'{}')",
                [r],
            )
            .unwrap();
            c.execute(
                "INSERT INTO ai_budget_ledger(run_id, reservation_id, kind, micros, ts_ms, json) VALUES (?1,'res1','reserve',500,1,'{}')",
                [r],
            )
            .unwrap();
        }
        c.execute("INSERT INTO ai_side_effects(effect_key, run_id, kind, state, external_id, created_ms, updated_ms, json) VALUES ('llm:run1:0','run1','llm','done','ext1',1,2,'{}')", []).unwrap();
        c.execute("INSERT INTO ai_provenance(asset_id, run_id, kind, content_hash, created_ms, json) VALUES ('ast_a','run1','generated','sha256:aa',1,'{}')", []).unwrap();
        c.execute("INSERT INTO ai_memory(id, scope, status, created_ms, updated_ms, json) VALUES ('m1','project','active',1,2,'{}')", []).unwrap();
        c.execute("INSERT INTO ai_memory_log(memory_id, event, actor, run_id, ts_ms, json) VALUES ('m1','proposed','run','run1',1,'{}')", []).unwrap();
    }
}

/// Projeto REAL no schema `v` (histórico com ramo de redo + dados de todas as tabelas daquela fase).
fn project_at(dir: &TempDir, v: u32) -> (PathBuf, String, usize) {
    let path = dir.file("p.capia");
    let (store, state) = ProjectStore::create_at_schema(&path, &fast(), v).unwrap();
    assert_eq!(store.schema_version(), v);
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
    let digest = capia_commands::document_digest(e.document());
    let history = e.history().len();
    drop(e);
    seed(&path, v);
    (path, digest, history)
}

fn backup_of(path: &Path, v: u32) -> PathBuf {
    let mut n = path.file_name().unwrap().to_os_string();
    n.push(format!(".v{v}.bak"));
    path.with_file_name(n)
}

fn files(dir: &TempDir) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with("-wal") && !n.ends_with("-shm"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_schema_version_migrates_forward_without_losing_a_single_row() {
    assert_eq!(
        CURRENT_SCHEMA_VERSION, 5,
        "add the new version to this matrix"
    );
    for v in 1..=CURRENT_SCHEMA_VERSION {
        let dir = TempDir::new(&format!("compat-v{v}"));
        let (path, digest, history) = project_at(&dir, v);
        let before = dump(&path);
        let migrations_before = rows_of(
            &raw(&path),
            "schema_migrations",
            &columns(&raw(&path), "schema_migrations"),
        );
        assert_eq!(user_version(&path), i64::from(v));
        // sanidade da semente: as tabelas daquela fase têm linhas de verdade (teste não vazio)
        let seeded: usize = before
            .iter()
            .filter(|(t, _)| {
                matches!(
                    t.as_str(),
                    "media_assets" | "jobs" | "ai_records" | "ai_runs" | "ai_memory"
                )
            })
            .map(|(_, (_, rows))| rows.len())
            .sum();
        assert_eq!(seeded > 0, v >= 2, "v{v}: seeded rows");

        // `validate`/`inspect` são não destrutivos e valem para QUALQUER schema suportado, antes de
        // migrar (regressão corrigida: no schema 2 a leitura do catálogo pedia a coluna do schema 3)
        let h0 = file_hash(&path);
        let report = ProjectStore::validate_file(&path);
        assert!(
            report.ok,
            "v{v}: validate before migrating: {:?}",
            report.issues
        );
        let info = ProjectStore::inspect(&path).unwrap();
        assert_eq!(
            (info.schema_version, info.needs_migration),
            (v, v < CURRENT_SCHEMA_VERSION)
        );
        assert_eq!(
            file_hash(&path),
            h0,
            "v{v}: validate/inspect must not modify the file"
        );

        let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
        assert_eq!(store.schema_version(), CURRENT_SCHEMA_VERSION, "v{v}");
        assert_eq!(
            capia_commands::document_digest(&state.doc),
            digest,
            "v{v}: document"
        );
        assert_eq!(state.history.len(), history, "v{v}: history entries");
        assert!(state.cursor < state.history.len(), "v{v}: redo branch kept");
        for op in ["a-seq", "a-V1", "a-V2", "c1", "c2"] {
            assert!(store.has_operation_id(op).unwrap(), "v{v}: {op} lost");
        }
        store.close().unwrap();

        // nenhuma linha se perdeu, em NENHUMA tabela (inclusive as de fases posteriores à de origem)
        assert_nothing_lost(&before, &path, &format!("v{v}→{CURRENT_SCHEMA_VERSION}"));
        let after = dump(&path);
        for (t, (_, rows)) in &after {
            if !before.contains_key(t) {
                assert!(rows.is_empty(), "v{v}: new table `{t}` must start empty");
            }
        }
        assert_eq!(user_version(&path), i64::from(CURRENT_SCHEMA_VERSION));
        assert_eq!(
            applied_versions(&path),
            (1..=i64::from(CURRENT_SCHEMA_VERSION)).collect::<Vec<_>>(),
            "v{v}: contiguous migration trail"
        );
        // a trilha antiga é preservada linha a linha (nome e instante)
        let trail = rows_of(
            &raw(&path),
            "schema_migrations",
            &columns(&raw(&path), "schema_migrations"),
        );
        assert_eq!(
            &trail[..migrations_before.len()],
            &migrations_before[..],
            "v{v}"
        );

        // backup pré-migração: só existe se houve migração; é um projeto íntegro NO schema antigo
        let bak = backup_of(&path, v);
        if v < CURRENT_SCHEMA_VERSION {
            assert!(bak.exists(), "v{v}: backup before migrating is mandatory");
            assert_eq!(
                user_version(&bak),
                i64::from(v),
                "v{v}: backup keeps its schema"
            );
            assert_nothing_lost(&before, &bak, &format!("v{v} backup"));
            let (old, old_state) =
                ProjectStore::open_with_migrations(&bak, &fast(), &MIGRATIONS[..v as usize], v)
                    .unwrap();
            assert_eq!(old.schema_version(), v);
            assert_eq!(capia_commands::document_digest(&old_state.doc), digest);
        } else {
            assert!(!bak.exists(), "v{v}: nothing to migrate, no backup");
        }

        // só para frente: o software "antigo" recusa o arquivo migrado e NÃO o toca
        if v < CURRENT_SCHEMA_VERSION {
            let h = file_hash(&path);
            let err =
                ProjectStore::open_with_migrations(&path, &fast(), &MIGRATIONS[..v as usize], v)
                    .unwrap_err();
            assert_eq!(err.code, StoreErrorCode::UnsupportedSchemaVersion, "v{v}");
            assert_eq!(
                file_hash(&path),
                h,
                "v{v}: refused open must not modify the file"
            );
        }

        // reabrir é idempotente: sem novo backup, sem mudança nos dados
        let listing = files(&dir);
        let (again, _) = ProjectStore::open(&path, &fast()).unwrap();
        again.close().unwrap();
        assert_eq!(files(&dir), listing, "v{v}: re-open must not create files");
        assert_nothing_lost(&before, &path, &format!("v{v} second open"));

        // o projeto migrado é plenamente utilizável: commit/undo e o armazém de autonomia
        let mut e = open(&path, &fast());
        e.execute(
            &user(),
            tx("post", vec![insert("post1", "V1", 40, "pc", 5)]),
            9,
        )
        .unwrap();
        assert!(clip_ids(&e, "V1").contains(&"pc".to_owned()));
        e.undo(&user(), 10).unwrap();
        drop(e);
        let runs = AutonomyStore::open(&path, Duration::from_secs(2)).unwrap();
        let expected_runs = if v >= 5 { 2 } else { 0 };
        assert_eq!(
            runs.list_runs(None, 10).unwrap().len(),
            expected_runs,
            "v{v}"
        );
        runs.create_run(
            "new_run",
            "pending",
            "understand",
            None,
            None,
            &serde_json::json!({}),
            1,
        )
        .unwrap();
        assert!(ProjectStore::validate_file(&path).ok, "v{v}: validate_file");
    }
}

fn fails_midway(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE half_done (id INTEGER) STRICT;")?;
    tx.execute_batch("THIS IS NOT SQL")
}

/// Lista real de migrações com o degrau `k` quebrado (DDL aplicado, depois erro).
fn with_broken(k: u32) -> Vec<Migration> {
    let mut list = MIGRATIONS.to_vec();
    list[k as usize - 1] = Migration {
        version: k,
        name: "broken",
        up: fails_midway,
    };
    list
}

#[test]
fn a_migration_failing_at_any_step_leaves_the_last_good_version_and_is_recoverable() {
    // origem v1 (a mais antiga) com o degrau k quebrado, k = 2..=atual
    for k in 2..=CURRENT_SCHEMA_VERSION {
        let dir = TempDir::new(&format!("compat-fail-{k}"));
        let (path, digest, _) = project_at(&dir, 1);
        let before = dump(&path);
        let bad = with_broken(k);
        let err = ProjectStore::open_with_migrations(&path, &fast(), &bad, CURRENT_SCHEMA_VERSION)
            .unwrap_err();
        assert_eq!(err.code, StoreErrorCode::MigrationFailed, "step {k}");
        assert_eq!(err.details.as_ref().unwrap()["failed_version"], k);
        // atômico por degrau: o arquivo fica na última versão concluída, o DDL parcial sumiu
        assert_eq!(user_version(&path), i64::from(k - 1), "step {k}");
        assert!(
            !tables(&raw(&path)).contains(&"half_done".to_owned()),
            "step {k}: partial DDL must be rolled back"
        );
        assert_nothing_lost(&before, &path, &format!("failed step {k}"));
        let report = ProjectStore::validate_file(&path);
        assert!(
            report.ok,
            "step {k}: the half-migrated file is a sound project of version {}: {:?}",
            k - 1,
            report.issues
        );
        // o backup guarda o projeto ORIGINAL (v1) — é o caminho de rollback manual
        let bak = backup_of(&path, 1);
        assert!(bak.exists(), "step {k}: backup exists");
        assert_eq!(user_version(&bak), 1);
        assert_nothing_lost(&before, &bak, &format!("step {k} backup"));
        // recuperação 1: corrigir o degrau e tentar de novo, no mesmo arquivo
        let (store, state) = ProjectStore::open(&path, &fast()).unwrap();
        assert_eq!(store.schema_version(), CURRENT_SCHEMA_VERSION);
        assert_eq!(capia_commands::document_digest(&state.doc), digest);
        store.close().unwrap();
        assert_nothing_lost(&before, &path, &format!("retry after step {k}"));
        // recuperação 2: restaurar o backup num arquivo novo e migrar a partir dele
        let restored = dir.file("restored.capia");
        std::fs::copy(&bak, &restored).unwrap();
        let (store, state) = ProjectStore::open(&restored, &fast()).unwrap();
        assert_eq!(store.schema_version(), CURRENT_SCHEMA_VERSION);
        assert_eq!(capia_commands::document_digest(&state.doc), digest);
        store.close().unwrap();
        assert_nothing_lost(&before, &restored, &format!("restored backup, step {k}"));
    }
}

#[test]
fn a_failing_migration_on_a_data_rich_v4_project_keeps_every_ai_row() {
    let dir = TempDir::new("compat-v4-fail");
    let (path, _, _) = project_at(&dir, 4);
    let before = dump(&path);
    let err = ProjectStore::open_with_migrations(&path, &fast(), &with_broken(5), 5).unwrap_err();
    assert_eq!(err.code, StoreErrorCode::MigrationFailed);
    assert_eq!(user_version(&path), 4);
    assert_nothing_lost(&before, &path, "v4 + failed v5");
    assert!(
        !tables(&raw(&path)).contains(&"ai_runs".to_owned()),
        "no Phase 5 table leaked from the rolled-back migration"
    );
    assert!(backup_of(&path, 4).exists());
}

#[test]
fn the_migration_list_is_contiguous_and_every_version_has_a_matrix_case() {
    for (i, m) in MIGRATIONS.iter().enumerate() {
        assert_eq!(m.version as usize, i + 1);
    }
    // se alguém criar o schema 6 sem estender `seed`/a matriz acima, este teste falha de propósito
    assert_eq!(MIGRATIONS.len(), 5);
}
