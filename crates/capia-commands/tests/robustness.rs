//! Robustez: valores extremos vindos de UI/API (Ticks/velocidades/valores nos limites de i64/f64,
//! NaN, ±∞) nunca derrubam o engine (sem pânico, sem overflow silencioso) e nunca deixam o
//! documento inválido — só `Err` estruturado ou comando aceito e válido.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::{
    Actor, ClipMove, Command, CommandEnvelope, Edge, NewClip, RippleScope, Transaction,
};
use capia_model::{ClipContent, Interp, validate_document};
use capia_time::{Rational, Ticks};
use common::{FRAME, base_engine, tx};

const TICKS: [i64; 9] = [
    i64::MIN,
    i64::MIN + 1,
    -FRAME,
    -1,
    0,
    1,
    FRAME,
    i64::MAX - 1,
    i64::MAX,
];
const FLOATS: [f64; 8] = [
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    -1e308,
    -1.0,
    0.0,
    1.0,
    1e308,
];

fn seeded() -> capia_commands::Engine {
    let mut e = base_engine();
    let mut n = 50_000;
    let seed = vec![
        Command::InsertClip {
            track: "V2".into(),
            start: Ticks(10 * FRAME),
            clip: NewClip {
                id: Some("c1".into()),
                name: String::new(),
                duration: Ticks(60 * FRAME),
                content: ClipContent::Media {
                    asset: "endless".into(),
                    has_video: true,
                    has_audio: false,
                },
                source_in: Ticks::ZERO,
                speed: Rational::ONE,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: false,
            split_new_id: None,
        },
        Command::AddKeyframe {
            clip: "c1".into(),
            prop: "opacity".into(),
            at: Ticks(20 * FRAME),
            value: 0.5,
            interp: None,
        },
        Command::InsertClip {
            track: "V1".into(),
            start: Ticks::ZERO,
            clip: NewClip {
                id: Some("m1".into()),
                name: String::new(),
                duration: Ticks(30 * FRAME),
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
    ];
    e.execute(&Actor::user("seed"), tx("seed", &mut n, seed), 0)
        .unwrap();
    e
}

fn commands_for(t: i64, speed: Rational, f: f64) -> Vec<Command> {
    let ticks = Ticks(t);
    vec![
        Command::InsertClip {
            track: "V3".into(),
            start: ticks,
            clip: NewClip {
                id: None,
                name: String::new(),
                duration: ticks,
                content: ClipContent::Media {
                    asset: "short".into(),
                    has_video: true,
                    has_audio: false,
                },
                source_in: ticks,
                speed,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: true,
            split_new_id: None,
        },
        Command::InsertClip {
            track: "V1".into(),
            start: ticks,
            clip: NewClip {
                id: None,
                name: String::new(),
                duration: Ticks(FRAME),
                content: ClipContent::Solid {
                    color: "#000".into(),
                },
                source_in: Ticks::ZERO,
                speed: Rational::ONE,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: true,
            split_new_id: None,
        },
        Command::TrimClip {
            clip: "c1".into(),
            edge: Edge::In,
            to: ticks,
            ripple: Some(true),
            scope: RippleScope::Sequence,
        },
        Command::TrimClip {
            clip: "m1".into(),
            edge: Edge::Out,
            to: ticks,
            ripple: None,
            scope: RippleScope::Sequence,
        },
        Command::SplitClip {
            clip: "c1".into(),
            at: ticks,
            new_id: None,
        },
        Command::SetClipSpeed {
            clip: "c1".into(),
            speed,
            ripple: Some(true),
            scope: RippleScope::Sequence,
        },
        Command::MoveClips {
            moves: vec![ClipMove {
                clip: "c1".into(),
                track: Some("V3".into()),
                start: ticks,
            }],
        },
        Command::AddKeyframe {
            clip: "c1".into(),
            prop: "opacity".into(),
            at: ticks,
            value: f,
            interp: Some(Interp::Linear),
        },
        Command::AddKeyframe {
            clip: "c1".into(),
            prop: "scale".into(),
            at: Ticks(20 * FRAME),
            value: 1.0,
            interp: Some(Interp::Bezier {
                x1: f,
                y1: f,
                x2: f,
                y2: f,
            }),
        },
        Command::MoveKeyframe {
            clip: "c1".into(),
            prop: "opacity".into(),
            from: ticks,
            to: Ticks(t.wrapping_add(FRAME)),
        },
        Command::SetProperty {
            clip: "c1".into(),
            prop: "scale".into(),
            value: f,
        },
        Command::AddMarker {
            sequence: "S".into(),
            id: None,
            time: ticks,
            label: String::new(),
        },
        Command::DeleteClip {
            clip: "c1".into(),
            ripple: Some(true),
            scope: RippleScope::Sequence,
        },
    ]
}

#[test]
fn extreme_values_are_rejected_or_applied_validly_never_panic() {
    let speeds = [
        Rational::new(i64::MAX, 1).unwrap(),
        Rational::new(1, i64::MAX).unwrap(),
        Rational::new(i64::MIN + 1, 1).unwrap(),
        Rational::new(i64::MAX, i64::MAX - 1).unwrap(),
        Rational::ZERO,
        Rational::ONE,
        Rational::new(1, 100).unwrap(),
        Rational::new(100, 1).unwrap(),
    ];
    let mut checked = 0usize;
    for &t in &TICKS {
        for (i, &speed) in speeds.iter().enumerate() {
            let f = FLOATS[(i + (t & 7) as usize) % FLOATS.len()];
            for (k, command) in commands_for(t, speed, f).into_iter().enumerate() {
                let mut e = seeded();
                let (before, rev) = (e.document().clone(), e.revision());
                let env = CommandEnvelope {
                    operation_id: format!("x-{t}-{i}-{k}"),
                    reference: None,
                    command,
                };
                let t = Transaction {
                    transaction_id: None,
                    label: "x".into(),
                    base_revision: None,
                    commands: vec![env],
                    max_ops: None,
                };
                match e.execute(&Actor::user("fuzz"), t, 0) {
                    Ok(_) => {
                        let v = validate_document(e.document());
                        assert!(
                            v.is_empty(),
                            "accepted an extreme command and broke invariants: {v:?}"
                        );
                        // e continua desfazível
                        e.undo(&Actor::user("fuzz"), 0).unwrap();
                    }
                    Err(_) => assert!(e.document() == &before && e.revision() == rev),
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 800, "{checked}");
}

#[test]
fn malformed_json_commands_are_rejected_by_deserialization_not_by_panics() {
    let bad = [
        r#"{"operation_id":"a","type":"insert_clip"}"#,
        r#"{"operation_id":"a","type":"nope"}"#,
        r#"{"operation_id":"a","type":"split_clip","clip":"c","at":1.5}"#,
        r#"{"operation_id":"a","type":"set_clip_speed","clip":"c","speed":"1/0"}"#,
        r#"{"operation_id":"a","type":"trim_clip","clip":"c","edge":"middle","to":1}"#,
        r#"{"operation_id":"a","type":"split_clip","clip":"c","at":9223372036854775808}"#,
        r#"[]"#,
        r#"null"#,
    ];
    for json in bad {
        assert!(
            serde_json::from_str::<CommandEnvelope>(json).is_err(),
            "{json}"
        );
    }
    let good = r#"{"operation_id":"a","ref":"$x","type":"split_clip","clip":"c","at":12}"#;
    let env: CommandEnvelope = serde_json::from_str(good).unwrap();
    assert_eq!(env.reference.as_deref(), Some("$x"));
    assert_eq!(
        serde_json::from_str::<CommandEnvelope>(&serde_json::to_string(&env).unwrap()).unwrap(),
        env
    );
}
