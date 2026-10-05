//! Undo seletivo (Fase 5): desfaz entradas escolhidas (uma AI Run) sem reescrever o histórico e
//! sem apagar edição manual incompatível em silêncio.

#![allow(clippy::unwrap_used, clippy::expect_used, unreachable_pub)]

mod common;

use capia_commands::{
    Actor, Command, CommandEnvelope, Engine, NewClip, SelectiveUndoMode, Transaction,
};
use capia_model::{ClipContent, ErrorCode};
use capia_time::{Rational, Ticks};
use common::{FRAME, base_engine, seq_id};

fn insert(clip: &str, track: &str, start: i64, op: &str) -> Transaction {
    Transaction {
        transaction_id: None,
        label: format!("insert {clip}"),
        base_revision: None,
        commands: vec![CommandEnvelope {
            operation_id: op.into(),
            reference: None,
            command: Command::InsertClip {
                track: track.into(),
                start: Ticks(start * FRAME),
                clip: NewClip {
                    id: Some(clip.into()),
                    name: clip.into(),
                    duration: Ticks(30 * FRAME),
                    content: ClipContent::Media {
                        asset: "long".into(),
                        has_video: true,
                        has_audio: false,
                    },
                    source_in: Ticks(0),
                    speed: Rational::ONE,
                    reversed: false,
                    properties: Default::default(),
                },
                split_at_insert: false,
                split_new_id: None,
            },
        }],
        max_ops: None,
    }
}

fn agent() -> Actor {
    Actor::agent("run:r1")
}

fn user() -> Actor {
    Actor::user("me")
}

fn agent_commit(e: &mut Engine, t: Transaction) -> u64 {
    let pv = e.preview(&agent(), t, 1).unwrap();
    e.apply_plan(&agent(), &pv.plan_token.unwrap(), 2)
        .unwrap()
        .entry_id
}

fn has_clip(e: &Engine, id: &str) -> bool {
    e.document()
        .sequence(&seq_id())
        .unwrap()
        .clip(&id.into())
        .is_some()
}

#[test]
fn undoes_a_run_when_nothing_else_touched_it_and_keeps_history() {
    let mut e = base_engine();
    let a = agent_commit(&mut e, insert("a1", "V2", 0, "r1-a1"));
    let b = agent_commit(&mut e, insert("a2", "V3", 40, "r1-a2"));
    let before = e.history().len();
    let r = e
        .selective_undo(&user(), &[a, b], SelectiveUndoMode::Safe, "Undo run r1", 10)
        .unwrap();
    assert!(!has_clip(&e, "a1") && !has_clip(&e, "a2"));
    // histórico preservado e ampliado (nada reescrito); a própria entrada é desfazível
    assert_eq!(e.history().len(), before + 1);
    assert!(e.history().iter().any(|h| h.id == a));
    e.undo(&user(), 11).unwrap();
    assert!(has_clip(&e, "a1") && has_clip(&e, "a2"));
    assert!(r.revision > r.revision_before);
}

#[test]
fn keeps_unrelated_manual_edits_and_refuses_conflicting_ones() {
    let mut e = base_engine();
    let a = agent_commit(&mut e, insert("a1", "V2", 0, "r1-a1"));
    // edição manual em outra track/clip: não conflita
    e.execute(&user(), insert("m1", "V3", 100, "m-1"), 5)
        .unwrap();
    e.selective_undo(&user(), &[a], SelectiveUndoMode::Safe, "Undo", 10)
        .unwrap();
    assert!(!has_clip(&e, "a1"));
    assert!(has_clip(&e, "m1"), "manual work must survive");
}

#[test]
fn a_later_manual_edit_on_the_same_entity_is_a_conflict() {
    let mut e = base_engine();
    let a = agent_commit(&mut e, insert("a1", "V2", 0, "r1-a1"));
    // o usuário renomeia o clip da run
    let rename = Transaction {
        transaction_id: None,
        label: "rename".into(),
        base_revision: None,
        commands: vec![CommandEnvelope {
            operation_id: "m-rename".into(),
            reference: None,
            command: Command::RenameClip {
                clip: "a1".into(),
                name: "mine".into(),
            },
        }],
        max_ops: None,
    };
    e.execute(&user(), rename, 5).unwrap();
    let rep = e.selective_undo_report(&[a]).unwrap();
    assert!(!rep.conflicts.is_empty());
    let err = e
        .selective_undo(&user(), &[a], SelectiveUndoMode::Safe, "Undo", 10)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(has_clip(&e, "a1"), "nothing may change on conflict");
    // partial: todas as entradas conflitam ⇒ também recusa
    let err = e
        .selective_undo(&user(), &[a], SelectiveUndoMode::Partial, "Undo", 11)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
}

#[test]
fn partial_undoes_the_clean_entries_only() {
    let mut e = base_engine();
    let a = agent_commit(&mut e, insert("a1", "V2", 0, "r1-a1"));
    let b = agent_commit(&mut e, insert("a2", "V3", 40, "r1-a2"));
    let rename = Transaction {
        transaction_id: None,
        label: "rename".into(),
        base_revision: None,
        commands: vec![CommandEnvelope {
            operation_id: "m-rename".into(),
            reference: None,
            command: Command::RenameClip {
                clip: "a1".into(),
                name: "mine".into(),
            },
        }],
        max_ops: None,
    };
    e.execute(&user(), rename, 5).unwrap();
    assert_eq!(
        e.selective_undo(&user(), &[a, b], SelectiveUndoMode::Safe, "U", 9)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    e.selective_undo(&user(), &[a, b], SelectiveUndoMode::Partial, "U", 10)
        .unwrap();
    assert!(has_clip(&e, "a1"), "conflicting entry stays");
    assert!(!has_clip(&e, "a2"), "clean entry is undone");
}

#[test]
fn agents_cannot_selectively_undo_and_unknown_entries_are_rejected() {
    let mut e = base_engine();
    let a = agent_commit(&mut e, insert("a1", "V2", 0, "r1-a1"));
    assert_eq!(
        e.selective_undo(&agent(), &[a], SelectiveUndoMode::Safe, "U", 9)
            .unwrap_err()
            .code,
        ErrorCode::PreviewRequired
    );
    assert_eq!(
        e.selective_undo(&user(), &[9999], SelectiveUndoMode::Safe, "U", 9)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        e.selective_undo(&user(), &[], SelectiveUndoMode::Safe, "U", 9)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
}
