//! Metas de desempenho do núcleo (docs/TIMELINE_ENGINE.md §7, ROADMAP Fase 2):
//! * transação de 500 operações em projeto de 10.000 clips < 200 ms;
//! * carregar (desserializar + indexar + validar) projeto de 10.000 clips < 2 s.
//!
//! Medidos em `--release`; marcados `#[ignore]` porque o build de debug não representa o produto:
//! `cargo test --release -p capia-commands --test perf -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Command, CommandEnvelope, Edge, Engine, RippleScope, Transaction};
use capia_model::{
    Asset, Clip, ClipContent, Document, PrimitiveOp, SequenceHeader, SequenceId, Track, TrackKind,
    TrackSlot, validate_document,
};
use capia_time::{FrameRate, Rational, Ticks};
use std::time::Instant;

const FRAME: i64 = 23_520_000;

/// 10 tracks livres × 1.000 clips de 10 frames com 2 frames de espaço.
fn big_document() -> Document {
    let mut doc = Document::new();
    let sid = SequenceId::from("S");
    doc.apply_op(&PrimitiveOp::Sequence {
        id: sid.clone(),
        old: None,
        new: Some(SequenceHeader {
            name: "S".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: 48_000,
            width: 1920,
            height: 1080,
            folder: None,
        }),
    })
    .unwrap();
    doc.apply_op(&PrimitiveOp::Asset {
        id: "m".into(),
        old: None,
        new: Some(Asset {
            id: "m".into(),
            name: "m".into(),
            duration: None,
            has_video: true,
            has_audio: false,
            offline: false,
        }),
    })
    .unwrap();
    for t in 0..10 {
        let tid = format!("T{t}");
        doc.apply_op(&PrimitiveOp::Track {
            sequence: sid.clone(),
            id: tid.as_str().into(),
            old: None,
            new: Some(TrackSlot {
                index: t,
                track: Track::new(tid.as_str(), TrackKind::Visual),
            }),
        })
        .unwrap();
        for i in 0..1_000_i64 {
            let id = format!("c{t}_{i}");
            doc.apply_op(&PrimitiveOp::Clip {
                sequence: sid.clone(),
                id: id.as_str().into(),
                old: None,
                new: Some(Clip {
                    id: id.as_str().into(),
                    track: tid.as_str().into(),
                    start: Ticks(i * 12 * FRAME),
                    duration: Ticks(10 * FRAME),
                    name: String::new(),
                    enabled: true,
                    content: ClipContent::Media {
                        asset: "m".into(),
                        has_video: true,
                        has_audio: false,
                    },
                    source_in: Ticks::ZERO,
                    speed: Rational::ONE,
                    reversed: false,
                    properties: Default::default(),
                    group: None,
                    transition_in: None,
                }),
            })
            .unwrap();
        }
    }
    doc
}

#[test]
#[ignore = "perf: run with --release"]
fn a_500_op_transaction_on_10k_clips_commits_under_200_ms() {
    let mut e = Engine::new(big_document(), [5; 32]);
    // 500 comandos de tipos variados, em tracks diferentes
    let mut commands = Vec::new();
    for i in 0..500_i64 {
        let t = i % 10;
        let k = (i / 10) * 7 % 900;
        let clip = format!("c{t}_{k}").into();
        let command = match i % 4 {
            0 => Command::SplitClip {
                clip,
                at: Ticks((k * 12 + 5) * FRAME),
                new_id: Some(format!("s{i}").into()),
            },
            1 => Command::TrimClip {
                clip,
                edge: Edge::Out,
                to: Ticks((k * 12 + 8) * FRAME),
                ripple: None,
                scope: RippleScope::Track,
            },
            2 => Command::SetProperty {
                clip,
                prop: "opacity".into(),
                value: 0.5,
            },
            _ => Command::AddKeyframe {
                clip,
                prop: "scale".into(),
                at: Ticks((k * 12 + 1) * FRAME),
                value: 2.0,
                interp: None,
            },
        };
        commands.push(CommandEnvelope {
            operation_id: format!("p{i}"),
            reference: None,
            command,
        });
    }
    let tx = Transaction {
        transaction_id: None,
        label: "perf".into(),
        base_revision: None,
        commands,
        max_ops: None,
    };
    let start = Instant::now();
    e.execute(&Actor::user("perf"), tx, 0).unwrap();
    let elapsed = start.elapsed();
    eprintln!("500-op transaction on 10.000 clips: {elapsed:?}");
    assert!(elapsed.as_millis() < 200, "took {elapsed:?}");
}

#[test]
#[ignore = "perf: run with --release"]
fn loading_a_10k_clip_document_takes_under_2_s() {
    let json = serde_json::to_string(&big_document()).unwrap();
    let start = Instant::now();
    let doc: Document = serde_json::from_str(&json).unwrap();
    assert!(validate_document(&doc).is_empty());
    let elapsed = start.elapsed();
    eprintln!(
        "load+index+validate 10.000 clips ({} KiB): {elapsed:?}",
        json.len() / 1024
    );
    assert!(elapsed.as_secs_f64() < 2.0, "took {elapsed:?}");
}
