//! Metas de desempenho da persistência (ROADMAP Fase 2), projeto com 10.000 clips:
//! abrir < 2 s · recarregar documento < 2 s · transação comum (commit durável, `synchronous=FULL`)
//! < 200 ms. Medidos em `--release`, marcados `#[ignore]` (o debug não representa o produto) e
//! imprimem os números: `cargo test --release -p capia-store --test perf -- --ignored --nocapture`.
//! Em CI servem só de *sanity check*: timing de runner compartilhado não é benchmark científico.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_commands::{Engine, EngineConfig};
use capia_model::{
    Asset, Clip, ClipContent, Document, PrimitiveOp, SequenceHeader, SequenceId, Track, TrackKind,
    TrackSlot,
};
use capia_store::{ProjectStore, StoreOptions};
use capia_time::{FrameRate, Rational, Ticks};
use common::*;
use std::time::Instant;

/// 10 tracks livres × 1.000 clips.
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
fn a_10k_clip_project_opens_reloads_and_commits_within_budget() {
    let dir = TempDir::new("perf");
    let path = dir.file("big.capia");
    let opts = StoreOptions::default(); // FULL: o custo real de um commit durável
    let doc = big_document();

    let t = Instant::now();
    let (store, _) = ProjectStore::create_with_document(&path, &doc, &opts).unwrap();
    let create = t.elapsed();
    store.close().unwrap();
    let size_kib = std::fs::metadata(&path).unwrap().len() / 1024;

    let t = Instant::now();
    let (store, state) = ProjectStore::open(&path, &opts).unwrap();
    let open = t.elapsed();
    assert_eq!(
        state.doc.sequence(&"S".into()).unwrap().clip_count(),
        10_000
    );

    let t = Instant::now();
    let reloaded = store.load_state().unwrap();
    let reload = t.elapsed();
    assert_eq!(reloaded.doc, state.doc);

    let mut engine = store
        .into_engine(state, key(), EngineConfig::default())
        .unwrap();
    // transação comum: um clip novo (commit durável com fsync)
    let mut times = Vec::new();
    for i in 0..30_i64 {
        let t_ = tx(
            "perf",
            vec![insert(
                &format!("p{i}"),
                "T0",
                20_000 + i * 20,
                &format!("new{i}"),
                10,
            )],
        );
        let t = Instant::now();
        engine.execute(&user(), t_, 7).unwrap();
        times.push(t.elapsed());
    }
    times.sort();
    let (median, worst) = (times[times.len() / 2], times[times.len() - 1]);

    // commit que grava um snapshot de 10.000 clips (o mais caro) — força com cadência 1
    drop(engine);
    let snap_opts = StoreOptions {
        snapshot_every: 1,
        ..StoreOptions::default()
    };
    let mut engine =
        ProjectStore::open_engine(&path, &snap_opts, key(), EngineConfig::default()).unwrap();
    let t = Instant::now();
    engine
        .execute(
            &user(),
            tx("snap", vec![insert("snapop", "T1", 20_000, "snapclip", 10)]),
            8,
        )
        .unwrap();
    let snapshot_commit = t.elapsed();
    drop(engine);

    // histórico longo: 300 commits depois do último snapshot, depois reabre (replay do rabo)
    let tail_opts = StoreOptions {
        snapshot_every: 100_000,
        ..StoreOptions::default()
    };
    let mut engine =
        ProjectStore::open_engine(&path, &tail_opts, key(), EngineConfig::default()).unwrap();
    for i in 0..300_i64 {
        engine
            .execute(
                &user(),
                tx(
                    "tail",
                    vec![insert(
                        &format!("t{i}"),
                        "T2",
                        20_000 + i * 20,
                        &format!("tl{i}"),
                        10,
                    )],
                ),
                9,
            )
            .unwrap();
    }
    drop(engine);
    let t = Instant::now();
    let (_, replayed) = ProjectStore::open(&path, &tail_opts).unwrap();
    let open_with_tail = t.elapsed();
    assert_eq!(
        replayed.doc.sequence(&"S".into()).unwrap().clip_count(),
        10_000 + 30 + 1 + 300
    );

    eprintln!("PERF 10.000 clips ({size_kib} KiB on disk, synchronous=FULL)");
    eprintln!("PERF create_with_document: {create:?}");
    eprintln!("PERF open (snapshot + load + quick_check): {open:?}");
    eprintln!("PERF reload document (load_state): {reload:?}");
    eprintln!("PERF common transaction commit: median {median:?}, worst {worst:?} (n=30)");
    eprintln!("PERF commit that writes a 10k-clip snapshot: {snapshot_commit:?}");
    eprintln!("PERF open with a 300-event replay tail: {open_with_tail:?}");
    assert!(open.as_secs_f64() < 2.0, "open took {open:?}");
    assert!(reload.as_secs_f64() < 2.0, "reload took {reload:?}");
    assert!(median.as_millis() < 200, "common commit median {median:?}");
    assert!(
        open_with_tail.as_secs_f64() < 2.0,
        "open with tail took {open_with_tail:?}"
    );
    let _: Option<Engine> = None;
}
