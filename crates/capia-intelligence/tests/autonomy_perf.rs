//! Soak e medidas da autonomia: 12 deliverables numa Run (master + variantes) e a latência do
//! overhead do orquestrador com Replay (sem modelo real: mede o NOSSO custo, não o do LLM).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::machine::RunStatus;
use common::auto::*;
use serde_json::json;

fn many(name: &str, n: usize) -> Option<AutoWorld> {
    auto_world(name, move |asset| {
        Script::new(
            || demand_json(false),
            move || {
                let mut d = vec![json!({"key": "main", "sequence_strategy": "standalone"})];
                for i in 1..n {
                    d.push(json!({"key": format!("v{i}"), "sequence_strategy": "hook_plus_master", "master": "main", "hooks": [format!("h{i}")]}));
                }
                producer_json(json!(d), json!([]))
            },
            move |_d, _n| edit_json(&asset.lock().unwrap(), json!([])),
            |_n| json!({"findings": []}),
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn soak_twelve_deliverables_in_one_run_finish_with_distinct_editable_sequences() {
    let Some(a) = many("soak12", 12) else { return };
    let run = a.create(a.inputs(), auto_policy());
    let t0 = std::time::Instant::now();
    let done = a.run_to_rest(&run.id).await;
    let secs = t0.elapsed().as_secs_f64();
    assert_eq!(
        done.status,
        RunStatus::Completed,
        "{:?} {:?}",
        done.error,
        done.pending
    );
    let mut ids: Vec<_> = done
        .sequences
        .iter()
        .map(|s| s.sequence_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 12);
    // um passo de histórico por unidade, sem duplicatas de operation_id
    let h = a.w.ctx.engine.read("history.list", json!({})).unwrap();
    let ops: Vec<String> = h["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            e["actor"]["id"]
                .as_str()
                .is_some_and(|x| x.starts_with("run:"))
        })
        .map(|e| e["operation_id"].as_str().unwrap_or("").to_owned())
        .collect();
    let mut u = ops.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), ops.len());
    eprintln!("SOAK12 seconds={secs:.2} history_entries={}", ops.len());
    // folga larga (CI lento/Windows): a regressão que importa é ordem de grandeza
    assert!(secs < 90.0, "12 deliverables took {secs:.1}s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "medição (cargo test --release -p capia-intelligence --test autonomy_perf -- --ignored --nocapture)"]
async fn perf_orchestrator_overhead_single_deliverable() {
    let Some(a) = many("perf1", 1) else { return };
    let mut samples = Vec::new();
    for _ in 0..5 {
        let run = a.create(a.inputs(), auto_policy());
        let t0 = std::time::Instant::now();
        let done = a.run_to_rest(&run.id).await;
        assert_eq!(done.status, RunStatus::Completed);
        samples.push(t0.elapsed().as_secs_f64());
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "PERF run_total_s median={:.3} max={:.3}",
        samples[2], samples[4]
    );
}
