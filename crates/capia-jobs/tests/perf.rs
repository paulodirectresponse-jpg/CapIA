//! Medições do executor (ignoradas por padrão):
//! `cargo test --release -p capia-jobs --test perf -- --ignored --nocapture`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_jobs::{Executor, ExecutorConfig, JobKind, JobSpec, JobState, Priority};
use serde_json::json;
use std::time::{Duration, Instant};

fn exec(workers: usize) -> Executor {
    Executor::new(
        ExecutorConfig {
            workers,
            queue_capacity: [4096; 3],
            keep_finished: 4096,
            ..ExecutorConfig::default()
        },
        None,
    )
}

#[test]
#[ignore = "medição"]
fn one_thousand_tiny_jobs_overhead_queueing_and_cancel() {
    // overhead por job (trabalho nulo, 4 workers)
    let e = exec(4);
    let t0 = Instant::now();
    let handles: Vec<_> = (0..1000)
        .map(|i| {
            e.submit(
                JobSpec::new(JobKind::Custom("tiny".into()), Priority::Normal),
                move |_| Ok(json!(i)),
            )
            .unwrap()
            .handle
        })
        .collect();
    let submit_time = t0.elapsed();
    for h in &handles {
        assert_eq!(h.wait().state, JobState::Completed);
    }
    let total = t0.elapsed();
    println!(
        "PERF jobs.submit_1000: {submit_time:?} ({:?}/job)",
        submit_time / 1000
    );
    println!(
        "PERF jobs.run_1000_noop: {total:?} ({:?}/job overhead)",
        total / 1000
    );
    assert!(total < Duration::from_secs(5));

    // fila: 1000 jobs lentos numa fila de 1 worker; cancelar todos
    let e = exec(1);
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let g = gate.clone();
    let blocker = e
        .submit(
            JobSpec::new(JobKind::Custom("block".into()), Priority::Interactive),
            move |ctx| {
                while !g.load(std::sync::atomic::Ordering::SeqCst) && !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(json!(null))
            },
        )
        .unwrap()
        .handle;
    let queued: Vec<_> = (0..1000)
        .map(|_| {
            e.submit(
                JobSpec::new(JobKind::Custom("q".into()), Priority::Background),
                |_| Ok(json!(null)),
            )
            .unwrap()
            .handle
        })
        .collect();
    let t0 = Instant::now();
    for h in &queued {
        assert!(e.cancel(&h.id()));
    }
    let cancel_time = t0.elapsed();
    println!(
        "PERF jobs.cancel_1000_queued: {cancel_time:?} ({:?}/job)",
        cancel_time / 1000
    );
    assert!(
        queued
            .iter()
            .all(|h| h.snapshot().state == JobState::Cancelled)
    );
    gate.store(true, std::sync::atomic::Ordering::SeqCst);
    blocker.wait();
    // latência de despacho de um job interativo com a fila de fundo cheia
    let e = exec(2);
    let bg: Vec<_> = (0..200)
        .map(|_| {
            e.submit(
                JobSpec::new(JobKind::Custom("bg".into()), Priority::Background),
                |_| {
                    std::thread::sleep(Duration::from_millis(2));
                    Ok(json!(null))
                },
            )
            .unwrap()
            .handle
        })
        .collect();
    let t0 = Instant::now();
    let h = e
        .submit(
            JobSpec::new(JobKind::Custom("i".into()), Priority::Interactive),
            |_| Ok(json!(1)),
        )
        .unwrap()
        .handle;
    h.wait();
    println!(
        "PERF jobs.interactive_latency_behind_200_background: {:?}",
        t0.elapsed()
    );
    for b in bg {
        b.wait();
    }
}
