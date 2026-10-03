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

use capia_commands::{
    Actor, ClipMove, Command, CommandEnvelope, Edge, Engine, NewClip, RippleScope, Transaction,
};
use capia_model::{
    Asset, ClipContent, Document, Interp, Sequence, SequenceId, TrackId, TrackKind,
    validate_document,
};
use capia_time::{FrameRate, Rational, Ticks};

const FPS: FrameRate = FrameRate::FPS_30;
const FRAME: i64 = 23_520_000;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len() as u64) as usize])
        }
    }

    fn frames(&mut self, max: u64) -> Ticks {
        let ticks = Ticks(self.below(max) as i64 * FRAME);
        // de vez em quando, desalinhado de propósito (deve ser rejeitado em tracks visuais)
        if self.chance(4) {
            Ticks(ticks.0 + 1)
        } else {
            ticks
        }
    }
}

fn tx(label: &str, n: &mut u32, commands: Vec<Command>) -> Transaction {
    Transaction {
        transaction_id: None,
        label: label.into(),
        base_revision: None,
        commands: commands
            .into_iter()
            .map(|command| {
                *n += 1;
                CommandEnvelope {
                    operation_id: format!("op-{n}"),
                    reference: None,
                    command,
                }
            })
            .collect(),
        max_ops: None,
    }
}

fn seq_id() -> SequenceId {
    SequenceId::from("S")
}

fn base_engine() -> Engine {
    let mut n = 0;
    let mut e = Engine::new(Document::new(), [3; 32]);
    let mut setup = vec![Command::CreateSequence {
        id: Some(seq_id()),
        name: "S".into(),
        frame_rate: FPS,
        sample_rate: None,
    }];
    for (id, kind, magnetic) in [
        ("V3", TrackKind::Visual, false),
        ("V2", TrackKind::Visual, false),
        ("V1", TrackKind::Visual, true),
        ("A1", TrackKind::Audio, false),
        ("A2", TrackKind::Audio, false),
    ] {
        setup.push(Command::AddTrack {
            sequence: seq_id(),
            id: Some(id.into()),
            kind,
            name: None,
            role: None,
            magnetic,
            index: None,
        });
    }
    for (id, secs, v, a) in [
        ("short", Some(2), true, true),
        ("long", Some(60), true, true),
        ("endless", None, true, false),
        ("sfx", Some(10), false, true),
    ] {
        setup.push(Command::RegisterAsset {
            asset: Asset {
                id: id.into(),
                name: id.into(),
                duration: secs.map(|s| Ticks(s * capia_time::TICKS_PER_SECOND)),
                has_video: v,
                has_audio: a,
                offline: false,
            },
        });
    }
    e.execute(&Actor::system(), tx("setup", &mut n, setup), 0)
        .unwrap();
    e
}

fn sequence(e: &Engine) -> &Sequence {
    e.document().sequence(&seq_id()).unwrap()
}

fn random_scope(rng: &mut Rng) -> RippleScope {
    match rng.below(5) {
        0 => RippleScope::Sequence,
        1 => RippleScope::Group,
        2 => RippleScope::Tracks {
            tracks: vec!["V2".into(), "A1".into()],
        },
        _ => RippleScope::Track,
    }
}

/// Instante aleatório em torno de um clip (dentro dele 70% das vezes; borda e fora no resto).
fn near_clip(rng: &mut Rng, seq: &Sequence, id: &capia_model::ClipId) -> Ticks {
    match seq.clip(id) {
        Some(c) if rng.chance(85) => {
            let frames = c.duration.0 / FRAME;
            let t = Ticks(c.start.0 + rng.below(frames as u64 + 1) as i64 * FRAME);
            if rng.chance(4) { Ticks(t.0 + 1) } else { t }
        }
        _ => rng.frames(260),
    }
}

fn random_command(rng: &mut Rng, e: &Engine, counter: &mut u32) -> Command {
    let seq = sequence(e);
    let clips: Vec<_> = seq.clips().map(|c| c.id.clone()).collect();
    let tracks: Vec<TrackId> = seq.tracks().iter().map(|t| t.id.clone()).collect();
    let clip = |rng: &mut Rng| rng.pick(&clips).cloned().unwrap_or_else(|| "ghost".into());
    let kind = rng.below(100);
    match kind {
        0..=27 => {
            *counter += 1;
            let track = rng.pick(&tracks).cloned().unwrap();
            let audio = track.as_str().starts_with('A');
            let (asset, has_video, has_audio) = if audio {
                ("sfx", false, true)
            } else {
                (
                    *rng.pick(&["short", "long", "endless"]).unwrap(),
                    true,
                    false,
                )
            };
            let content = match rng.below(8) {
                0 if !audio => ClipContent::Text { text: "hi".into() },
                1 if !audio => ClipContent::Solid {
                    color: "#fff".into(),
                },
                _ => ClipContent::Media {
                    asset: asset.into(),
                    has_video,
                    has_audio,
                },
            };
            Command::InsertClip {
                track,
                start: rng.frames(200),
                clip: NewClip {
                    id: Some(format!("c{counter}").into()),
                    name: String::new(),
                    duration: Ticks((1 + rng.below(80) as i64) * FRAME),
                    content,
                    source_in: Ticks(rng.below(5) as i64 * FRAME),
                    speed: if rng.chance(30) {
                        Rational::new(1 + rng.below(4) as i64, 1 + rng.below(3) as i64).unwrap()
                    } else {
                        Rational::ONE
                    },
                    reversed: false,
                    properties: Default::default(),
                },
                split_at_insert: rng.chance(50),
                split_new_id: Some(format!("s{counter}").into()),
            }
        }
        28..=39 => Command::DeleteClip {
            clip: clip(rng),
            ripple: if rng.chance(50) { Some(true) } else { None },
            scope: random_scope(rng),
        },
        40..=54 => {
            let c = clip(rng);
            let to = near_clip(rng, seq, &c);
            Command::TrimClip {
                clip: c,
                edge: if rng.chance(50) { Edge::In } else { Edge::Out },
                to,
                ripple: if rng.chance(30) { Some(true) } else { None },
                scope: random_scope(rng),
            }
        }
        55..=64 => {
            *counter += 1;
            let c = clip(rng);
            let at = near_clip(rng, seq, &c);
            Command::SplitClip {
                clip: c,
                at,
                new_id: Some(format!("p{counter}").into()),
            }
        }
        65..=71 => Command::SetClipSpeed {
            clip: clip(rng),
            speed: Rational::new(1 + rng.below(250) as i64, 1 + rng.below(60) as i64).unwrap(),
            ripple: None,
            scope: random_scope(rng),
        },
        72..=77 => {
            let moves = (0..1 + rng.below(3))
                .map(|_| ClipMove {
                    clip: clip(rng),
                    track: if rng.chance(60) {
                        rng.pick(&tracks).cloned()
                    } else {
                        None
                    },
                    start: rng.frames(260),
                })
                .collect();
            Command::MoveClips { moves }
        }
        78..=87 => {
            let c = clip(rng);
            let at = seq.clip(&c).map_or(Ticks::ZERO, |cl| {
                Ticks(cl.start.0 + rng.below((cl.duration.0 / FRAME + 1) as u64) as i64 * FRAME)
            });
            Command::AddKeyframe {
                clip: c,
                prop: (*rng.pick(&["opacity", "scale", "position_x"]).unwrap()).into(),
                at,
                value: rng.below(120) as f64 / 100.0,
                interp: Some(match rng.below(3) {
                    0 => Interp::Hold,
                    1 => Interp::Bezier {
                        x1: 0.25,
                        y1: 1.4,
                        x2: 0.75,
                        y2: -0.3,
                    },
                    _ => Interp::Linear,
                }),
            }
        }
        88..=91 => {
            let c = clip(rng);
            let to = near_clip(rng, seq, &c);
            // `from`: um keyframe existente (velocidade 1: tempo de conteúdo → timeline)
            let from = seq
                .clip(&c)
                .and_then(|cl| {
                    let kfs = cl.properties.get("opacity")?.keyframes();
                    let k = rng.pick(kfs)?;
                    Some(Ticks(cl.start.0 + k.time.0 - cl.source_in.0))
                })
                .unwrap_or_else(|| rng.frames(260));
            Command::MoveKeyframe {
                clip: c,
                prop: "opacity".into(),
                from,
                to,
            }
        }
        92..=93 => Command::DeleteKeyframe {
            clip: clip(rng),
            prop: "opacity".into(),
            at: rng.frames(260),
        },
        94..=95 => Command::SetProperty {
            clip: clip(rng),
            prop: "opacity".into(),
            value: rng.below(130) as f64 / 100.0,
        },
        96..=97 => Command::SetTrackFlags {
            track: rng.pick(&tracks).cloned().unwrap(),
            locked: if rng.chance(50) {
                Some(rng.chance(50))
            } else {
                None
            },
            hidden: None,
            muted: None,
            solo: None,
            magnetic: None,
            sync_lock: if rng.chance(50) {
                Some(rng.chance(50))
            } else {
                None
            },
            group: if rng.chance(30) {
                Some("G".into())
            } else {
                None
            },
            clear_group: false,
            compact: false,
        },
        _ => {
            *counter += 1;
            Command::AddMarker {
                sequence: seq_id(),
                id: Some(format!("m{counter}").into()),
                time: rng.frames(300),
                label: String::new(),
            }
        }
    }
}

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
