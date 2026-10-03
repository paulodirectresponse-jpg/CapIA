//! Testes de propriedade do Command Engine (docs/TEST_STRATEGY.md): sequências aleatórias e
//! determinísticas (gerador xorshift com semente — sem dependências), com a semente e os comandos
//! impressos em qualquer falha para reprodução.
//!
//! Propriedades verificadas em **cada** sequência:
//! 1. as invariantes do documento valem após qualquer comando bem-sucedido;
//! 2. comando rejeitado deixa o documento e a revisão **idênticos** (atomicidade);
//! 3. `apply ∘ undo = id` e `undo ∘ redo = id` (a cada passo e no histórico inteiro);
//! 4. round-trip JSON do documento é exato;
//! 5. o replay da mesma sequência num engine novo produz o mesmo documento (determinismo).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Engine, Transaction};
use capia_model::{Document, validate_document};
use common::{Rng, base_engine, random_command, tx};

mod common;

fn same_state(a: &Document, b: &Document) -> bool {
    let mut a = a.clone();
    a.revision = b.revision;
    &a == b
}

fn check_indexes(e: &Engine) {
    for (_, s) in e.document().sequences() {
        assert!(s.index_is_consistent(), "track index out of sync");
    }
}

/// Executa uma sequência aleatória e confere todas as propriedades. Devolve os comandos aceitos.
fn run_case(seed: u64, steps: usize) -> Vec<Transaction> {
    let mut rng = Rng::new(seed);
    let mut e = base_engine();
    let initial = e.document().clone();
    let base_entries = e.history().len();
    let mut counter = 0;
    let mut op = 1_000;
    let mut accepted = Vec::new();
    let mut log = Vec::new();
    let fail = |msg: String, log: &Vec<String>| -> ! {
        panic!("seed {seed}: {msg}\ncommands so far:\n{}", log.join("\n"))
    };
    for _ in 0..steps {
        let command = random_command(&mut rng, &e, &mut counter);
        log.push(format!("{command:?}"));
        let transaction = tx("random", &mut op, vec![command]);
        let (before, rev_before) = (e.document().clone(), e.revision());
        match e.execute(&Actor::user("prop"), transaction.clone(), 0) {
            Ok(_) => {
                let v = validate_document(e.document());
                if !v.is_empty() {
                    fail(
                        format!("invariants violated after a successful commit: {v:?}"),
                        &log,
                    );
                }
                check_indexes(&e);
                let after = e.document().clone();
                // apply ∘ undo = id  e  undo ∘ redo = id
                e.undo(&Actor::user("prop"), 0).unwrap();
                if !same_state(e.document(), &before) {
                    fail("undo did not restore the previous document".into(), &log);
                }
                check_indexes(&e);
                e.redo(&Actor::user("prop"), 0).unwrap();
                if !same_state(e.document(), &after) {
                    fail("redo did not reproduce the committed document".into(), &log);
                }
                accepted.push(transaction);
            }
            Err(err) => {
                if e.revision() != rev_before || e.document() != &before {
                    fail(
                        format!("rejected command ({}) changed the document", err.code),
                        &log,
                    );
                }
            }
        }
    }
    // histórico inteiro: undo tudo → documento inicial; redo tudo → documento final
    let final_doc = e.document().clone();
    let mut undone = 0;
    while e.applied_history().len() > base_entries {
        e.undo(&Actor::user("prop"), 0).unwrap();
        undone += 1;
    }
    if !same_state(e.document(), &initial) {
        fail(
            format!("undoing {undone} entries did not return to the initial document"),
            &log,
        );
    }
    while e.can_redo() {
        e.redo(&Actor::user("prop"), 0).unwrap();
    }
    if !same_state(e.document(), &final_doc) {
        fail(
            "redoing the whole history did not reproduce the final document".into(),
            &log,
        );
    }
    // round-trip JSON exato
    let json = serde_json::to_string(e.document()).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    if &back != e.document() {
        fail("JSON round-trip changed the document".into(), &log);
    }
    accepted
}

fn cases() -> u64 {
    std::env::var("CAPIA_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

#[test]
fn random_command_sequences_preserve_invariants_and_undo_exactly() {
    let n = cases();
    let mut by_type: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for seed in 0..n {
        for t in run_case(seed, 8) {
            *by_type
                .entry(t.commands[0].command.type_name())
                .or_default() += 1;
        }
    }
    // a geração precisa exercitar de verdade cada família de comando (não só ser rejeitada)
    if n >= 10_000 {
        for (name, min) in [
            ("insert_clip", 5_000),
            ("delete_clip", 500),
            ("trim_clip", 1_000),
            ("split_clip", 1_000),
            ("set_clip_speed", 500),
            ("move_clips", 200),
            ("add_keyframe", 1_000),
            ("move_keyframe", 20),
            ("set_property", 100),
            ("set_track_flags", 200),
            ("add_marker", 200),
        ] {
            let got = by_type.get(name).copied().unwrap_or(0);
            assert!(
                got >= min,
                "{name}: only {got} accepted in {n} cases ({by_type:?})"
            );
        }
    }
    eprintln!("accepted commands by type over {n} cases: {by_type:?}");
}

#[test]
fn replaying_accepted_commands_is_deterministic() {
    for seed in 0..300 {
        let accepted = run_case(seed, 12);
        let mut a = base_engine();
        let mut b = base_engine();
        for t in accepted {
            let ra = a.execute(&Actor::user("x"), t.clone(), 0).unwrap();
            let rb = b.execute(&Actor::user("x"), t, 0).unwrap();
            assert_eq!(ra, rb, "seed {seed}");
        }
        assert_eq!(a.document(), b.document(), "seed {seed}");
    }
}
