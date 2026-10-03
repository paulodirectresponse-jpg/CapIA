//! Propriedade com nested (em memória): sequências aleatórias de comandos que criam/apagam
//! sequences, aninham, retargetam e ligam `follow_length`. Em **toda** sequência:
//! invariantes + DAG sempre válidos, índices consistentes, rejeição atômica, `apply∘undo = id`,
//! `undo∘redo = id` e replay determinístico.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::{Actor, Engine, Transaction};
use capia_model::{Document, validate_document, validate_nested_graph};
use common::nested::{nested_base_engine, random_nested_command};
use common::{Rng, tx};

fn same_state(a: &Document, b: &Document) -> bool {
    let mut a = a.clone();
    a.revision = b.revision;
    &a == b
}

fn run_case(seed: u64, steps: usize) -> (Vec<Transaction>, usize) {
    let mut rng = Rng::new(seed ^ 0xA5A5);
    let mut e: Engine = nested_base_engine();
    let initial = e.document().clone();
    let base = e.history().len();
    let (mut counter, mut op) = (0, 1_000);
    let mut accepted = Vec::new();
    let mut nested_accepted = 0;
    for _ in 0..steps {
        let command = random_nested_command(&mut rng, &e, &mut counter);
        let is_nested = matches!(
            command,
            capia_commands::Command::InsertNested { .. }
                | capia_commands::Command::SetNestedTarget { .. }
        );
        let t = tx("n", &mut op, vec![command.clone()]);
        let (before, rev) = (e.document().clone(), e.revision());
        match e.execute(&Actor::user("p"), t.clone(), 0) {
            Ok(_) => {
                nested_accepted += usize::from(is_nested);
                let v = validate_document(e.document());
                assert!(
                    v.is_empty(),
                    "seed {seed}: invariants broken by {command:?}: {v:?}"
                );
                assert!(
                    validate_nested_graph(e.document()).is_empty(),
                    "seed {seed}: nested graph invalid"
                );
                for (_, s) in e.document().sequences() {
                    assert!(
                        s.index_is_consistent(),
                        "seed {seed}: index out of sync after {command:?}"
                    );
                }
                let after = e.document().clone();
                e.undo(&Actor::user("p"), 0).unwrap();
                assert!(
                    same_state(e.document(), &before),
                    "seed {seed}: undo of {command:?} is not exact"
                );
                e.redo(&Actor::user("p"), 0).unwrap();
                assert!(
                    same_state(e.document(), &after),
                    "seed {seed}: redo of {command:?} is not exact"
                );
                accepted.push(t);
            }
            Err(err) => {
                assert!(
                    e.document() == &before && e.revision() == rev,
                    "seed {seed}: rejected {command:?} ({}) changed the document",
                    err.code
                );
            }
        }
    }
    while e.applied_history().len() > base {
        e.undo(&Actor::user("p"), 0).unwrap();
    }
    assert!(
        same_state(e.document(), &initial),
        "seed {seed}: full undo did not return to the start"
    );
    (accepted, nested_accepted)
}

fn cases() -> u64 {
    std::env::var("CAPIA_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000)
}

#[test]
fn nested_command_sequences_preserve_the_dag_the_invariants_and_undo() {
    let n = cases();
    let (mut accepted, mut nested) = (0, 0);
    for seed in 0..n {
        let (a, nn) = run_case(seed, 14);
        accepted += a.len();
        nested += nn;
    }
    if n >= 1_000 {
        assert!(
            nested as u64 > n / 3,
            "the generator should land nested edits ({nested} in {n} cases)"
        );
        assert!(
            accepted as u64 > 4 * n,
            "only {accepted} accepted commands in {n} cases"
        );
    }
    eprintln!("nested property: {n} cases, {accepted} accepted commands, {nested} nested edits");
}

#[test]
fn nested_replay_is_deterministic() {
    for seed in 0..200 {
        let (accepted, _) = run_case(seed, 14);
        let (mut a, mut b) = (nested_base_engine(), nested_base_engine());
        for t in accepted {
            assert_eq!(
                a.execute(&Actor::user("x"), t.clone(), 0).unwrap(),
                b.execute(&Actor::user("x"), t, 0).unwrap(),
                "seed {seed}"
            );
        }
        assert_eq!(a.document(), b.document(), "seed {seed}");
    }
}
