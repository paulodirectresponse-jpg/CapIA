//! Executor: limites, prioridade sem starvation (fairness), cancelamento, dedup, pânico,
//! desligamento e persistência observada por `JobSink`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_jobs::*;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn exec(workers: usize) -> Executor {
    Executor::new(
        ExecutorConfig {
            workers,
            ..ExecutorConfig::default()
        },
        None,
    )
}

fn spec(p: Priority) -> JobSpec {
    JobSpec::new(JobKind::Custom("t".into()), p)
}

/// Job que bloqueia até receber o sinal (ou ser cancelado).
fn gate() -> (mpsc::Sender<()>, Arc<Mutex<mpsc::Receiver<()>>>) {
    let (tx, rx) = mpsc::channel();
    (tx, Arc::new(Mutex::new(rx)))
}

#[test]
fn runs_jobs_and_reports_state_progress_and_result() {
    let e = exec(2);
    let s = e
        .submit(
            spec(Priority::Normal).label("sum").params(json!({"n": 3})),
            |ctx| {
                for i in 1..=4 {
                    ctx.set_progress(i, 4);
                }
                Ok(json!({ "sum": 6 }))
            },
        )
        .unwrap();
    assert!(!s.deduplicated);
    let snap = s.handle.wait();
    assert_eq!(snap.state, JobState::Completed);
    assert_eq!(snap.result, Some(json!({ "sum": 6 })));
    assert_eq!(snap.progress, Progress { done: 4, total: 4 });
    assert_eq!(snap.params, json!({ "n": 3 }));
    assert!(snap.started_ms.unwrap() >= snap.created_ms);
    assert!(snap.finished_ms.unwrap() >= snap.started_ms.unwrap());
    assert_eq!(e.get(&snap.id).unwrap().state, JobState::Completed);
}

#[test]
fn progress_never_regresses() {
    let e = exec(1);
    let h = e
        .submit(spec(Priority::Normal), |ctx| {
            ctx.set_progress(50, 100);
            ctx.set_progress(10, 100); // regressão ignorada
            Ok(Value::Null)
        })
        .unwrap()
        .handle;
    assert_eq!(h.wait().progress.done, 50);
}

#[test]
fn failures_and_panics_are_structured_and_do_not_kill_the_worker() {
    let e = exec(1);
    let bad = e
        .submit(spec(Priority::Normal), |_| {
            Err(JobError::new("X_FAILED", "boom"))
        })
        .unwrap()
        .handle
        .wait();
    assert_eq!(bad.state, JobState::Failed);
    assert_eq!(bad.error.unwrap().code, "X_FAILED");
    let panicked = e
        .submit(spec(Priority::Normal), |_| -> Result<Value, JobError> {
            panic!("kaboom")
        })
        .unwrap()
        .handle
        .wait();
    assert_eq!(panicked.state, JobState::Failed);
    assert_eq!(panicked.error.unwrap().code, CODE_PANICKED);
    // o único worker continua vivo
    let ok = e
        .submit(spec(Priority::Normal), |_| Ok(json!(1)))
        .unwrap()
        .handle
        .wait();
    assert_eq!(ok.state, JobState::Completed);
}

#[test]
fn queues_are_bounded() {
    let e = Executor::new(
        ExecutorConfig {
            workers: 1,
            queue_capacity: [2, 2, 2],
            ..ExecutorConfig::default()
        },
        None,
    );
    let (tx, rx) = gate();
    let blocker = {
        let rx = Arc::clone(&rx);
        e.submit(spec(Priority::Normal), move |_| {
            rx.lock().unwrap().recv().ok();
            Ok(Value::Null)
        })
        .unwrap()
        .handle
    };
    while blocker.snapshot().state != JobState::Running {
        std::thread::yield_now();
    }
    for _ in 0..2 {
        e.submit(spec(Priority::Background), |_| Ok(Value::Null))
            .unwrap();
    }
    let err = e
        .submit(spec(Priority::Background), |_| Ok(Value::Null))
        .unwrap_err();
    assert!(
        matches!(
            err,
            SubmitError::QueueFull {
                priority: Priority::Background,
                capacity: 2
            }
        ),
        "{err}"
    );
    // as outras categorias têm fila própria
    assert!(
        e.submit(spec(Priority::Interactive), |_| Ok(Value::Null))
            .is_ok()
    );
    tx.send(()).unwrap();
}

#[test]
fn interactive_goes_first_but_background_never_starves() {
    // 1 worker; enche as três filas enquanto ele está ocupado, depois solta e registra a ORDEM
    let e = Executor::new(
        ExecutorConfig {
            workers: 1,
            queue_capacity: [64, 64, 64],
            credits: [6, 3, 1],
            ..ExecutorConfig::default()
        },
        None,
    );
    let (tx, rx) = gate();
    let blocker = {
        let rx = Arc::clone(&rx);
        e.submit(spec(Priority::Normal), move |_| {
            rx.lock().unwrap().recv().ok();
            Ok(Value::Null)
        })
        .unwrap()
        .handle
    };
    while blocker.snapshot().state != JobState::Running {
        std::thread::yield_now();
    }
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let mut handles = Vec::new();
    for (p, tag, n) in [
        (Priority::Background, "B", 12),
        (Priority::Normal, "N", 12),
        (Priority::Interactive, "I", 30),
    ] {
        for _ in 0..n {
            let o = Arc::clone(&order);
            handles.push(
                e.submit(spec(p), move |_| {
                    o.lock().unwrap().push(tag);
                    Ok(Value::Null)
                })
                .unwrap()
                .handle,
            );
        }
    }
    tx.send(()).unwrap();
    for h in &handles {
        h.wait();
    }
    let order = order.lock().unwrap().clone();
    assert_eq!(order.len(), 54);
    // interativo domina o início…
    assert!(order[..6].iter().all(|t| *t == "I"), "{order:?}");
    // …mas o background roda ANTES de a fila interativa esvaziar (créditos 6/3/1 por ciclo):
    let first_b = order.iter().position(|t| *t == "B").unwrap();
    let last_i = order.iter().rposition(|t| *t == "I").unwrap();
    assert!(
        first_b < last_i,
        "background starved until interactive drained: {order:?}"
    );
    assert!(
        first_b <= 10,
        "first background job ran too late (index {first_b}): {order:?}"
    );
    // o interativo tem preferência: sua posição média é bem mais cedo que a do background
    let avg = |tag| {
        let v: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == tag)
            .map(|(i, _)| i)
            .collect();
        v.iter().sum::<usize>() as f64 / v.len() as f64
    };
    assert!(avg("I") < avg("N") && avg("N") < avg("B"), "{order:?}");
}

#[test]
fn a_flood_of_interactive_jobs_does_not_starve_background_forever() {
    let e = exec(1);
    let stop = Arc::new(AtomicBool::new(false));
    // produtor inunda a fila interativa
    let producer = {
        let (stop, ex) = (Arc::clone(&stop), &e);
        let ran = Arc::new(AtomicUsize::new(0));
        let ran2 = Arc::clone(&ran);
        let bg = e
            .submit(spec(Priority::Background), |_| Ok(json!("bg")))
            .unwrap()
            .handle;
        let mut submitted = 0;
        while !bg.is_done() && submitted < 5_000 {
            let r = Arc::clone(&ran2);
            if ex
                .submit(spec(Priority::Interactive), move |_| {
                    r.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_micros(200));
                    Ok(Value::Null)
                })
                .is_ok()
            {
                submitted += 1;
            } else {
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        stop.store(true, Ordering::SeqCst);
        assert!(
            bg.is_done(),
            "the background job starved under an interactive flood"
        );
        ran.load(Ordering::SeqCst)
    };
    assert!(producer > 0);
}

#[test]
fn cancelling_a_queued_job_removes_it_and_never_runs_it() {
    let e = exec(1);
    let (tx, rx) = gate();
    let blocker = {
        let rx = Arc::clone(&rx);
        e.submit(spec(Priority::Normal), move |_| {
            rx.lock().unwrap().recv().ok();
            Ok(Value::Null)
        })
        .unwrap()
        .handle
    };
    while blocker.snapshot().state != JobState::Running {
        std::thread::yield_now();
    }
    let ran = Arc::new(AtomicBool::new(false));
    let r2 = Arc::clone(&ran);
    let h = e
        .submit(spec(Priority::Background), move |_| {
            r2.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        })
        .unwrap()
        .handle;
    assert!(e.cancel(&h.id()));
    assert_eq!(h.snapshot().state, JobState::Cancelled);
    assert!(h.snapshot().error.unwrap().is_cancelled());
    assert!(!e.cancel(&h.id()), "already terminal");
    tx.send(()).unwrap();
    blocker.wait();
    std::thread::sleep(Duration::from_millis(50));
    assert!(!ran.load(Ordering::SeqCst), "a cancelled job must not run");
}

#[test]
fn cancelling_a_running_job_trips_its_token_and_it_stops_promptly() {
    let e = exec(1);
    let started = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&started);
    let h = e
        .submit(spec(Priority::Normal), move |ctx| {
            s2.store(true, Ordering::SeqCst);
            loop {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(2));
            }
        })
        .unwrap()
        .handle;
    while !started.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let t = Instant::now();
    assert!(e.cancel(&h.id()));
    let snap = h.wait();
    assert_eq!(snap.state, JobState::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn identical_active_work_is_deduplicated_and_runs_once() {
    let e = exec(2);
    let runs = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = gate();
    let mk = |runs: Arc<AtomicUsize>, rx: Arc<Mutex<mpsc::Receiver<()>>>| {
        move |_: &JobCtx| {
            runs.fetch_add(1, Ordering::SeqCst);
            rx.lock().unwrap().recv().ok();
            Ok(json!("done"))
        }
    };
    let a = e
        .submit(
            spec(Priority::Normal).dedup("k"),
            mk(Arc::clone(&runs), Arc::clone(&rx)),
        )
        .unwrap();
    let b = e
        .submit(
            spec(Priority::Normal).dedup("k"),
            mk(Arc::clone(&runs), Arc::clone(&rx)),
        )
        .unwrap();
    assert!(!a.deduplicated && b.deduplicated);
    assert_eq!(a.handle.id(), b.handle.id());
    tx.send(()).unwrap();
    assert_eq!(b.handle.wait().state, JobState::Completed);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    // depois de terminar, a chave é liberada
    let c = e
        .submit(spec(Priority::Normal).dedup("k"), |_| Ok(json!(2)))
        .unwrap();
    assert!(!c.deduplicated);
    assert_eq!(c.handle.wait().result, Some(json!(2)));
}

#[test]
fn shutdown_interrupts_queued_and_running_jobs_so_they_can_be_retried() {
    let e = exec(1);
    let started = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&started);
    let running = e
        .submit(spec(Priority::Normal), move |ctx| {
            s2.store(true, Ordering::SeqCst);
            loop {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(2));
            }
        })
        .unwrap()
        .handle;
    while !started.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let queued = e
        .submit(spec(Priority::Background), |_| Ok(Value::Null))
        .unwrap()
        .handle;
    e.shutdown();
    assert_eq!(running.snapshot().state, JobState::Interrupted);
    assert_eq!(queued.snapshot().state, JobState::Interrupted);
    assert!(matches!(
        e.submit(spec(Priority::Normal), |_| Ok(Value::Null)),
        Err(SubmitError::ShuttingDown)
    ));
}

#[derive(Default)]
struct Rec(Mutex<Vec<(String, JobState)>>);
impl JobSink for Rec {
    fn record(&self, s: &JobSnapshot) {
        self.0.lock().unwrap().push((s.id.0.clone(), s.state));
    }
}

#[test]
fn the_sink_sees_every_transition_in_order() {
    let rec = Arc::new(Rec::default());
    let e = Executor::new(
        ExecutorConfig {
            workers: 1,
            ..ExecutorConfig::default()
        },
        Some(Arc::clone(&rec) as Arc<dyn JobSink>),
    );
    let h = e
        .submit(spec(Priority::Normal), |_| Ok(Value::Null))
        .unwrap()
        .handle;
    h.wait();
    e.shutdown();
    let seen: Vec<JobState> = rec.0.lock().unwrap().iter().map(|(_, s)| *s).collect();
    assert_eq!(
        seen,
        [JobState::Queued, JobState::Running, JobState::Completed]
    );
}

#[test]
fn a_thousand_small_jobs_complete_with_bounded_threads() {
    let e = Executor::new(
        ExecutorConfig {
            workers: 4,
            queue_capacity: [2000, 2000, 2000],
            ..ExecutorConfig::default()
        },
        None,
    );
    let counter = Arc::new(AtomicUsize::new(0));
    let hs: Vec<JobHandle> = (0..1000)
        .map(|_| {
            let c = Arc::clone(&counter);
            e.submit(spec(Priority::Normal), move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Null)
            })
            .unwrap()
            .handle
        })
        .collect();
    for h in &hs {
        assert_eq!(h.wait().state, JobState::Completed);
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1000);
}

#[test]
fn kinds_priorities_and_states_round_trip_as_text() {
    for k in [
        JobKind::AssetHash,
        JobKind::FrameIndex,
        JobKind::Waveform,
        JobKind::Proxy,
        JobKind::Thumbnail,
        JobKind::BatchRelink,
        JobKind::Custom("x".into()),
    ] {
        assert_eq!(JobKind::parse(&k.as_str()), Some(k));
    }
    for p in Priority::ALL {
        assert_eq!(Priority::parse(p.as_str()), Some(p));
    }
    for s in [
        JobState::Queued,
        JobState::Running,
        JobState::Completed,
        JobState::Failed,
        JobState::Cancelled,
        JobState::Interrupted,
    ] {
        assert_eq!(JobState::parse(s.as_str()), Some(s));
    }
    assert_eq!(JobKind::parse("nope"), None);
}
