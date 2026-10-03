//! Propriedade com persistência: comandos aleatórios → salvar → reabrir → continuar → salvar →
//! reabrir, em **lockstep** com um engine só em memória. Depois de cada reabertura o estado
//! durável carregado do `.capia` tem que ser **idêntico** ao do engine em memória (documento,
//! histórico completo com ops inversas, cursor, `operation_id`s, resultados, log de mudanças,
//! auditoria) e os dois têm que responder igual a qualquer comando seguinte (inclusive erros).
//!
//! Orçamento: 1.000 sequências por padrão em cada gerador (≈ 28 mil comandos e ≈ 8 mil reaberturas no total);
//! `CAPIA_PROP_CASES` altera. É menos que os 10.000 da M05 porque cada reabertura faz IO real
//! (SQLite com `synchronous=NORMAL`: a atomicidade vem do WAL, o fsync só importa para queda de
//! energia, que os testes de crash cobrem à parte).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
#[path = "../../capia-commands/tests/common/mod.rs"]
mod generators;

use capia_commands::{Actor, Command, Engine, EngineConfig, Transaction};
use capia_model::validate_document;
use capia_store::{ProjectStore, StoreOptions, Synchronous};
use generators::nested::{base_tx, random_nested_command};
use generators::{Rng, base_engine, random_command, tx};
use std::path::Path;

fn opts(snapshot_every: u64) -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        snapshot_every,
        ..StoreOptions::default()
    }
}

fn cases(default: u64) -> u64 {
    std::env::var("CAPIA_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn reopen(path: &Path, o: &StoreOptions) -> Engine {
    ProjectStore::open_engine(path, o, [1; 32], EngineConfig::default()).unwrap()
}

/// Cria o projeto com o mesmo documento inicial do engine de referência e devolve ambos
/// (o de referência é restaurado do estado persistido, então começam idênticos).
fn start(path: &Path, o: &StoreOptions, base: Engine) -> (Engine, Engine) {
    let (store, state) = ProjectStore::create_with_document(path, base.document(), o).unwrap();
    let persisted = store
        .into_engine(state, [1; 32], EngineConfig::default())
        .unwrap();
    let mem = Engine::restore(persisted.export_state(), [2; 32], EngineConfig::default()).unwrap();
    (persisted, mem)
}

struct Stats {
    commands: usize,
    reopens: usize,
    accepted: usize,
}

fn run_case<F>(seed: u64, steps: usize, base: fn() -> Engine, mut next: F, st: &mut Stats)
where
    F: FnMut(&mut Rng, &Engine, &mut u32) -> Command,
{
    let dir = common::TempDir::new("prop");
    let path = dir.file("p.capia");
    let o = opts(1 + seed % 5); // cadências de snapshot variadas: 1..=5
    let mut rng = Rng::new(seed);
    let (mut persisted, mut mem) = start(&path, &o, base());
    let (mut counter, mut op) = (0u32, 10_000);
    for step in 0..steps {
        // 12%: undo/redo em vez de comando
        let action = rng.below(100);
        let (p, m) = if action < 6 && mem.can_undo() {
            (
                persisted.undo(&Actor::user("p"), 0),
                mem.undo(&Actor::user("p"), 0),
            )
        } else if action < 12 && mem.can_redo() {
            (
                persisted.redo(&Actor::user("p"), 0),
                mem.redo(&Actor::user("p"), 0),
            )
        } else {
            let command = next(&mut rng, &mem, &mut counter);
            let t: Transaction = tx("prop", &mut op, vec![command]);
            (
                persisted.execute(&Actor::user("p"), t.clone(), 1),
                mem.execute(&Actor::user("p"), t, 1),
            )
        };
        st.commands += 1;
        assert_eq!(
            p.is_ok(),
            m.is_ok(),
            "seed {seed} step {step}: persisted and in-memory engines disagree: {p:?} vs {m:?}"
        );
        match (&p, &m) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b, "seed {seed} step {step}");
                st.accepted += 1;
            }
            (Err(a), Err(b)) => assert_eq!(a.code, b.code, "seed {seed} step {step}"),
            _ => {}
        }
        // salvar → reabrir → continuar (≈ 1 em 4 passos e sempre no fim)
        if rng.below(4) == 0 || step + 1 == steps {
            drop(persisted);
            persisted = reopen(&path, &o);
            st.reopens += 1;
            assert_eq!(
                persisted.export_state(),
                mem.export_state(),
                "seed {seed} step {step}: state loaded from disk differs from the in-memory engine"
            );
            assert!(validate_document(persisted.document()).is_empty());
        }
    }
    // histórico inteiro reabre e desfaz igual
    while mem.can_undo() {
        persisted.undo(&Actor::user("p"), 2).unwrap();
        mem.undo(&Actor::user("p"), 2).unwrap();
    }
    drop(persisted);
    let persisted = reopen(&path, &o);
    assert_eq!(
        persisted.export_state(),
        mem.export_state(),
        "seed {seed}: after undoing everything"
    );
    let report = ProjectStore::validate_file(&path);
    assert!(report.ok, "seed {seed}: {:?}", report.issues);
}

#[test]
fn random_commands_with_save_and_reopen_stay_identical_to_the_in_memory_engine() {
    let n = cases(1_000);
    let mut st = Stats {
        commands: 0,
        reopens: 0,
        accepted: 0,
    };
    for seed in 0..n {
        run_case(seed, 14, base_engine, random_command, &mut st);
    }
    eprintln!(
        "timeline property: {n} cases, {} commands ({} accepted), {} reopens",
        st.commands, st.accepted, st.reopens
    );
    assert!(st.accepted as u64 > n * 4);
}

#[test]
fn nested_edits_with_save_and_reopen_stay_identical_to_the_in_memory_engine() {
    fn base() -> Engine {
        let mut e = Engine::new(Default::default(), [6; 32]);
        e.execute(&Actor::system(), base_tx(), 0).unwrap();
        e
    }
    let n = cases(1_000);
    let mut st = Stats {
        commands: 0,
        reopens: 0,
        accepted: 0,
    };
    for seed in 0..n {
        run_case(seed, 14, base, random_nested_command, &mut st);
    }
    eprintln!(
        "nested property: {n} cases, {} commands ({} accepted), {} reopens",
        st.commands, st.accepted, st.reopens
    );
    assert!(st.accepted as u64 > n * 4);
}
