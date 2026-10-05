//! Orçamento por Run (PHASE5_AUTONOMY_TESTS §custo): limite exato, preço desconhecido, reservas
//! paralelas e "nunca gasta antes de aprovar".
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::generation::{GenKind, ReplayGenerationProvider};
use capia_intelligence::autonomy::machine::RunStatus;
use capia_intelligence::autonomy::model::{DecisionKind, RunBudget, RunPolicy};
use common::auto::*;
use common::*;
use serde_json::{Value, json};
use std::sync::Arc;

fn need(id: &str, required: bool, priority: &[&str]) -> Value {
    json!({"id": id, "kind": "video", "purpose": "b-roll produto", "description": "pessoa usando cafe na cozinha",
           "required": required, "source_priority": priority, "target_duration_ms": 4000})
}

fn beats_with_need(asset: &str, need_id: &str) -> Value {
    json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "estimated_duration_ms": 7000, "beats": [
        {"id": "hook", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": asset, "source_in_ms": 0},
         "overlays": [{"text": "Compre agora", "start_offset_ms": 0, "duration_ms": 1500}]},
        {"id": "broll", "role": "broll", "duration_ms": 3000, "asset": {"need_id": need_id, "source_in_ms": 0}},
        {"id": "cta", "role": "cta", "duration_ms": 1000, "asset": {"asset_id": asset, "source_in_ms": 7000}}]})
}

fn scripted(name: &str, src: &str) -> Option<AutoWorld> {
    auto_world(name, |asset| {
        let src = src.to_owned();
        Script::new(
            || demand_json(false),
            move || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([need("n1", true, &[src.as_str()])]),
                )
            },
            move |_d, _n| beats_with_need(&asset.lock().unwrap(), "n1"),
            |_n| json!({"findings": []}),
        )
    })
}

fn gen_world(name: &str, price: Option<u64>) -> Option<(AutoWorld, Arc<ReplayGenerationProvider>)> {
    let tc = ffmpeg()?;
    let a = scripted(name, "generate")?;
    let payload = broll_bytes(&tc, &a.w.dir, "gen.mp4", 4);
    let prov = Arc::new(ReplayGenerationProvider::new(
        "gen",
        vec![GenKind::Video],
        price,
        payload,
    ));
    a.generators.register(prov.clone());
    Some((a, prov))
}

fn policy() -> RunPolicy {
    RunPolicy {
        generation_requires_approval: false,
        ..auto_policy()
    }
}

fn run_with(a: &AutoWorld, max: Option<u64>) -> String {
    let budget = RunBudget {
        max_cost_micros: max,
        ..RunBudget::default()
    };
    a.orch
        .create_run(a.inputs(), Some(policy()), Some(budget), "pf", None)
        .unwrap()
        .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_limit_equal_to_the_estimate_runs_and_one_micro_less_asks_first() {
    let Some((a, prov)) = gen_world("bud-exact", Some(500_000)) else {
        return;
    };
    let id = run_with(&a, Some(500_000));
    let done = a.run_to_rest(&id).await;
    assert_eq!(
        done.status,
        RunStatus::Completed,
        "{:?} {:?}",
        done.error,
        done.pending
    );
    assert_eq!(prov.submit_count(), 1);
    assert!(done.usage.cost_micros <= 500_000);

    let Some((b, prov2)) = gen_world("bud-under", Some(500_000)) else {
        return;
    };
    let id = run_with(&b, Some(499_999));
    let w = b.run_to_rest(&id).await;
    assert_eq!(
        w.pending.as_ref().unwrap().kind,
        DecisionKind::BudgetExtension,
        "{:?}",
        w.pending
    );
    assert_eq!(
        prov2.submit_count(),
        0,
        "nothing is spent before the user extends the budget"
    );
    // recusar a extensão não gasta nada
    let d = w.pending.unwrap();
    let _ = b.orch.decide(&id, &d.id, "stop", &Value::Null, "user");
    assert_eq!(prov2.submit_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_price_is_never_treated_as_zero() {
    let Some((a, prov)) = gen_world("bud-unknown", None) else {
        return;
    };
    // com teto de custo definido, preço desconhecido não pode furar o teto em silêncio
    let id = run_with(&a, Some(1_000));
    let w = a.run_to_rest(&id).await;
    let asked = w.status == RunStatus::WaitingUser;
    if !asked {
        // se seguiu, o gasto tem de constar como desconhecido, nunca 0 "conhecido"
        assert!(
            w.usage.unknown_cost_calls > 0 || prov.submit_count() == 0,
            "{:?}",
            w.usage
        );
    } else {
        assert_eq!(prov.submit_count(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_generation_cap_is_enforced_before_submitting() {
    let Some((a, prov)) = gen_world("bud-gens", Some(1)) else {
        return;
    };
    let budget = RunBudget {
        max_generations: Some(0),
        ..RunBudget::default()
    };
    let id = a
        .orch
        .create_run(a.inputs(), Some(policy()), Some(budget), "pf", None)
        .unwrap()
        .id;
    let w = a.run_to_rest(&id).await;
    assert_ne!(w.status, RunStatus::Completed);
    assert_eq!(prov.submit_count(), 0);
}
