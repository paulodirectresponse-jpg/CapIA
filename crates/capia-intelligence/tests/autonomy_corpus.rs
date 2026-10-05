//! Corpus de autonomia (PHASE5_AUTONOMY_TESTS §3): casos de ponta a ponta com Replay. Os casos que
//! já têm teste dedicado estão mapeados em `tests/acceptance/autonomy/corpus.json`; aqui ficam os
//! que dependem de briefing/referência (2, 9, 10) e o guarda que verifica o mapeamento inteiro.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::machine::RunStatus;
use common::auto::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

fn demand_with(must_include: &[&str], must_avoid: &[&str]) -> Value {
    let mut d = demand_json(false);
    let item = |t: &&str| json!({"text": t, "basis": "explicit", "sources": [{"doc": "D1", "unit": "u1", "quote": "Produto: Cafe Serra Azul."}]});
    d["must_include"] = json!(must_include.iter().map(item).collect::<Vec<_>>());
    d["must_avoid"] = json!(must_avoid.iter().map(item).collect::<Vec<_>>());
    d
}

fn overlay_texts(a: &AutoWorld, seq: &str) -> Vec<String> {
    let s =
        a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": seq}))
            .unwrap();
    s["clips"]
        .as_object()
        .unwrap()
        .values()
        .filter_map(|c| {
            c["content"]["text"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| c["content"]["text"]["text"].as_str().map(str::to_owned))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn case_10_strict_must_include_and_must_avoid_are_enforced_by_the_critic_loop() {
    let Some(a) = auto_world("corpus-strict", |asset| {
        Script::new(
            || demand_with(&["frete gratis"], &["concorrente"]),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            move |_d, n| {
                let asset = asset.lock().unwrap().clone();
                let mut v = edit_json(&asset, json!([]));
                let text = if n == 0 {
                    "Melhor que o concorrente"
                } else {
                    "Frete gratis hoje"
                };
                v["beats"][0]["overlays"] =
                    json!([{"text": text, "start_offset_ms": 0, "duration_ms": 1500}]);
                v
            },
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert!(
        matches!(done.status, RunStatus::Completed | RunStatus::WaitingUser),
        "{:?} {:?}",
        done.status,
        done.error
    );
    let seq = &done
        .sequences
        .iter()
        .find(|s| s.role != "superseded")
        .unwrap()
        .sequence_id;
    let texts = overlay_texts(&a, seq).join(" | ").to_lowercase();
    assert!(
        !texts.contains("concorrente"),
        "forbidden word must not survive: {texts}"
    );
    assert!(
        texts.contains("frete gratis"),
        "required phrase must be covered: {texts}"
    );
    assert!(
        a.script.count("planner:main") >= 2,
        "the planner was asked again (replan)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cases_2_and_9_a_reference_informs_the_plan_and_the_brief_wins_on_conflict() {
    let Some(a) = auto_world("corpus-ref", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            move |_d, _n| edit_json(&asset.lock().unwrap(), json!([])),
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let mut inputs = a.inputs();
    inputs.references = vec![a.w.asset_id.clone()]; // referência com ritmo próprio; o brief limita a 30 s
    let run = a.create(inputs, auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    // a gramática da referência chegou ao Producer/Planner como dado (não como instrução)
    let prompts = a.script.prompts.lock().unwrap().clone();
    assert!(
        prompts.iter().any(|(_, u)| u.contains("reference grammar")),
        "the reference grammar is part of the planning context"
    );
    assert!(
        prompts.iter().any(|(_, u)| u.contains("## transcript")),
        "the raw footage transcript reaches the planning context"
    );
    // o limite do brief vale sobre a referência
    let seq = &done.sequences[0].sequence_id;
    let s =
        a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": seq}))
            .unwrap();
    let end = s["clips"]
        .as_object()
        .unwrap()
        .values()
        .map(|c| c["start"].as_i64().unwrap() + c["duration"].as_i64().unwrap())
        .max()
        .unwrap();
    assert!(end <= 30 * 705_600_000, "brief max duration wins: {end}");
    let _ = a.spy.applies.load(Ordering::SeqCst);
}

/// O manifesto do corpus só pode apontar para testes que existem (nada de caso "fantasma").
#[test]
fn the_corpus_manifest_maps_all_20_cases_to_existing_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("tests/acceptance/autonomy/corpus.json")).unwrap(),
    )
    .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 20);
    let ids: Vec<u64> = cases.iter().map(|c| c["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, (1..=20).collect::<Vec<u64>>());
    for c in cases {
        let file = c["file"].as_str().unwrap();
        let test = c["test"].as_str().unwrap();
        let src =
            std::fs::read_to_string(root.join(file)).unwrap_or_else(|_| panic!("missing {file}"));
        assert!(
            src.contains(&format!("fn {test}(")),
            "case {}: `{test}` not found in {file}",
            c["id"]
        );
    }
}
