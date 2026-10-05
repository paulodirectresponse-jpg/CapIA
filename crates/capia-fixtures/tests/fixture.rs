//! A fixture precisa ser REAL (arquivo válido, aberto pelo produto), determinística por semente e
//! cumprir os mínimos da Fase 6. O teste de escala completa é `--ignored` (release).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_fixtures::query::{clips_in_range, clips_in_range_naive};
use capia_fixtures::{FRAME, LargeSpec, build_large_project};
use capia_model::ClipContent;
use capia_project::Project;
use capia_store::{AutonomyStore, StoreOptions};
use capia_time::Ticks;
use std::path::PathBuf;
use std::time::Duration;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("capia-fx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_small_fixture_is_a_valid_real_project_with_every_ingredient() {
    let d = Dir::new("small");
    let path = d.0.join("p.capia");
    let spec = LargeSpec::small();
    let st = build_large_project(&path, &spec).unwrap();
    assert_eq!(st.sequences, spec.sequences);
    assert_eq!(st.clips, spec.clips, "exactly the requested clip count");
    assert!(st.nested_clips > 0 && st.text_clips > 0 && st.caption_clips > 0);
    assert!(st.audio_clips > 0 && st.media_clips > 0 && st.keyframed_clips > 0);
    assert_eq!(st.document_assets, spec.assets);
    assert_eq!(st.catalog_assets, spec.assets as u64);
    assert!(st.history_entries as usize >= spec.history_tail);
    assert_eq!(st.biggest_sequence, "seq_000", "main is the biggest");
    assert_eq!(
        st.run_stages,
        7 * 8 + 3 * 5,
        "completed runs have 8 stages, others 5"
    );
    assert!(st.run_events >= st.run_stages * 3);

    // o arquivo é válido para o produto (validação completa não destrutiva) e reabre igual
    let report = Project::validate(&path);
    assert!(report.ok, "{report:?}");
    let p = Project::open(&path, &StoreOptions::default()).unwrap();
    assert_eq!(
        capia_commands::document_digest(p.document()),
        st.document_digest
    );
    // nested de verdade, com profundidade ≥ 2 e DAG válido (o engine valida ciclos/profundidade)
    let main = p.document().sequence(&"seq_000".into()).unwrap();
    assert!(main.nested_refs().count() >= 3);
    let nested_in_children: usize = p
        .document()
        .sequences()
        .filter(|(id, _)| id.as_str() != "seq_000")
        .map(|(_, s)| s.nested_refs().count())
        .sum();
    assert!(nested_in_children > 0, "children compose grandchildren");
    assert!(
        main.clips()
            .any(|c| matches!(c.content, ClipContent::Text { .. }))
    );
    // runs persistidas e legíveis pela API pública do store
    let runs = AutonomyStore::open(&path, Duration::from_secs(2)).unwrap();
    let list = runs.list_runs(None, 100).unwrap();
    assert_eq!(list.len(), spec.runs);
    assert!(list.iter().any(|r| r.status == "completed"));
    assert!(list.iter().any(|r| r.status == "failed"));
}

#[test]
fn the_same_spec_gives_the_same_document_and_another_seed_a_different_one() {
    let d = Dir::new("det");
    let spec = LargeSpec::small();
    let a = build_large_project(&d.0.join("a.capia"), &spec).unwrap();
    let b = build_large_project(&d.0.join("b.capia"), &spec).unwrap();
    assert_eq!(a.document_digest, b.document_digest);
    assert_eq!(
        (a.clips, a.nested_clips, a.keyframed_clips, a.run_events),
        (b.clips, b.nested_clips, b.keyframed_clips, b.run_events)
    );
    let other = LargeSpec {
        seed: spec.seed + 1,
        ..spec
    };
    let c = build_large_project(&d.0.join("c.capia"), &other).unwrap();
    assert_ne!(a.document_digest, c.document_digest);
}

#[test]
fn bad_specs_are_rejected_before_touching_the_disk() {
    let d = Dir::new("bad");
    let path = d.0.join("x.capia");
    for spec in [
        LargeSpec {
            sequences: 1,
            ..LargeSpec::small()
        },
        LargeSpec {
            assets: 3,
            ..LargeSpec::small()
        },
        LargeSpec {
            clips: 10,
            ..LargeSpec::small()
        },
    ] {
        assert!(build_large_project(&path, &spec).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn range_query_matches_the_naive_oracle_on_the_fixture() {
    let d = Dir::new("range");
    let path = d.0.join("p.capia");
    build_large_project(&path, &LargeSpec::small()).unwrap();
    let p = Project::open(&path, &StoreOptions::default()).unwrap();
    let main = p.document().sequence(&"seq_000".into()).unwrap();
    let total = main.duration().0;
    assert!(total > 0);
    for k in 0..40i64 {
        let start = Ticks((total / 41) * k);
        let end = Ticks(start.0 + (k % 7 + 1) * 90 * FRAME);
        let fast: Vec<_> = clips_in_range(main, start, end)
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let mut slow: Vec<_> = clips_in_range_naive(main, start, end)
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let mut fast_sorted = fast.clone();
        fast_sorted.sort();
        slow.sort();
        assert_eq!(fast_sorted, slow, "window {k}");
        assert!(!fast.is_empty() || k == 40);
    }
}

#[test]
#[ignore = "escala completa: cargo test --release -p capia-fixtures --test fixture -- --ignored --nocapture"]
fn the_default_fixture_meets_the_phase6_minimums() {
    let d = Dir::new("full");
    let path = d.0.join("p.capia");
    let spec = LargeSpec::default();
    let st = build_large_project(&path, &spec).unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&st).unwrap());
    assert!(st.sequences >= 30);
    assert!(st.clips >= 5_000);
    assert!(st.nested_clips > 0 && st.caption_clips > 0 && st.audio_clips > 0);
    assert!(st.catalog_assets >= 1_000);
    assert!(st.runs >= 50);
    assert!(
        st.build_ms < 60_000,
        "build must stay CI-friendly: {} ms",
        st.build_ms
    );
    assert!(Project::validate(&path).ok);
}
