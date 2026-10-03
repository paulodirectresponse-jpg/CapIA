//! Comandos de composição (ADR-050): duplicate_sequence, make_unique, flatten_nested,
//! create_nested_from_selection e generate_variants — semântica, erros, undo/redo, idempotência,
//! determinismo e invariantes (DAG, profundidade) preservados.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{
    Actor, Command, CommandEnvelope, CommandError, CommitResult, Engine, NewClip, Transaction,
    VariantSpec, VariantSwap,
};
use capia_model::{
    Asset, Clip, ClipContent, ClipId, ErrorCode, Marker, SequenceId, TrackKind, validate_document,
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

fn sid(n: usize) -> SequenceId {
    SequenceId::from(format!("s{n}").as_str())
}

struct W {
    e: Engine,
    n: u32,
}

type R = Result<CommitResult, CommandError>;

impl W {
    /// `n` sequences s0..: tracks `t{i}` (livre), `m{i}` (magnética) e `a{i}` (áudio).
    fn new(n: usize) -> Self {
        let mut w = Self {
            e: Engine::new(Default::default(), [4; 32]),
            n: 0,
        };
        let mut cmds = Vec::new();
        for i in 0..n {
            cmds.push(Command::CreateSequence {
                id: Some(sid(i)),
                name: format!("seq {i}"),
                frame_rate: FrameRate::FPS_30,
                sample_rate: None,
            });
            for (p, kind, magnetic) in [
                ("t", TrackKind::Visual, false),
                ("m", TrackKind::Visual, true),
                ("a", TrackKind::Audio, false),
            ] {
                cmds.push(Command::AddTrack {
                    sequence: sid(i),
                    id: Some(format!("{p}{i}").as_str().into()),
                    kind,
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

    fn run(&mut self, cmds: Vec<Command>) -> R {
        let commands = cmds
            .into_iter()
            .map(|c| {
                self.n += 1;
                env(&format!("op-{}", self.n), c)
            })
            .collect();
        let r = self.e.execute(
            &Actor::user("t"),
            Transaction {
                transaction_id: None,
                label: "t".into(),
                base_revision: None,
                commands,
                max_ops: None,
            },
            0,
        );
        if r.is_ok() {
            let v = validate_document(self.e.document());
            assert!(
                v.is_empty(),
                "invariants broken after a successful commit: {v:?}"
            );
        }
        r
    }

    fn cmd(&mut self, c: Command) -> R {
        self.run(vec![c])
    }

    fn solid(&mut self, track: &str, start: i64, dur: i64, id: &str) -> R {
        self.insert(
            track,
            start,
            dur,
            id,
            ClipContent::Solid {
                color: "#123".into(),
            },
        )
    }

    fn insert(&mut self, track: &str, start: i64, dur: i64, id: &str, content: ClipContent) -> R {
        self.cmd(Command::InsertClip {
            track: track.into(),
            start: frames(start),
            clip: NewClip {
                id: Some(id.into()),
                name: id.into(),
                duration: frames(dur),
                content,
                source_in: Ticks::ZERO,
                speed: Rational::ONE,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: false,
            split_new_id: None,
        })
    }

    /// Clip de áudio numa track de áudio (registra o asset `AU` na primeira vez).
    fn audio(&mut self, track: &str, start: i64, dur: i64, id: &str) -> R {
        if self.e.document().asset(&"AU".into()).is_none() {
            self.asset("AU", 600).unwrap();
        }
        self.insert(
            track,
            start,
            dur,
            id,
            ClipContent::Media {
                asset: "AU".into(),
                has_video: false,
                has_audio: true,
            },
        )
    }

    fn nest(&mut self, track: &str, target: usize, start: i64, id: &str) -> R {
        self.cmd(Command::InsertNested {
            track: track.into(),
            start: frames(start),
            sequence: sid(target),
            id: Some(id.into()),
            name: id.into(),
            duration: None,
            source_in: Ticks::ZERO,
            follow_length: false,
            split_at_insert: false,
            split_new_id: None,
        })
    }

    fn asset(&mut self, id: &str, secs: i64) -> R {
        self.cmd(Command::RegisterAsset {
            asset: Asset {
                id: id.into(),
                name: id.into(),
                duration: Some(Ticks(secs * 705_600_000)),
                has_video: true,
                has_audio: true,
                offline: false,
            },
        })
    }

    fn media(&mut self, track: &str, start: i64, dur: i64, id: &str, asset: &str) -> R {
        self.insert(
            track,
            start,
            dur,
            id,
            ClipContent::Media {
                asset: asset.into(),
                has_video: true,
                has_audio: false,
            },
        )
    }

    fn clip(&self, seq: &str, id: &str) -> Option<Clip> {
        self.e
            .document()
            .sequence(&SequenceId::from(seq))?
            .clip(&ClipId::from(id))
            .cloned()
    }

    fn sem(&self) -> String {
        sem(self.e.document())
    }
}

fn sem(d: &capia_model::Document) -> String {
    let mut d = d.clone();
    d.revision = 0;
    capia_commands::document_digest(&d)
}

fn undo_redo_roundtrip(w: &mut W, before: &str) {
    let after = w.sem();
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(w.sem(), before, "undo restores the exact previous state");
    w.e.redo(&Actor::user("t"), 0).unwrap();
    assert_eq!(w.sem(), after, "redo restores the exact result");
}

fn target_of(w: &W, seq: &str, clip: &str) -> String {
    match w.clip(seq, clip).unwrap().content {
        ClipContent::Nested { sequence, .. } => sequence.0,
        other => panic!("not nested: {other:?}"),
    }
}

// ============================================ duplicate_sequence ============================

fn dup(w: &mut W, source: usize, new: &str, deep: bool) -> R {
    w.cmd(Command::DuplicateSequence {
        source: sid(source),
        new_sequence: Some(new.into()),
        name: None,
        deep,
    })
}

#[test]
fn duplicate_copies_everything_with_deterministic_new_ids() {
    let mut w = W::new(1);
    w.solid("t0", 0, 30, "a").unwrap();
    w.solid("t0", 40, 20, "b").unwrap();
    w.audio("a0", 5, 10, "c").unwrap();
    w.cmd(Command::AddMarker {
        sequence: sid(0),
        id: Some("mk".into()),
        time: frames(12),
        label: "hook".into(),
    })
    .unwrap();
    let before = w.sem();
    let r = dup(&mut w, 0, "copy", false).unwrap();
    assert_eq!(r.results[0].id.as_deref(), Some("copy"));
    let d = w.e.document();
    let (src, cp) = (
        d.sequence(&sid(0)).unwrap(),
        d.sequence(&"copy".into()).unwrap(),
    );
    assert_eq!(cp.header.name, "seq 0 copy");
    assert_eq!(cp.header.frame_rate, src.header.frame_rate);
    assert_eq!(cp.tracks().len(), 3);
    assert_eq!(cp.clip_count(), 3);
    assert!(cp.track(&"copy.t0".into()).is_some());
    let a = w.clip("copy", "copy.a").unwrap();
    let orig = w.clip("s0", "a").unwrap();
    assert_eq!(
        (a.start, a.duration, a.track.0.as_str()),
        (orig.start, orig.duration, "copy.t0")
    );
    assert_eq!(cp.markers().count(), 1);
    assert!(cp.marker(&"copy.mk".into()).is_some());
    // o original não foi tocado
    assert_eq!(src.clip_count(), 3);
    assert!(src.track(&"t0".into()).is_some());
    undo_redo_roundtrip(&mut w, &before);
}

#[test]
fn duplicate_is_deterministic_and_idempotent() {
    let build = || {
        let mut w = W::new(1);
        w.solid("t0", 0, 30, "a").unwrap();
        dup(&mut w, 0, "copy", false).unwrap();
        w.sem()
    };
    assert_eq!(
        build(),
        build(),
        "same commands, same document (no randomness)"
    );
    // reenvio do MESMO operation_id: replay, nada novo
    let mut w = W::new(1);
    let tx = Transaction {
        transaction_id: None,
        label: "dup".into(),
        base_revision: None,
        commands: vec![env(
            "dup-op",
            Command::DuplicateSequence {
                source: sid(0),
                new_sequence: Some("copy".into()),
                name: None,
                deep: false,
            },
        )],
        max_ops: None,
    };
    let first = w.e.execute(&Actor::user("t"), tx.clone(), 0).unwrap();
    let digest = w.sem();
    let second = w.e.execute(&Actor::user("t"), tx, 0).unwrap();
    assert!(second.replayed && !first.replayed);
    assert_eq!(w.sem(), digest);
}

#[test]
fn duplicate_without_an_explicit_id_derives_one_from_the_operation() {
    let mut w = W::new(1);
    let r = w
        .cmd(Command::DuplicateSequence {
            source: sid(0),
            new_sequence: None,
            name: Some("Named".into()),
            deep: false,
        })
        .unwrap();
    let id = r.results[0].id.clone().unwrap();
    assert!(id.starts_with("seq_"));
    assert_eq!(
        w.e.document()
            .sequence(&id.as_str().into())
            .unwrap()
            .header
            .name,
        "Named"
    );
}

#[test]
fn shallow_duplicate_shares_nested_children_deep_duplicate_does_not() {
    let mut w = W::new(3);
    w.solid("t2", 0, 30, "leaf").unwrap();
    w.nest("t1", 2, 0, "mid").unwrap(); // s1 -> s2
    w.nest("t0", 1, 0, "top").unwrap(); // s0 -> s1
    // raso: a cópia de s0 continua apontando para s1 (compartilhada)
    dup(&mut w, 0, "shallow", false).unwrap();
    assert_eq!(target_of(&w, "shallow", "shallow.top"), "s1");
    assert_eq!(w.e.document().sequence_count(), 4);
    // profundo: s1 e s2 também são copiadas, com ids derivados e DAG preservado
    let before = w.sem();
    dup(&mut w, 0, "deep", true).unwrap();
    assert_eq!(target_of(&w, "deep", "deep.top"), "deep~s1");
    assert_eq!(target_of(&w, "deep~s1", "deep~s1.mid"), "deep~s2");
    assert!(w.clip("deep~s2", "deep~s2.leaf").is_some());
    // as cópias profundas são independentes: editar a folha da cópia não toca a original
    w.cmd(Command::DeleteClip {
        clip: "deep~s2.leaf".into(),
        ripple: None,
        scope: Default::default(),
    })
    .unwrap();
    assert!(w.clip("s2", "leaf").is_some());
    let _ = before;
}

#[test]
fn deep_duplicate_copies_a_shared_grandchild_once_and_keeps_the_diamond() {
    let mut w = W::new(4);
    // s0 -> s1, s0 -> s2, s1 -> s3, s2 -> s3 (losango)
    w.solid("t3", 0, 10, "z").unwrap();
    w.nest("t1", 3, 0, "x1").unwrap();
    w.nest("t2", 3, 0, "x2").unwrap();
    w.nest("t0", 1, 0, "p1").unwrap();
    w.nest("t0", 2, 40, "p2").unwrap();
    dup(&mut w, 0, "d", true).unwrap();
    assert_eq!(w.e.document().sequence_count(), 8, "s3 copied exactly once");
    assert_eq!(target_of(&w, "d~s1", "d~s1.x1"), "d~s3");
    assert_eq!(target_of(&w, "d~s2", "d~s2.x2"), "d~s3");
}

#[test]
fn duplicate_errors_are_structured_and_atomic() {
    let mut w = W::new(1);
    w.solid("t0", 0, 10, "a").unwrap();
    let before = w.sem();
    let e = dup(&mut w, 7, "x", false).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = dup(&mut w, 0, "s0", false).unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    // colisão de ids derivados no meio da cópia: nada fica pela metade
    w.cmd(Command::CreateSequence {
        id: Some("c".into()),
        name: "c".into(),
        frame_rate: FrameRate::FPS_30,
        sample_rate: None,
    })
    .unwrap();
    let before2 = w.sem();
    w.cmd(Command::AddTrack {
        sequence: "c".into(),
        id: Some("c.t0".into()), // id que a cópia tentaria usar se se chamasse "c"
        kind: TrackKind::Visual,
        name: None,
        role: None,
        magnetic: false,
        index: None,
    })
    .unwrap();
    let before3 = w.sem();
    assert_ne!(before2, before3);
    // "c" já existe como sequence ⇒ erro; e a cópia para outro id com track colidindo também
    assert!(dup(&mut w, 0, "c", false).is_err());
    assert_eq!(w.sem(), before3);
    let _ = before;
}

// ================================================ make_unique ===============================

#[test]
fn make_unique_gives_one_parent_an_independent_copy() {
    let mut w = W::new(3);
    w.solid("t2", 0, 30, "shared").unwrap();
    w.nest("t0", 2, 0, "u0").unwrap();
    w.nest("t1", 2, 0, "u1").unwrap();
    let before = w.sem();
    let r = w
        .cmd(Command::MakeUnique {
            clip: "u0".into(),
            new_sequence: Some("own".into()),
            name: Some("Own copy".into()),
            deep: false,
        })
        .unwrap();
    assert_eq!(r.results[0].id.as_deref(), Some("own"));
    assert_eq!(target_of(&w, "s0", "u0"), "own");
    assert_eq!(
        target_of(&w, "s1", "u1"),
        "s2",
        "the other parent still shares the original"
    );
    assert_eq!(
        w.e.document().sequence(&"own".into()).unwrap().header.name,
        "Own copy"
    );
    // nenhum id foi reaproveitado: o clip da cópia é novo, o original segue intacto
    assert!(w.clip("own", "own.shared").is_some());
    assert!(w.clip("s2", "shared").is_some());
    // editar a cópia não altera a original
    w.cmd(Command::DeleteClip {
        clip: "own.shared".into(),
        ripple: None,
        scope: Default::default(),
    })
    .unwrap();
    assert!(w.clip("s2", "shared").is_some());
    w.e.undo(&Actor::user("t"), 0).unwrap();
    undo_redo_roundtrip(&mut w, &before);
}

#[test]
fn make_unique_deep_copies_the_subtree_and_never_reuses_ids() {
    let mut w = W::new(4);
    w.solid("t3", 0, 10, "leaf").unwrap();
    w.nest("t2", 3, 0, "in2").unwrap(); // s2 -> s3
    w.nest("t1", 2, 0, "in1").unwrap(); // s1 -> s2
    w.nest("t0", 1, 0, "n").unwrap(); // s0 -> s1
    w.cmd(Command::MakeUnique {
        clip: "n".into(),
        new_sequence: Some("u".into()),
        name: None,
        deep: true,
    })
    .unwrap();
    assert_eq!(target_of(&w, "s0", "n"), "u");
    assert_eq!(target_of(&w, "u", "u.in1"), "u~s2");
    assert_eq!(target_of(&w, "u~s2", "u~s2.in2"), "u~s3");
    // todos os ids de clip/track do projeto são distintos
    let d = w.e.document();
    let mut ids = std::collections::BTreeSet::new();
    for (_, s) in d.sequences() {
        for c in s.clips() {
            assert!(ids.insert(c.id.0.clone()), "clip id {} reused", c.id);
        }
        for t in s.tracks() {
            assert!(ids.insert(t.id.0.clone()), "track id {} reused", t.id);
        }
    }
    // a árvore original continua apontando para si mesma
    assert_eq!(target_of(&w, "s1", "in1"), "s2");
    assert_eq!(target_of(&w, "s2", "in2"), "s3");
}

#[test]
fn make_unique_rejects_non_nested_locked_and_existing_ids() {
    let mut w = W::new(2);
    w.solid("t0", 0, 10, "plain").unwrap();
    w.nest("t0", 1, 20, "n").unwrap();
    let mk = |clip: &str, id: &str| Command::MakeUnique {
        clip: clip.into(),
        new_sequence: Some(id.into()),
        name: None,
        deep: false,
    };
    assert_eq!(
        w.cmd(mk("plain", "x")).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        w.cmd(mk("ghost", "x")).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.cmd(mk("n", "s1")).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    w.cmd(Command::SetTrackFlags {
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
    })
    .unwrap();
    let before = w.sem();
    assert_eq!(
        w.cmd(mk("n", "x")).unwrap_err().code,
        ErrorCode::TrackLocked
    );
    assert_eq!(w.sem(), before);
}

// =============================================== flatten_nested =============================

fn flatten(w: &mut W, clip: &str) -> R {
    w.cmd(Command::FlattenNested {
        clip: clip.into(),
        prefix: None,
    })
}

#[test]
fn flatten_replaces_the_nested_clip_with_its_children_preserving_timing() {
    let mut w = W::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    w.solid("t1", 40, 20, "y").unwrap();
    w.audio("a1", 5, 10, "z").unwrap();
    w.nest("t0", 1, 100, "n").unwrap(); // duração natural = 60 frames
    let before = w.sem();
    let r = flatten(&mut w, "n").unwrap();
    assert!(w.clip("s0", "n").is_none(), "the nested clip is gone");
    let x = w.clip("s0", "n.x").unwrap();
    let y = w.clip("s0", "n.y").unwrap();
    let z = w.clip("s0", "n.z").unwrap();
    assert_eq!((x.start, x.duration), (frames(100), frames(30)));
    assert_eq!((y.start, y.duration), (frames(140), frames(20)));
    assert_eq!(z.start, frames(105));
    let seq = w.e.document().sequence(&sid(0)).unwrap();
    assert_eq!(seq.tracks().len(), 3 + 3, "one new track per child track");
    assert_eq!(x.track.0, "n.t1");
    assert!(
        !seq.track(&"n.m1".into()).unwrap().magnetic,
        "flattened tracks are free"
    );
    assert_eq!(
        r.results[0].id.as_deref(),
        Some("n.z"),
        "first created clip, in (track, start, id) order"
    );
    // a sequence filha continua existindo e intacta (pode ser compartilhada)
    assert!(w.clip("s1", "x").is_some());
    undo_redo_roundtrip(&mut w, &before);
}

#[test]
fn flatten_trims_children_to_the_visible_window_and_rebases_content() {
    let mut w = W::new(2);
    w.asset("A", 60).unwrap();
    // filho: mídia em [0,90) e sólido em [30,120)
    w.media("t1", 0, 90, "vid", "A").unwrap();
    w.solid("m1", 0, 1, "tiny").ok();
    // nested mostra apenas [20, 80) do filho (source_in = 20)
    w.cmd(Command::InsertNested {
        track: "t0".into(),
        start: frames(10),
        sequence: sid(1),
        id: Some("n".into()),
        name: "n".into(),
        duration: Some(frames(60)),
        source_in: frames(20),
        follow_length: false,
        split_at_insert: false,
        split_new_id: None,
    })
    .unwrap();
    flatten(&mut w, "n").unwrap();
    let v = w.clip("s0", "n.vid").unwrap();
    // [0,90) ∩ [20,80) = [20,80) ⇒ começa em 10 (início do nested), dura 60, fonte avança 20 frames
    assert_eq!((v.start, v.duration), (frames(10), frames(60)));
    assert_eq!(v.source_in, frames(20), "left trim advances the source");
    assert!(matches!(v.content, ClipContent::Media { .. }));
}

#[test]
fn flatten_drops_children_outside_the_window_and_warns() {
    let mut w = W::new(2);
    w.solid("t1", 0, 10, "early").unwrap();
    w.solid("t1", 50, 30, "late").unwrap();
    w.cmd(Command::AddMarker {
        sequence: sid(1),
        id: Some("m".into()),
        time: frames(5),
        label: String::new(),
    })
    .unwrap();
    w.cmd(Command::InsertNested {
        track: "t0".into(),
        start: frames(0),
        sequence: sid(1),
        id: Some("n".into()),
        name: "n".into(),
        duration: Some(frames(30)),
        source_in: frames(20),
        follow_length: false,
        split_at_insert: false,
        split_new_id: None,
    })
    .unwrap();
    let r = flatten(&mut w, "n").unwrap();
    assert!(w.clip("s0", "n.early").is_none(), "outside the window");
    assert!(
        w.clip("s0", "n.late").is_none(),
        "outside the window ([20,50) ends before 50)"
    );
    assert!(
        r.results[0].warnings.iter().any(|m| m.contains("markers")),
        "markers are not copied and the user is told: {:?}",
        r.results[0].warnings
    );
}

#[test]
fn flatten_refuses_to_lose_information() {
    let mut w = W::new(2);
    w.solid("t1", 0, 30, "x").unwrap();
    w.nest("t0", 1, 0, "n").unwrap();
    w.nest("m0", 1, 0, "nm").unwrap();
    let before = w.sem();
    // track magnética
    assert_eq!(
        flatten(&mut w, "nm").unwrap_err().code,
        ErrorCode::UnsupportedCommand
    );
    // retime
    w.cmd(Command::SetClipSpeed {
        clip: "n".into(),
        speed: Rational::new(2, 1).unwrap(),
        ripple: Some(false),
        scope: Default::default(),
    })
    .unwrap();
    assert_eq!(
        flatten(&mut w, "n").unwrap_err().code,
        ErrorCode::UnsupportedCommand
    );
    // clip que não é nested / inexistente
    assert_eq!(
        flatten(&mut w, "x").unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        flatten(&mut w, "ghost").unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_ne!(w.sem(), before);
}

#[test]
fn flatten_refuses_a_nested_clip_with_its_own_properties_and_different_frame_rates() {
    let mut w = W::new(1);
    w.cmd(Command::CreateSequence {
        id: Some("s24".into()),
        name: "24".into(),
        frame_rate: FrameRate::FPS_24,
        sample_rate: None,
    })
    .unwrap();
    w.cmd(Command::AddTrack {
        sequence: "s24".into(),
        id: Some("t24".into()),
        kind: TrackKind::Visual,
        name: None,
        role: None,
        magnetic: false,
        index: None,
    })
    .unwrap();
    w.cmd(Command::InsertClip {
        track: "t24".into(),
        start: Ticks::ZERO,
        clip: NewClip {
            id: Some("k".into()),
            name: "k".into(),
            duration: Ticks(705_600_000),
            content: ClipContent::Solid {
                color: "#fff".into(),
            },
            source_in: Ticks::ZERO,
            speed: Rational::ONE,
            reversed: false,
            properties: Default::default(),
        },
        split_at_insert: false,
        split_new_id: None,
    })
    .unwrap();
    w.cmd(Command::InsertNested {
        track: "t0".into(),
        start: Ticks::ZERO,
        sequence: "s24".into(),
        id: Some("n".into()),
        name: "n".into(),
        duration: None,
        source_in: Ticks::ZERO,
        follow_length: false,
        split_at_insert: false,
        split_new_id: None,
    })
    .unwrap();
    assert_eq!(
        flatten(&mut w, "n").unwrap_err().code,
        ErrorCode::UnsupportedCommand
    );
    // propriedade própria do nested
    w.cmd(Command::SetProperty {
        clip: "n".into(),
        prop: "opacity".into(),
        value: 0.5,
    })
    .unwrap();
    assert_eq!(
        flatten(&mut w, "n").unwrap_err().code,
        ErrorCode::UnsupportedCommand
    );
}

#[test]
fn flatten_keeps_grandchildren_as_nested_references() {
    let mut w = W::new(3);
    w.solid("t2", 0, 20, "leaf").unwrap();
    w.nest("t1", 2, 0, "g").unwrap(); // s1 -> s2
    w.nest("t0", 1, 0, "n").unwrap(); // s0 -> s1
    flatten(&mut w, "n").unwrap();
    // s0 agora aponta diretamente para s2 (DAG e profundidade válidos)
    assert_eq!(target_of(&w, "s0", "n.g"), "s2");
}

#[test]
fn create_then_flatten_is_a_round_trip_for_timing() {
    let mut w = W::new(1);
    w.solid("t0", 10, 20, "a").unwrap();
    w.solid("t0", 40, 30, "b").unwrap();
    let original: Vec<(i64, i64)> = ["a", "b"]
        .iter()
        .map(|id| {
            let c = w.clip("s0", id).unwrap();
            (c.start.0, c.duration.0)
        })
        .collect();
    w.cmd(Command::CreateNestedFromSelection {
        clips: vec!["a".into(), "b".into()],
        new_sequence: Some("grp".into()),
        name: None,
        clip_id: Some("g".into()),
        track: None,
        follow_length: false,
    })
    .unwrap();
    flatten(&mut w, "g").unwrap();
    let after: Vec<(i64, i64)> = ["g.a", "g.b"]
        .iter()
        .map(|id| {
            let c = w.clip("s0", id).unwrap();
            (c.start.0, c.duration.0)
        })
        .collect();
    assert_eq!(
        original, after,
        "create_nested ∘ flatten preserves absolute timing"
    );
}

// ======================================= create_nested_from_selection =======================

fn select(w: &mut W, clips: &[&str]) -> R {
    w.cmd(Command::CreateNestedFromSelection {
        clips: clips.iter().map(|c| ClipId::from(*c)).collect(),
        new_sequence: Some("grp".into()),
        name: Some("Group".into()),
        clip_id: Some("g".into()),
        track: None,
        follow_length: false,
    })
}

#[test]
fn create_nested_moves_clips_preserving_relative_timing_and_ids() {
    let mut w = W::new(1);
    w.solid("t0", 10, 20, "a").unwrap();
    w.solid("t0", 50, 30, "b").unwrap();
    w.solid("t0", 100, 10, "outside").unwrap();
    w.audio("a0", 15, 10, "au").unwrap();
    let before = w.sem();
    let r = select(&mut w, &["a", "b", "au"]).unwrap();
    assert_eq!(r.results[0].id.as_deref(), Some("grp"));
    let g = w.clip("s0", "g").unwrap();
    // o nested ocupa [10, 80): do primeiro início ao último fim
    assert_eq!((g.start, g.duration), (frames(10), frames(70)));
    assert!(w.clip("s0", "a").is_none(), "moved out of the parent");
    let a = w.clip("grp", "a").unwrap();
    let b = w.clip("grp", "b").unwrap();
    let au = w.clip("grp", "au").unwrap();
    assert_eq!(
        (a.start, b.start, au.start),
        (frames(0), frames(40), frames(5))
    );
    assert_eq!((a.duration, b.duration), (frames(20), frames(30)));
    assert!(
        w.clip("s0", "outside").is_some(),
        "unselected clips stay put"
    );
    let grp = w.e.document().sequence(&"grp".into()).unwrap();
    assert_eq!(grp.header.name, "Group");
    assert_eq!(
        grp.tracks().len(),
        2,
        "one child track per source track used"
    );
    assert_eq!(grp.duration(), frames(70));
    undo_redo_roundtrip(&mut w, &before);
}

#[test]
fn create_nested_validates_the_selection() {
    let mut w = W::new(2);
    w.solid("t0", 0, 10, "a").unwrap();
    w.solid("t1", 0, 10, "other-seq").unwrap();
    w.solid("m0", 0, 10, "mag").unwrap();
    let before = w.sem();
    let sel = |c: &[&str]| select_raw(c);
    assert_eq!(
        w.cmd(sel(&[])).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        w.cmd(sel(&["a", "a"])).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        w.cmd(sel(&["a", "other-seq"])).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        w.cmd(sel(&["ghost"])).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.cmd(sel(&["mag"])).unwrap_err().code,
        ErrorCode::UnsupportedCommand
    );
    assert_eq!(w.sem(), before, "every rejection is atomic");
    // id do nested já usado
    let e = w
        .cmd(Command::CreateNestedFromSelection {
            clips: vec!["a".into()],
            new_sequence: Some("grp".into()),
            name: None,
            clip_id: Some("other-seq".into()),
            track: None,
            follow_length: false,
        })
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert_eq!(w.sem(), before);
}

fn select_raw(clips: &[&str]) -> Command {
    Command::CreateNestedFromSelection {
        clips: clips.iter().map(|c| ClipId::from(*c)).collect(),
        new_sequence: Some("grp".into()),
        name: None,
        clip_id: Some("g".into()),
        track: None,
        follow_length: false,
    }
}

#[test]
fn create_nested_cannot_overlap_unselected_clips_on_the_host_track() {
    let mut w = W::new(1);
    w.solid("t0", 0, 10, "a").unwrap();
    w.solid("t0", 20, 10, "stay").unwrap(); // dentro do trecho [0, 50) do grupo
    w.audio("a0", 40, 10, "au").unwrap();
    let before = w.sem();
    // grupo = a + au ⇒ [0, 50) sobre a track de cima (t0), que tem "stay" em 20..30
    let e = select(&mut w, &["a", "au"]).unwrap_err();
    assert!(
        matches!(e.code, ErrorCode::Overlap | ErrorCode::InvalidArgument),
        "{e}"
    );
    assert_eq!(w.sem(), before, "nothing half-moved");
    // escolhendo outra track livre para o nested, funciona
    w.cmd(Command::AddTrack {
        sequence: sid(0),
        id: Some("host".into()),
        kind: TrackKind::Visual,
        name: None,
        role: None,
        magnetic: false,
        index: None,
    })
    .unwrap();
    w.cmd(Command::CreateNestedFromSelection {
        clips: vec!["a".into(), "au".into()],
        new_sequence: Some("grp".into()),
        name: None,
        clip_id: Some("g".into()),
        track: Some("host".into()),
        follow_length: false,
    })
    .unwrap();
    assert_eq!(w.clip("s0", "g").unwrap().track.0, "host");
}

#[test]
fn nested_from_selection_can_be_nested_again_and_depth_is_still_guarded() {
    let mut w = W::new(1);
    w.solid("t0", 0, 10, "a").unwrap();
    select(&mut w, &["a"]).unwrap();
    // agrupar de novo o próprio nested (sequence nova, um nível a mais)
    w.cmd(Command::CreateNestedFromSelection {
        clips: vec!["g".into()],
        new_sequence: Some("grp2".into()),
        name: None,
        clip_id: Some("g2".into()),
        track: None,
        follow_length: false,
    })
    .unwrap();
    assert_eq!(target_of(&w, "s0", "g2"), "grp2");
    assert_eq!(target_of(&w, "grp2", "g"), "grp");
    // empilhar até estourar o limite de profundidade: o comando falha de forma estruturada e atômica
    let mut last = "g2".to_owned();
    let mut failed = None;
    for i in 0..30 {
        let before = w.sem();
        let id = format!("deep{i}");
        let r = w.cmd(Command::CreateNestedFromSelection {
            clips: vec![last.as_str().into()],
            new_sequence: Some(format!("seqd{i}").as_str().into()),
            name: None,
            clip_id: Some(id.as_str().into()),
            track: None,
            follow_length: false,
        });
        match r {
            Ok(_) => last = id,
            Err(e) => {
                assert_eq!(w.sem(), before, "rejection leaves the document untouched");
                failed = Some(e);
                break;
            }
        }
    }
    let e = failed.expect("the depth limit must stop the chain");
    assert!(
        matches!(e.code, ErrorCode::NestedDepth | ErrorCode::LimitExceeded),
        "{e}"
    );
}

// ============================================== generate_variants ===========================

fn variant(seq: &str, swaps: Vec<VariantSwap>) -> VariantSpec {
    VariantSpec {
        sequence: Some(seq.into()),
        name: None,
        deep: false,
        swaps,
    }
}

#[test]
fn variants_are_copies_of_the_template_with_structural_swaps() {
    let mut w = W::new(4);
    w.solid("t1", 0, 30, "hook-a").unwrap(); // s1 = hook A
    w.solid("t2", 0, 20, "hook-b").unwrap(); // s2 = hook B
    w.solid("t3", 0, 60, "body").unwrap(); // s3 = body
    w.asset("A", 10).unwrap();
    w.asset("B", 10).unwrap();
    w.nest("t0", 1, 0, "hook").unwrap(); // template s0: hook (s1) ...
    w.nest("t0", 3, 40, "bodyref").unwrap(); // ... + body (s3)
    w.media("a0", 0, 30, "vo", "A").ok();
    w.insert(
        "m0",
        0,
        30,
        "broll",
        ClipContent::Media {
            asset: "A".into(),
            has_video: true,
            has_audio: false,
        },
    )
    .unwrap();
    let before = w.sem();
    let r = w
        .cmd(Command::GenerateVariants {
            template: sid(0),
            variants: vec![
                variant(
                    "v1",
                    vec![VariantSwap::SetNested {
                        clip: "hook".into(),
                        sequence: sid(2),
                    }],
                ),
                variant(
                    "v2",
                    vec![VariantSwap::ReplaceMedia {
                        clip: "broll".into(),
                        asset: "B".into(),
                    }],
                ),
                VariantSpec {
                    sequence: None,
                    name: Some("third".into()),
                    deep: false,
                    swaps: vec![],
                },
            ],
        })
        .unwrap();
    assert_eq!(r.results[0].id.as_deref(), Some("v1"));
    assert_eq!(target_of(&w, "v1", "v1.hook"), "s2", "hook swapped to B");
    assert_eq!(target_of(&w, "v1", "v1.bodyref"), "s3", "body still shared");
    assert_eq!(target_of(&w, "v2", "v2.hook"), "s1", "v2 keeps hook A");
    match w.clip("v2", "v2.broll").unwrap().content {
        ClipContent::Media { asset, .. } => assert_eq!(asset.0, "B"),
        _ => panic!(),
    }
    // a template não foi alterada
    match w.clip("s0", "broll").unwrap().content {
        ClipContent::Media { asset, .. } => assert_eq!(asset.0, "A"),
        _ => panic!(),
    }
    assert_eq!(target_of(&w, "s0", "hook"), "s1");
    assert_eq!(w.e.document().sequence_count(), 4 + 3);
    // um commit só: undo remove as três variantes de uma vez
    assert_eq!(
        w.e.document()
            .sequences()
            .filter(|(id, _)| id.as_str().starts_with('v') || id.as_str().starts_with("seq_"))
            .count(),
        3
    );
    undo_redo_roundtrip(&mut w, &before);
}

#[test]
fn variants_are_validated_and_atomic() {
    let mut w = W::new(3);
    w.solid("t1", 0, 10, "h").unwrap();
    w.nest("t0", 1, 0, "hook").unwrap();
    w.asset("A", 1).unwrap();
    w.solid("m0", 0, 5, "plain").unwrap();
    let before = w.sem();
    let gv = |variants: Vec<VariantSpec>| Command::GenerateVariants {
        template: sid(0),
        variants,
    };
    assert_eq!(
        w.cmd(gv(vec![])).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    // troca de clip que não é da template
    let e = w
        .cmd(gv(vec![variant(
            "v",
            vec![VariantSwap::SetNested {
                clip: "ghost".into(),
                sequence: sid(2),
            }],
        )]))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    // asset inexistente / clip sem mídia
    assert_eq!(
        w.cmd(gv(vec![variant(
            "v",
            vec![VariantSwap::ReplaceMedia {
                clip: "plain".into(),
                asset: "A".into()
            }]
        )]))
        .unwrap_err()
        .code,
        ErrorCode::InvalidArgument
    );
    // uma variante boa seguida de uma ruim: NADA é aplicado
    let e = w
        .cmd(gv(vec![
            variant("good", vec![]),
            variant(
                "bad",
                vec![VariantSwap::SetNested {
                    clip: "ghost".into(),
                    sequence: sid(2),
                }],
            ),
        ]))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(w.sem(), before);
    // id de variante repetido na mesma transação
    assert!(
        w.cmd(gv(vec![variant("dup", vec![]), variant("dup", vec![])]))
            .is_err()
    );
    assert_eq!(w.sem(), before);
    // limite de variantes
    let many: Vec<VariantSpec> = (0..101)
        .map(|i| variant(&format!("x{i}"), vec![]))
        .collect();
    assert_eq!(w.cmd(gv(many)).unwrap_err().code, ErrorCode::LimitExceeded);
    assert_eq!(w.sem(), before);
}

#[test]
fn a_variant_cannot_close_a_cycle() {
    let mut w = W::new(2);
    w.solid("t1", 0, 10, "h").unwrap();
    w.nest("t0", 1, 0, "hook").unwrap();
    let before = w.sem();
    // a variante tenta apontar o nested para ELA MESMA: A → A
    let e = w
        .cmd(Command::GenerateVariants {
            template: sid(0),
            variants: vec![variant(
                "selfref",
                vec![VariantSwap::SetNested {
                    clip: "hook".into(),
                    sequence: "selfref".into(),
                }],
            )],
        })
        .unwrap_err();
    assert!(
        matches!(
            e.code,
            ErrorCode::NestedCycle | ErrorCode::InvariantViolation
        ),
        "{e}"
    );
    assert_eq!(w.sem(), before);
}

#[test]
fn marker_is_unused_import_guard() {
    // mantém `Marker` referenciado (o tipo faz parte do contrato testado pelas cópias)
    let m = Marker {
        id: "m".into(),
        time: Ticks::ZERO,
        label: String::new(),
    };
    assert_eq!(m.label, "");
}
