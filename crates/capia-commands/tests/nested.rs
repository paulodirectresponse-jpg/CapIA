//! Comandos de nested sequences (ADR-045): DAG, profundidade, referência inválida, delete,
//! retarget, `follow_length`, undo/redo e idempotência.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Command, CommandEnvelope, CommandError, Engine, NewClip, Transaction};
use capia_model::{ClipContent, ClipId, ErrorCode, SequenceId, Track, TrackId, TrackKind};
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

struct Ids(u32);
impl Ids {
    fn op(&mut self) -> String {
        self.0 += 1;
        format!("op-{}", self.0)
    }
}

struct World {
    e: Engine,
    ids: Ids,
}

fn sid(n: usize) -> SequenceId {
    SequenceId::from(format!("s{n}").as_str())
}

impl World {
    /// `n` sequences s0..: cada uma com `t{i}` (livre) e `m{i}` (magnética).
    fn new(n: usize) -> Self {
        let mut w = Self {
            e: Engine::new(Default::default(), [4; 32]),
            ids: Ids(0),
        };
        let mut cmds = Vec::new();
        for i in 0..n {
            cmds.push(Command::CreateSequence {
                id: Some(sid(i)),
                name: format!("seq {i}"),
                frame_rate: FrameRate::FPS_30,
                sample_rate: None,
            });
            for (prefix, magnetic) in [("t", false), ("m", true)] {
                cmds.push(Command::AddTrack {
                    sequence: sid(i),
                    id: Some(format!("{prefix}{i}").as_str().into()),
                    kind: TrackKind::Visual,
                    name: None,
                    role: None,
                    magnetic,
                    index: None,
                });
            }
        }
        w.run(cmds).unwrap();
        w
    }

    fn run(&mut self, cmds: Vec<Command>) -> Result<capia_commands::CommitResult, CommandError> {
        let commands = cmds.into_iter().map(|c| env(&self.ids.op(), c)).collect();
        self.e.execute(
            &Actor::user("t"),
            Transaction {
                transaction_id: None,
                label: "t".into(),
                base_revision: None,
                commands,
                max_ops: None,
            },
            0,
        )
    }

    fn solid(
        &mut self,
        track: &str,
        start: i64,
        dur: i64,
        id: &str,
    ) -> Result<capia_commands::CommitResult, CommandError> {
        self.run(vec![Command::InsertClip {
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
        }])
    }

    fn nest(
        &mut self,
        parent: usize,
        target: usize,
        start: i64,
        id: &str,
        follow: bool,
    ) -> Result<capia_commands::CommitResult, CommandError> {
        self.run(vec![Command::InsertNested {
            track: format!("t{parent}").as_str().into(),
            start: frames(start),
            sequence: sid(target),
            id: Some(id.into()),
            name: String::new(),
            duration: None,
            source_in: Ticks::ZERO,
            follow_length: follow,
            split_at_insert: false,
            split_new_id: None,
        }])
    }

    fn clip(&self, seq: usize, id: &str) -> Option<capia_model::Clip> {
        self.e
            .document()
            .sequence(&sid(seq))
            .unwrap()
            .clip(&ClipId::from(id))
            .cloned()
    }

    fn dur(&self, seq: usize, id: &str) -> i64 {
        self.clip(seq, id).unwrap().duration.0 / FRAME
    }
}

/// Digest do estado ignorando o contador de revisão (undo/redo avançam a revisão).
fn sem(d: &capia_model::Document) -> String {
    let mut d = d.clone();
    d.revision = 0;
    capia_commands::document_digest(&d)
}

fn err(r: Result<capia_commands::CommitResult, CommandError>) -> CommandError {
    r.unwrap_err()
}

#[test]
fn nesting_a_sequence_derives_its_duration_and_supports_undo_redo() {
    let mut w = World::new(2);
    w.solid("t1", 0, 90, "x").unwrap();
    let r = w.nest(0, 1, 10, "n", false).unwrap();
    assert_eq!(r.results[0].id.as_deref(), Some("n"));
    let c = w.clip(0, "n").unwrap();
    assert_eq!(
        (c.start.0 / FRAME, c.duration.0 / FRAME),
        (10, 90),
        "duração natural = a da filha"
    );
    assert!(matches!(
        c.content,
        ClipContent::Nested {
            follow_length: false,
            ..
        }
    ));
    let before = sem(w.e.document());
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert!(w.clip(0, "n").is_none());
    w.e.redo(&Actor::user("t"), 0).unwrap();
    assert_eq!(sem(w.e.document()), before);
}

#[test]
fn an_empty_child_gets_the_one_frame_minimum() {
    let mut w = World::new(2);
    w.nest(0, 1, 0, "n", false).unwrap();
    assert_eq!(w.dur(0, "n"), 1);
}

#[test]
fn chains_are_allowed_and_cycles_are_rejected_with_the_command_index() {
    let mut w = World::new(4);
    for i in 0..4 {
        w.solid(&format!("t{i}"), 0, 30, &format!("x{i}")).unwrap();
    }
    w.nest(0, 1, 40, "ab", false).unwrap(); // A -> B
    w.nest(1, 2, 40, "bc", false).unwrap(); // B -> C   (A -> B -> C)
    // A -> A
    let e = err(w.nest(0, 0, 100, "aa", false));
    assert_eq!(e.code, ErrorCode::NestedCycle);
    assert_eq!(e.command_index, Some(0));
    // B -> A  (A -> B -> A)
    let e = err(w.nest(1, 0, 100, "ba", false));
    assert_eq!(e.code, ErrorCode::NestedCycle);
    assert!(e.message.contains("s1 -> s0 -> s1"), "{}", e.message);
    // C -> A fecha A -> B -> C -> A
    assert_eq!(
        err(w.nest(2, 0, 100, "ca", false)).code,
        ErrorCode::NestedCycle
    );
    // diamante (A -> C direto) é válido
    w.nest(0, 2, 200, "ac", false).unwrap();
    // nada foi gravado pelas rejeitadas
    assert!(w.clip(0, "aa").is_none() && w.clip(1, "ba").is_none() && w.clip(2, "ca").is_none());
    // o caminho de insert_clip com conteúdo Nested bruto passa pela mesma guarda
    let raw = w.run(vec![Command::InsertClip {
        track: "t2".into(),
        start: frames(500),
        clip: NewClip {
            id: Some("raw".into()),
            name: String::new(),
            duration: frames(5),
            content: ClipContent::Nested {
                sequence: sid(0),
                follow_length: false,
            },
            source_in: Ticks::ZERO,
            speed: Rational::ONE,
            reversed: false,
            properties: Default::default(),
        },
        split_at_insert: false,
        split_new_id: None,
    }]);
    assert_eq!(err(raw).code, ErrorCode::NestedCycle);
}

#[test]
fn the_depth_limit_is_exact_from_both_ends() {
    let hops = capia_model::MAX_NESTING_DEPTH;
    let mut w = World::new(hops + 3);
    for i in 0..hops {
        w.nest(i, i + 1, 0, &format!("n{i}"), false).unwrap(); // s0 -> … -> s16
    }
    // 17º salto por baixo
    let e = err(w.nest(hops, hops + 1, 0, "too_deep", false));
    assert_eq!(e.code, ErrorCode::NestedDepth);
    // 17º salto por cima: uma nova raiz apontando para a cabeça da cadeia
    let e = err(w.nest(hops + 2, 0, 0, "too_high", false));
    assert_eq!(e.code, ErrorCode::NestedDepth);
    // um atalho que não aumenta a profundidade continua válido
    w.nest(0, hops, 1000, "shortcut", false).unwrap();
    assert!(w.clip(hops, "too_deep").is_none());
}

#[test]
fn invalid_targets_are_rejected() {
    let mut w = World::new(2);
    let e = err(w.run(vec![Command::InsertNested {
        track: "t0".into(),
        start: Ticks::ZERO,
        sequence: "ghost".into(),
        id: None,
        name: String::new(),
        duration: None,
        source_in: Ticks::ZERO,
        follow_length: false,
        split_at_insert: false,
        split_new_id: None,
    }]));
    assert_eq!(e.code, ErrorCode::NotFound);
    let raw = err(w.run(vec![Command::InsertClip {
        track: "t0".into(),
        start: Ticks::ZERO,
        clip: NewClip {
            id: Some("r".into()),
            name: String::new(),
            duration: frames(5),
            content: ClipContent::Nested {
                sequence: "ghost".into(),
                follow_length: false,
            },
            source_in: Ticks::ZERO,
            speed: Rational::ONE,
            reversed: false,
            properties: Default::default(),
        },
        split_at_insert: false,
        split_new_id: None,
    }]));
    assert_eq!(raw.code, ErrorCode::DanglingReference);
    // não-nested
    w.solid("t0", 0, 10, "plain").unwrap();
    for c in [
        Command::SetNestedTarget {
            clip: "plain".into(),
            sequence: sid(1),
        },
        Command::SetFollowLength {
            clip: "plain".into(),
            follow_length: true,
        },
    ] {
        assert_eq!(err(w.run(vec![c])).code, ErrorCode::InvalidArgument);
    }
}

#[test]
fn nested_clips_move_trim_split_and_delete_like_any_clip() {
    let mut w = World::new(2);
    w.solid("t1", 0, 60, "x").unwrap();
    w.nest(0, 1, 0, "n", false).unwrap();
    w.run(vec![Command::MoveClips {
        moves: vec![capia_commands::ClipMove {
            clip: "n".into(),
            track: None,
            start: frames(100),
        }],
    }])
    .unwrap();
    assert_eq!(w.clip(0, "n").unwrap().start, frames(100));
    w.run(vec![Command::TrimClip {
        clip: "n".into(),
        edge: capia_commands::Edge::Out,
        to: frames(130),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert_eq!(w.dur(0, "n"), 30);
    w.run(vec![Command::SplitClip {
        clip: "n".into(),
        at: frames(110),
        new_id: Some("n2".into()),
    }])
    .unwrap();
    assert_eq!((w.dur(0, "n"), w.dur(0, "n2")), (10, 20));
    assert_eq!(
        w.clip(0, "n2").unwrap().source_in,
        frames(10),
        "o offset na filha avança com o split"
    );
    w.run(vec![Command::DeleteClip {
        clip: "n".into(),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert!(w.clip(0, "n").is_none());
    assert!(
        w.e.document()
            .sequence(&sid(0))
            .unwrap()
            .index_is_consistent()
    );
}

#[test]
fn delete_sequence_refuses_when_in_use_and_undoes_completely() {
    let mut w = World::new(3);
    w.solid("t1", 0, 30, "x").unwrap();
    w.run(vec![Command::AddMarker {
        sequence: sid(1),
        id: Some("mk".into()),
        time: frames(5),
        label: String::new(),
    }])
    .unwrap();
    w.nest(0, 1, 0, "n", false).unwrap();
    let e = err(w.run(vec![Command::DeleteSequence { sequence: sid(1) }]));
    assert_eq!(e.code, ErrorCode::InUse);
    assert_eq!(
        e.entities,
        vec![capia_model::EntityRef::new(
            capia_model::EntityKind::Clip,
            "n"
        )]
    );
    assert_eq!(e.hint.as_ref().unwrap()["used_by"][0]["sequence"], "s0");
    // sem o uso, apaga tudo (clips, marcadores, tracks, sequence)
    w.run(vec![Command::DeleteClip {
        clip: "n".into(),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    let before = sem(w.e.document());
    w.run(vec![Command::DeleteSequence { sequence: sid(1) }])
        .unwrap();
    assert!(w.e.document().sequence(&sid(1)).is_none());
    // e o undo devolve tudo, bit a bit
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(sem(w.e.document()), before);
    assert!(w.clip(1, "x").is_some());
    assert_eq!(w.e.document().sequence(&sid(1)).unwrap().marker_count(), 1);
    w.e.redo(&Actor::user("t"), 0).unwrap();
    assert!(w.e.document().sequence(&sid(1)).is_none());
    // sequence inexistente
    assert_eq!(
        err(w.run(vec![Command::DeleteSequence {
            sequence: "ghost".into()
        }]))
        .code,
        ErrorCode::NotFound
    );
}

#[test]
fn delete_sequence_refuses_locked_tracks() {
    let mut w = World::new(2);
    w.run(vec![Command::SetTrackFlags {
        track: "t1".into(),
        locked: Some(true),
        hidden: None,
        muted: None,
        solo: None,
        magnetic: None,
        sync_lock: None,
        group: None,
        clear_group: false,
        compact: false,
    }])
    .unwrap();
    assert_eq!(
        err(w.run(vec![Command::DeleteSequence { sequence: sid(1) }])).code,
        ErrorCode::TrackLocked
    );
}

#[test]
fn rename_sequence_validates_and_is_undoable() {
    let mut w = World::new(1);
    w.run(vec![Command::RenameSequence {
        sequence: sid(0),
        name: "  HOOK 1  ".into(),
    }])
    .unwrap();
    assert_eq!(
        w.e.document().sequence(&sid(0)).unwrap().header.name,
        "HOOK 1"
    );
    assert_eq!(
        err(w.run(vec![Command::RenameSequence {
            sequence: sid(0),
            name: "   ".into()
        }]))
        .code,
        ErrorCode::InvalidArgument
    );
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(
        w.e.document().sequence(&sid(0)).unwrap().header.name,
        "seq 0"
    );
}

#[test]
fn retargeting_validates_the_graph_without_the_old_edge() {
    let hops = capia_model::MAX_NESTING_DEPTH;
    let mut w = World::new(hops + 2);
    for i in 0..hops {
        w.nest(i, i + 1, 0, &format!("n{i}"), false).unwrap();
    }
    // s0 -> s1 -> … -> s16 (16 saltos). Retargetar o último elo para s17 troca a ponta sem aumentar:
    w.run(vec![Command::SetNestedTarget {
        clip: format!("n{}", hops - 1).as_str().into(),
        sequence: sid(hops + 1),
    }])
    .unwrap();
    // ciclo por retarget: s5 -> s2
    let e = err(w.run(vec![Command::SetNestedTarget {
        clip: "n5".into(),
        sequence: sid(2),
    }]));
    assert_eq!(e.code, ErrorCode::NestedCycle);
    // retarget para a própria sequence
    assert_eq!(
        err(w.run(vec![Command::SetNestedTarget {
            clip: "n5".into(),
            sequence: sid(5)
        }]))
        .code,
        ErrorCode::NestedCycle
    );
    // alvo inexistente
    assert_eq!(
        err(w.run(vec![Command::SetNestedTarget {
            clip: "n5".into(),
            sequence: "ghost".into()
        }]))
        .code,
        ErrorCode::NotFound
    );
    // mesmo alvo = no-op bem-sucedido
    w.run(vec![Command::SetNestedTarget {
        clip: "n5".into(),
        sequence: sid(6),
    }])
    .unwrap();
}

#[test]
fn operation_ids_make_nested_edits_idempotent() {
    let mut w = World::new(2);
    let tx = Transaction {
        transaction_id: None,
        label: "n".into(),
        base_revision: None,
        commands: vec![env(
            "nest-1",
            Command::InsertNested {
                track: "t0".into(),
                start: Ticks::ZERO,
                sequence: sid(1),
                id: None,
                name: String::new(),
                duration: Some(frames(10)),
                source_in: Ticks::ZERO,
                follow_length: false,
                split_at_insert: false,
                split_new_id: None,
            },
        )],
        max_ops: None,
    };
    let first = w.e.execute(&Actor::user("t"), tx.clone(), 0).unwrap();
    let rev = w.e.revision();
    let again = w.e.execute(&Actor::user("t"), tx, 0).unwrap();
    assert!(again.replayed);
    assert_eq!(w.e.revision(), rev);
    assert_eq!(
        again.results[0].id, first.results[0].id,
        "o id derivado é o mesmo"
    );
}

// ---- follow_length --------------------------------------------------------------------------

#[test]
fn follow_length_extends_the_parent_clip_in_the_same_transaction_as_the_master_edit() {
    let mut w = World::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    w.nest(0, 1, 0, "n", true).unwrap();
    assert_eq!(w.dur(0, "n"), 30);
    let entries = w.e.history().len();
    // editar o master: o nested do pai acompanha, num único passo de histórico
    w.solid("t1", 30, 20, "y").unwrap();
    assert_eq!(w.dur(0, "n"), 50);
    assert_eq!(w.e.history().len(), entries + 1);
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(
        w.dur(0, "n"),
        30,
        "um undo reverte master e seguidor juntos"
    );
    assert!(w.clip(1, "y").is_none());
    w.e.redo(&Actor::user("t"), 0).unwrap();
    assert_eq!(w.dur(0, "n"), 50);
    // encurtar o master também encurta
    w.run(vec![Command::DeleteClip {
        clip: "y".into(),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert_eq!(w.dur(0, "n"), 30);
}

#[test]
fn follow_length_ripples_a_magnetic_parent_track() {
    let mut w = World::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    // pai magnético: [nested(follow)] [after]
    let id = |s: &str| ClipId::from(s);
    w.run(vec![Command::InsertNested {
        track: "m0".into(),
        start: Ticks::ZERO,
        sequence: sid(1),
        id: Some(id("n")),
        name: String::new(),
        duration: None,
        source_in: Ticks::ZERO,
        follow_length: true,
        split_at_insert: false,
        split_new_id: None,
    }])
    .unwrap();
    w.run(vec![Command::InsertClip {
        track: "m0".into(),
        start: frames(30),
        clip: NewClip {
            id: Some(id("after")),
            name: String::new(),
            duration: frames(10),
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
    }])
    .unwrap();
    w.solid("t1", 30, 20, "y").unwrap();
    assert_eq!(w.dur(0, "n"), 50);
    assert_eq!(
        w.clip(0, "after").unwrap().start,
        frames(50),
        "posterior sofreu ripple"
    );
    w.run(vec![Command::DeleteClip {
        clip: "y".into(),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert_eq!(w.clip(0, "after").unwrap().start, frames(30));
}

#[test]
fn follow_length_on_a_free_track_is_limited_by_the_next_clip_and_warns() {
    let mut w = World::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    w.nest(0, 1, 0, "n", true).unwrap();
    w.solid("t0", 40, 10, "wall").unwrap(); // obstáculo a 40
    let r = w.solid("t1", 30, 30, "y").unwrap(); // master passa a 60 frames
    assert_eq!(w.dur(0, "n"), 40, "limitado pelo espaço livre");
    assert!(
        r.results[0]
            .warnings
            .iter()
            .any(|m| m.contains("cannot follow")),
        "{:?}",
        r.results[0].warnings
    );
    // liberou o espaço: a reconciliação seguinte volta a acompanhar
    w.run(vec![Command::DeleteClip {
        clip: "wall".into(),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert_eq!(w.dur(0, "n"), 60);
}

#[test]
fn follow_length_leaves_locked_tracks_alone_and_warns() {
    let mut w = World::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    w.nest(0, 1, 0, "n", true).unwrap();
    w.run(vec![Command::SetTrackFlags {
        track: "t0".into(),
        locked: Some(true),
        hidden: None,
        muted: None,
        solo: None,
        magnetic: None,
        sync_lock: None,
        group: None,
        clear_group: false,
        compact: false,
    }])
    .unwrap();
    let r = w.solid("t1", 30, 20, "y").unwrap();
    assert_eq!(w.dur(0, "n"), 30);
    assert!(r.results[0].warnings.iter().any(|m| m.contains("locked")));
}

#[test]
fn follow_length_propagates_through_two_levels_and_across_frame_rates() {
    // s2 (24 fps) <- s1 (30 fps) <- s0 (30 fps)
    let mut w = World::new(1);
    w.run(vec![
        Command::CreateSequence {
            id: Some(sid(1)),
            name: "mid".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: None,
        },
        Command::AddTrack {
            sequence: sid(1),
            id: Some("t1".into()),
            kind: TrackKind::Visual,
            name: None,
            role: None,
            magnetic: false,
            index: None,
        },
        Command::CreateSequence {
            id: Some(sid(2)),
            name: "leaf".into(),
            frame_rate: FrameRate::FPS_24,
            sample_rate: None,
        },
        Command::AddTrack {
            sequence: sid(2),
            id: Some("t2".into()),
            kind: TrackKind::Visual,
            name: None,
            role: None,
            magnetic: false,
            index: None,
        },
    ])
    .unwrap();
    w.nest(1, 2, 0, "mid_leaf", true).unwrap();
    w.nest(0, 1, 0, "top_mid", true).unwrap();
    // 24 fps: 24 frames = 1 s ; no pai de 30 fps vira 30 frames
    w.run(vec![Command::InsertClip {
        track: "t2".into(),
        start: Ticks::ZERO,
        clip: NewClip {
            id: Some("leafclip".into()),
            name: String::new(),
            duration: Ticks(24 * 29_400_000),
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
    }])
    .unwrap();
    assert_eq!(w.dur(1, "mid_leaf"), 30);
    assert_eq!(w.dur(0, "top_mid"), 30, "a mudança subiu dois níveis");
    // desligar a flag congela
    w.run(vec![Command::SetFollowLength {
        clip: "top_mid".into(),
        follow_length: false,
    }])
    .unwrap();
    w.run(vec![Command::TrimClip {
        clip: "leafclip".into(),
        edge: capia_commands::Edge::Out,
        to: Ticks(12 * 29_400_000),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert_eq!(w.dur(1, "mid_leaf"), 15);
    assert_eq!(
        w.dur(0, "top_mid"),
        30,
        "sem follow_length o clip mantém a duração"
    );
    // religar sincroniza de imediato
    w.run(vec![Command::SetFollowLength {
        clip: "top_mid".into(),
        follow_length: true,
    }])
    .unwrap();
    assert_eq!(w.dur(0, "top_mid"), 15);
}

#[test]
fn follow_length_respects_source_in_and_the_one_frame_minimum() {
    let mut w = World::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    w.run(vec![Command::InsertNested {
        track: "t0".into(),
        start: Ticks::ZERO,
        sequence: sid(1),
        id: Some("n".into()),
        name: String::new(),
        duration: None,
        source_in: frames(10),
        follow_length: true,
        split_at_insert: false,
        split_new_id: None,
    }])
    .unwrap();
    assert_eq!(w.dur(0, "n"), 20, "30 − source_in(10)");
    // master mais curto que source_in ⇒ mínimo de 1 frame
    w.run(vec![Command::TrimClip {
        clip: "x".into(),
        edge: capia_commands::Edge::Out,
        to: frames(5),
        ripple: None,
        scope: Default::default(),
    }])
    .unwrap();
    assert_eq!(w.dur(0, "n"), 1);
}

#[test]
fn nested_model_types_are_exported_for_tooling() {
    // garante que o índice derivado fica consistente depois de uma sequência de edições
    let mut w = World::new(3);
    w.nest(0, 1, 0, "a", true).unwrap();
    w.nest(0, 2, 100, "b", false).unwrap();
    w.e.undo(&Actor::user("t"), 0).unwrap();
    w.e.redo(&Actor::user("t"), 0).unwrap();
    for (_, s) in w.e.document().sequences() {
        assert!(s.index_is_consistent());
    }
    let _ = (TrackId::from("x"), Track::new("x", TrackKind::Visual));
}
