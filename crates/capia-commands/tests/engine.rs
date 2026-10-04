//! Semântica do Engine: transações, idempotência (ADR-029), plano por token (ADR-030),
//! conflitos/rebase, refs simbólicas, histórico.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{
    Actor, Command, CommandEnvelope, CommandError, Edge, Engine, EngineConfig, NewClip,
    RippleScope, Transaction,
};
use capia_model::{
    ClipContent, ClipId, Document, EntityKind, EntityRef, ErrorCode, SequenceId, TrackId, TrackKind,
};
use capia_time::{FrameRate, Rational, Ticks};

const FRAME: i64 = 23_520_000;

fn frames(n: i64) -> Ticks {
    Ticks(n * FRAME)
}

fn env(op: &str, command: Command) -> CommandEnvelope {
    CommandEnvelope {
        operation_id: op.into(),
        reference: None,
        command,
    }
}

fn tx(label: &str, commands: Vec<CommandEnvelope>) -> Transaction {
    Transaction {
        transaction_id: None,
        label: label.into(),
        base_revision: None,
        commands,
        max_ops: None,
    }
}

fn insert(op: &str, track: &str, start: i64, id: &str, dur: i64) -> CommandEnvelope {
    env(
        op,
        Command::InsertClip {
            track: track.into(),
            start: frames(start),
            clip: NewClip {
                id: Some(id.into()),
                name: id.into(),
                duration: frames(dur),
                content: ClipContent::Solid {
                    color: "#000".into(),
                },
                source_in: Ticks::ZERO,
                speed: Rational::ONE,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: false,
            split_new_id: None,
        },
    )
}

fn user() -> Actor {
    Actor::user("u")
}

/// Sequence S com V1 (magnética), V2 e V3 (livres).
fn engine() -> Engine {
    engine_with(EngineConfig::default())
}

fn engine_with(config: EngineConfig) -> Engine {
    let mut e = Engine::with_config(Document::new(), [9; 32], config);
    let mut cmds = vec![env(
        "setup-seq",
        Command::CreateSequence {
            id: Some("S".into()),
            name: "S".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: None,
            width: None,
            height: None,
            folder: None,
        },
    )];
    for (id, magnetic) in [("V3", false), ("V2", false), ("V1", true)] {
        cmds.push(env(
            &format!("setup-{id}"),
            Command::AddTrack {
                sequence: "S".into(),
                id: Some(id.into()),
                kind: TrackKind::Visual,
                name: None,
                role: None,
                magnetic,
                index: None,
            },
        ));
    }
    e.execute(&Actor::system(), tx("setup", cmds), 0).unwrap();
    e
}

fn clip_ids(e: &Engine, track: &str) -> Vec<String> {
    e.document()
        .sequence(&SequenceId::from("S"))
        .unwrap()
        .track_clips(&TrackId::from(track))
        .map(|c| c.id.0.clone())
        .collect()
}

fn code(r: Result<impl core::fmt::Debug, CommandError>) -> ErrorCode {
    r.unwrap_err().code
}

#[test]
fn a_transaction_is_one_history_entry_and_one_revision() {
    let mut e = engine();
    let rev = e.revision();
    let r = e
        .execute(
            &user(),
            tx(
                "two clips",
                vec![
                    insert("a1", "V2", 0, "a", 10),
                    insert("a2", "V2", 20, "b", 10),
                ],
            ),
            5,
        )
        .unwrap();
    assert_eq!(r.revision, rev + 1);
    assert_eq!(e.history().len(), 2); // setup + esta
    let entry = e.history().last().unwrap();
    assert_eq!(entry.commands.len(), 2);
    assert_eq!(entry.label, "two clips");
    assert_eq!(entry.timestamp_ms, 5);
    assert_eq!(entry.inverse_ops.len(), entry.ops.len());
    assert_eq!(clip_ids(&e, "V2"), ["a", "b"]);
}

#[test]
fn a_failing_command_rolls_the_whole_transaction_back_with_its_index() {
    let mut e = engine();
    let before = e.document().clone();
    let err = e
        .execute(
            &user(),
            tx(
                "bad",
                vec![
                    insert("a1", "V2", 0, "a", 10),
                    insert("a2", "V2", 5, "b", 10),
                ],
            ),
            0,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Overlap);
    assert_eq!(err.command_index, Some(1));
    assert!(err.hint.is_some(), "OVERLAP carries free ranges");
    assert_eq!(e.document(), &before);
    // os ids da transação rejeitada continuam livres
    e.execute(&user(), tx("ok", vec![insert("a1", "V2", 0, "a", 10)]), 0)
        .unwrap();
}

#[test]
fn resubmitting_the_same_operation_ids_replays_without_reapplying() {
    let mut e = engine();
    let t = tx("t", vec![insert("op-1", "V2", 0, "a", 10)]);
    let first = e.execute(&user(), t.clone(), 1).unwrap();
    let rev = e.revision();
    let again = e.execute(&user(), t, 2).unwrap();
    assert!(again.replayed && !first.replayed);
    assert_eq!(again.entry_id, first.entry_id);
    assert_eq!(e.revision(), rev, "nothing was applied twice");
    assert_eq!(clip_ids(&e, "V2"), ["a"]);
    assert!(e.applied_operation("op-1").is_some());
}

#[test]
fn reusing_an_operation_id_for_a_different_command_is_rejected() {
    let mut e = engine();
    e.execute(&user(), tx("t", vec![insert("op-1", "V2", 0, "a", 10)]), 0)
        .unwrap();
    let rev = e.revision();
    assert_eq!(
        code(e.execute(&user(), tx("t", vec![insert("op-1", "V2", 50, "z", 10)]), 0)),
        ErrorCode::OperationIdReused
    );
    assert_eq!(e.revision(), rev);
}

#[test]
fn partially_known_or_duplicated_operation_ids_conflict() {
    let mut e = engine();
    e.execute(&user(), tx("t", vec![insert("op-1", "V2", 0, "a", 10)]), 0)
        .unwrap();
    let mixed = tx(
        "t",
        vec![
            insert("op-1", "V2", 0, "a", 10),
            insert("op-2", "V2", 20, "b", 10),
        ],
    );
    assert_eq!(
        code(e.execute(&user(), mixed, 0)),
        ErrorCode::OperationIdConflict
    );
    let dup = tx(
        "t",
        vec![
            insert("same", "V3", 0, "x", 5),
            insert("same", "V3", 10, "y", 5),
        ],
    );
    assert_eq!(
        code(e.execute(&user(), dup, 0)),
        ErrorCode::OperationIdConflict
    );
    let long = tx("t", vec![insert(&"x".repeat(129), "V3", 0, "q", 5)]);
    assert_eq!(
        code(e.execute(&user(), long, 0)),
        ErrorCode::InvalidArgument
    );
    let empty = tx("t", vec![insert("", "V3", 0, "q", 5)]);
    assert_eq!(
        code(e.execute(&user(), empty, 0)),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        code(e.execute(&user(), tx("empty", vec![]), 0)),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn undo_does_not_free_operation_ids() {
    let mut e = engine();
    let t = tx("t", vec![insert("op-1", "V2", 0, "a", 10)]);
    e.execute(&user(), t.clone(), 0).unwrap();
    e.undo(&user(), 0).unwrap();
    assert!(clip_ids(&e, "V2").is_empty());
    let again = e.execute(&user(), t, 0).unwrap();
    assert!(again.replayed, "respects the undo: nothing is re-applied");
    assert!(clip_ids(&e, "V2").is_empty());
    e.redo(&user(), 0).unwrap();
    assert_eq!(clip_ids(&e, "V2"), ["a"]);
}

#[test]
fn derived_ids_are_deterministic_per_operation_id() {
    let mk = |op: &str, start: i64| {
        env(
            op,
            Command::InsertClip {
                track: "V2".into(),
                start: frames(start),
                clip: NewClip {
                    id: None,
                    name: String::new(),
                    duration: frames(5),
                    content: ClipContent::Solid {
                        color: "#000".into(),
                    },
                    source_in: Ticks::ZERO,
                    speed: Rational::ONE,
                    reversed: false,
                    properties: Default::default(),
                },
                split_at_insert: false,
                split_new_id: None,
            },
        )
    };
    let (mut a, mut b) = (engine(), engine());
    let ra = a
        .execute(&user(), tx("t", vec![mk("run1:edit:0:0", 0)]), 0)
        .unwrap();
    let rb = b
        .execute(&user(), tx("t", vec![mk("run1:edit:0:0", 0)]), 0)
        .unwrap();
    assert_eq!(ra.results[0].id, rb.results[0].id);
    assert!(ra.results[0].id.as_ref().unwrap().starts_with("clip_"));
    let rc = a.execute(&user(), tx("t", vec![mk("run1:edit:0:1", 50)]), 0);
    assert_ne!(rc.unwrap().results[0].id, ra.results[0].id);
}

#[test]
fn agents_and_api_clients_must_go_through_preview() {
    let mut e = engine();
    for actor in [Actor::agent("run_1"), Actor::api("client")] {
        assert_eq!(
            code(e.execute(&actor, tx("t", vec![insert("op", "V2", 0, "a", 5)]), 0)),
            ErrorCode::PreviewRequired
        );
    }
    assert!(clip_ids(&e, "V2").is_empty());
}

#[test]
fn preview_does_not_touch_the_document_and_apply_plan_commits_exactly_the_diff() {
    let mut e = engine();
    let agent = Actor::agent("run_1");
    let before = e.document().clone();
    let p = e
        .preview(
            &agent,
            tx("plan", vec![insert("p-1", "V2", 0, "a", 10)]),
            1_000,
        )
        .unwrap();
    assert_eq!(e.document(), &before, "preview is a dry run");
    assert!(!p.already_applied);
    assert_eq!(p.ops.len(), 1);
    assert_eq!(p.base_revision, e.revision());
    let token = p.plan_token.clone().unwrap();
    assert!(
        !token.contains(&"9".repeat(8)),
        "the key never shows up in the token"
    );
    let r = e.apply_plan(&agent, &token, 2_000).unwrap();
    assert_eq!(r.revision, before.revision + 1);
    assert_eq!(clip_ids(&e, "V2"), ["a"]);
    let entry = e.history().last().unwrap();
    assert_eq!(entry.actor, agent);
    assert!(entry.plan_id.is_some());
    // reenvio do apply após sucesso devolve o resultado original (idempotente)
    let again = e.apply_plan(&agent, &token, 3_000).unwrap();
    assert!(again.replayed);
    assert_eq!(again.entry_id, r.entry_id);
    assert_eq!(e.revision(), r.revision);
}

#[test]
fn tampered_unknown_and_foreign_tokens_are_rejected() {
    let mut e = engine();
    let agent = Actor::agent("run_1");
    let p = e
        .preview(&agent, tx("plan", vec![insert("p-1", "V2", 0, "a", 10)]), 0)
        .unwrap();
    let token = p.plan_token.unwrap();
    let flipped = {
        let mut chars: Vec<char> = token.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == '0' { '1' } else { '0' };
        chars.into_iter().collect::<String>()
    };
    for bad in [
        flipped.as_str(),
        "garbage",
        "",
        "plan_1.",
        "plan_999.00",
        ".abc",
    ] {
        assert_eq!(
            code(e.apply_plan(&agent, bad, 1)),
            ErrorCode::PlanTokenInvalid,
            "token {bad:?}"
        );
    }
    // token de outro processo (outra chave): plano desconhecido
    let mut other = engine_with(EngineConfig::default());
    assert_eq!(
        code(other.apply_plan(&agent, &token, 1)),
        ErrorCode::PlanTokenInvalid
    );
    // outro ator
    assert_eq!(
        code(e.apply_plan(&Actor::agent("run_2"), &token, 1)),
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        code(e.apply_plan(&Actor::api("run_1"), &token, 1)),
        ErrorCode::PermissionDenied
    );
    assert!(
        clip_ids(&e, "V2").is_empty(),
        "nothing was applied by the failed attempts"
    );
    e.apply_plan(&agent, &token, 1).unwrap();
}

#[test]
fn expired_plans_are_rejected() {
    let mut e = engine_with(EngineConfig {
        plan_ttl_ms: 1_000,
        ..EngineConfig::default()
    });
    let agent = Actor::agent("run_1");
    let p = e
        .preview(
            &agent,
            tx("plan", vec![insert("p-1", "V2", 0, "a", 10)]),
            10_000,
        )
        .unwrap();
    assert_eq!(p.expires_at_ms, 11_000);
    let token = p.plan_token.unwrap();
    assert_eq!(
        code(e.apply_plan(&agent, &token, 11_001)),
        ErrorCode::PlanExpired
    );
    assert!(clip_ids(&e, "V2").is_empty());
    // um novo preview purga os expirados
    e.preview(
        &agent,
        tx("plan", vec![insert("p-2", "V2", 0, "b", 10)]),
        20_000,
    )
    .unwrap();
    assert_eq!(e.pending_plans(), 1);
}

#[test]
fn the_preview_store_is_bounded() {
    let mut e = engine_with(EngineConfig {
        max_plans: 2,
        ..EngineConfig::default()
    });
    let agent = Actor::agent("run_1");
    let tokens: Vec<String> = (0..3)
        .map(|i| {
            e.preview(
                &agent,
                tx(
                    "plan",
                    vec![insert(
                        &format!("p-{i}"),
                        "V3",
                        i * 20,
                        &format!("c{i}"),
                        10,
                    )],
                ),
                0,
            )
            .unwrap()
            .plan_token
            .unwrap()
        })
        .collect();
    assert_eq!(e.pending_plans(), 2);
    assert_eq!(
        code(e.apply_plan(&agent, &tokens[0], 1)),
        ErrorCode::PlanTokenInvalid,
        "oldest evicted"
    );
    e.apply_plan(&agent, &tokens[2], 1).unwrap();
}

#[test]
fn a_plan_survives_unrelated_edits_but_not_conflicting_ones() {
    let agent = Actor::agent("run_1");
    // edição alheia que não toca nada do plano → rebase com o mesmo diff
    let mut e = engine();
    let token = e
        .preview(&agent, tx("plan", vec![insert("p-1", "V2", 0, "a", 10)]), 0)
        .unwrap()
        .plan_token
        .unwrap();
    e.execute(
        &user(),
        tx("other", vec![insert("u-1", "V3", 0, "z", 10)]),
        0,
    )
    .unwrap();
    let r = e.apply_plan(&agent, &token, 1).unwrap();
    assert!(!r.replayed);
    assert_eq!(clip_ids(&e, "V2"), ["a"]);
    assert_eq!(clip_ids(&e, "V3"), ["z"]);

    // edição que invalida o plano (ocupa o mesmo espaço) → PLAN_STATE_CHANGED, nada aplicado
    let mut e = engine();
    let token = e
        .preview(&agent, tx("plan", vec![insert("p-1", "V2", 0, "a", 10)]), 0)
        .unwrap()
        .plan_token
        .unwrap();
    e.execute(
        &user(),
        tx("other", vec![insert("u-1", "V2", 5, "z", 10)]),
        0,
    )
    .unwrap();
    let err = e.apply_plan(&agent, &token, 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::PlanStateChanged);
    assert_eq!(clip_ids(&e, "V2"), ["z"]);

    // edição que muda o diff sem invalidar (track magnética: o diff depende dos clips existentes)
    let mut e = engine();
    e.execute(
        &user(),
        tx("seed", vec![insert("s-1", "V1", 0, "m1", 30)]),
        0,
    )
    .unwrap();
    let token = e
        .preview(
            &agent,
            tx("plan", vec![insert("p-1", "V1", 30, "a", 10)]),
            0,
        )
        .unwrap()
        .plan_token
        .unwrap();
    e.execute(
        &user(),
        tx("other", vec![insert("u-1", "V1", 30, "m2", 30)]),
        0,
    )
    .unwrap();
    // o plano ainda é aplicável (insere em 30), mas o diff mudou (empurra m2): precisa de novo preview
    assert_eq!(
        code(e.apply_plan(&agent, &token, 1)),
        ErrorCode::PlanStateChanged
    );
}

#[test]
fn previewing_an_already_applied_transaction_reports_it() {
    let mut e = engine();
    let t = tx("t", vec![insert("op-1", "V2", 0, "a", 10)]);
    e.execute(&user(), t.clone(), 0).unwrap();
    let p = e.preview(&Actor::agent("a"), t, 0).unwrap();
    assert!(p.already_applied);
    assert!(p.plan_token.is_none());
}

#[test]
fn base_revision_rebases_when_disjoint_and_conflicts_when_not() {
    let mut e = engine();
    let base = e.revision();
    e.execute(&user(), tx("u", vec![insert("u-1", "V2", 0, "a", 10)]), 0)
        .unwrap();
    // disjunto: V3
    let mut t = tx("ai", vec![insert("ai-1", "V3", 0, "b", 10)]);
    t.base_revision = Some(base);
    e.execute(&user(), t, 0).unwrap();
    // mesma track V2 (entidade tocada por ambos) → CONFLICT
    let mut t = tx("ai2", vec![insert("ai-2", "V2", 50, "c", 10)]);
    t.base_revision = Some(base);
    let err = e.execute(&user(), t, 0).unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(
        err.entities
            .contains(&EntityRef::new(EntityKind::Track, "V2"))
    );
    // revisão do futuro
    let mut t = tx("x", vec![insert("x-1", "V3", 50, "d", 5)]);
    t.base_revision = Some(e.revision() + 5);
    assert_eq!(code(e.execute(&user(), t, 0)), ErrorCode::InvalidArgument);
}

#[test]
fn symbolic_refs_chain_commands_and_unresolved_refs_fail() {
    let mut e = Engine::new(Document::new(), [1; 32]);
    let mut seq = env(
        "r-1",
        Command::CreateSequence {
            id: None,
            name: "S".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: None,
            width: None,
            height: None,
            folder: None,
        },
    );
    seq.reference = Some("$seq".into());
    let mut trk = env(
        "r-2",
        Command::AddTrack {
            sequence: "$seq".into(),
            id: None,
            kind: TrackKind::Visual,
            name: None,
            role: None,
            magnetic: false,
            index: None,
        },
    );
    trk.reference = Some("$trk".into());
    let mut clip = insert("r-3", "$trk", 0, "c", 10);
    clip.reference = Some("$clip".into());
    let r = e
        .execute(&user(), tx("build", vec![seq, trk, clip]), 0)
        .unwrap();
    assert_eq!(r.refs.len(), 3);
    assert_eq!(r.refs["$clip"], "c");
    let seq_id = SequenceId::from(r.refs["$seq"].as_str());
    assert!(
        e.document()
            .sequence(&seq_id)
            .unwrap()
            .clip(&ClipId::from("c"))
            .is_some()
    );

    assert_eq!(
        code(e.execute(&user(), tx("t", vec![insert("r-4", "$nope", 0, "q", 5)]), 0)),
        ErrorCode::UnresolvedRef
    );
    let mut bad_ref = insert("r-5", &r.refs["$trk"], 50, "w", 5);
    bad_ref.reference = Some("nodollar".into());
    assert_eq!(
        code(e.execute(&user(), tx("t", vec![bad_ref]), 0)),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn max_ops_caps_degenerate_transactions() {
    let mut e = engine();
    let mut t = tx(
        "many",
        (0..10)
            .map(|i| insert(&format!("m-{i}"), "V3", i * 10, &format!("m{i}"), 5))
            .collect(),
    );
    t.max_ops = Some(5);
    let err = e.execute(&user(), t, 0).unwrap_err();
    assert_eq!(err.code, ErrorCode::LimitExceeded);
    assert!(clip_ids(&e, "V3").is_empty());
}

#[test]
fn undo_redo_walk_the_linear_history_and_new_edits_drop_the_redo_branch() {
    let mut e = engine();
    assert_eq!(code(e.redo(&user(), 0)), ErrorCode::NothingToRedo);
    e.execute(&user(), tx("a", vec![insert("a", "V2", 0, "a", 5)]), 1)
        .unwrap();
    e.execute(&user(), tx("b", vec![insert("b", "V2", 10, "b", 5)]), 2)
        .unwrap();
    e.undo(&user(), 3).unwrap();
    assert_eq!(clip_ids(&e, "V2"), ["a"]);
    assert!(e.can_redo());
    e.execute(&user(), tx("c", vec![insert("c", "V2", 20, "c", 5)]), 4)
        .unwrap();
    assert!(!e.can_redo(), "a new edit discards the redo branch");
    assert_eq!(code(e.redo(&user(), 5)), ErrorCode::NothingToRedo);
    assert_eq!(clip_ids(&e, "V2"), ["a", "c"]);
    // o histórico guarda só o ramo ativo; a auditoria guarda tudo
    assert_eq!(
        e.applied_history()
            .iter()
            .map(|h| h.label.as_str())
            .collect::<Vec<_>>(),
        ["setup", "a", "c"]
    );
    assert_eq!(e.audit_log().len(), 5);
    // desfazer até o início e além
    while e.can_undo() {
        e.undo(&user(), 6).unwrap();
    }
    assert_eq!(code(e.undo(&user(), 6)), ErrorCode::NothingToUndo);
    assert_eq!(e.document().sequence_count(), 0);
}

#[test]
fn undo_is_a_new_revision_and_replays_recorded_inverse_ops() {
    let mut e = engine();
    e.execute(&user(), tx("a", vec![insert("a", "V2", 0, "a", 5)]), 0)
        .unwrap();
    let rev = e.revision();
    let r = e.undo(&user(), 0).unwrap();
    assert_eq!(r.revision, rev + 1, "revisions are monotonic");
    assert_eq!(e.revision(), rev + 1);
}

#[test]
fn ripple_conflicts_are_structured_and_actionable() {
    let mut e = engine();
    e.execute(
        &user(),
        tx(
            "seed",
            vec![
                insert("1", "V1", 0, "a", 30),
                insert("2", "V1", 30, "b", 30),
                insert("3", "V1", 60, "c", 30),
                insert("4", "V2", 20, "y", 30),
            ],
        ),
        0,
    )
    .unwrap();
    let err = e
        .execute(
            &user(),
            tx(
                "t",
                vec![env(
                    "d",
                    Command::DeleteClip {
                        clip: "b".into(),
                        ripple: Some(true),
                        scope: RippleScope::Sequence,
                    },
                )],
            ),
            0,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RippleConflict);
    assert_eq!(err.entities, vec![EntityRef::new(EntityKind::Clip, "y")]);
    let hint = err.hint.unwrap();
    assert_eq!(hint["conflicts"][0]["track"], "V2");
    assert_eq!(hint["range_ticks"][0], 30 * FRAME);
    // sync_lock=false na track conflitante resolve (o hint sugere isso)
    e.execute(
        &user(),
        tx(
            "unsync",
            vec![env(
                "u",
                Command::SetTrackFlags {
                    track: "V2".into(),
                    locked: None,
                    hidden: None,
                    muted: None,
                    solo: None,
                    magnetic: None,
                    sync_lock: Some(false),
                    group: None,
                    clear_group: false,
                    compact: false,
                },
            )],
        ),
        0,
    )
    .unwrap();
    e.execute(
        &user(),
        tx(
            "t",
            vec![env(
                "d2",
                Command::DeleteClip {
                    clip: "b".into(),
                    ripple: Some(true),
                    scope: RippleScope::Sequence,
                },
            )],
        ),
        0,
    )
    .unwrap();
    assert_eq!(clip_ids(&e, "V1"), ["a", "c"]);
}

#[test]
fn locked_tracks_reject_edits_but_can_be_unlocked() {
    let mut e = engine();
    e.execute(&user(), tx("seed", vec![insert("1", "V2", 0, "a", 30)]), 0)
        .unwrap();
    let flags = |op: &str, locked: Option<bool>, hidden: Option<bool>| {
        env(
            op,
            Command::SetTrackFlags {
                track: "V2".into(),
                locked,
                hidden,
                muted: None,
                solo: None,
                magnetic: None,
                sync_lock: None,
                group: None,
                clear_group: false,
                compact: false,
            },
        )
    };
    e.execute(&user(), tx("lock", vec![flags("l", Some(true), None)]), 0)
        .unwrap();
    for c in [
        Command::TrimClip {
            clip: "a".into(),
            edge: Edge::Out,
            to: frames(10),
            ripple: None,
            scope: RippleScope::Track,
        },
        Command::SplitClip {
            clip: "a".into(),
            at: frames(10),
            new_id: None,
        },
        Command::SetProperty {
            clip: "a".into(),
            prop: "opacity".into(),
            value: 0.5,
        },
    ] {
        assert_eq!(
            code(e.execute(&user(), tx("t", vec![env("x1", c)]), 0)),
            ErrorCode::TrackLocked
        );
    }
    assert_eq!(
        code(e.execute(&user(), tx("t", vec![flags("h", None, Some(true))]), 0)),
        ErrorCode::TrackLocked
    );
    e.execute(
        &user(),
        tx("unlock", vec![flags("u", Some(false), None)]),
        0,
    )
    .unwrap();
    e.execute(
        &user(),
        tx(
            "t",
            vec![env(
                "x2",
                Command::SplitClip {
                    clip: "a".into(),
                    at: frames(10),
                    new_id: None,
                },
            )],
        ),
        0,
    )
    .unwrap();
}
