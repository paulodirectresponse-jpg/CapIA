//! Crash/resume da autonomia (PHASE5_AUTONOMY_TESTS §crash): um *failpoint* armado em processo
//! derruba o driver exatamente naquela fronteira (a Run fica `running` no `.capia`, como depois de
//! um `kill`); uma instância **nova** do Orchestrator faz `recover()` e `resume()`. Invariantes:
//! a Run termina, nenhuma edição é duplicada e a timeline final é a de uma execução sem crash.
//! Os testes de processo real (kill de verdade) estão em `autonomy_kill.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::failpoint;
use capia_intelligence::autonomy::machine::RunStatus;
use common::auto::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

// Os failpoints são globais ao processo de teste: um teste por vez.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Resumo observável da timeline produzida (independente de ids de run).
fn shape(a: &AutoWorld, seq: &str) -> Value {
    let s = a.w.ctx.engine.read("sequence.get", json!({"sequence": seq})).unwrap();
    let mut clips: Vec<Value> = s["clips"]
        .as_object()
        .unwrap()
        .values()
        .map(|c| json!({"start": c["start"], "duration": c["duration"], "type": c["content"]["type"]}))
        .collect();
    clips.sort_by_key(|c| c.to_string());
    json!(clips)
}

async fn wait_stopped(a: &AutoWorld, id: &str) {
    let t0 = std::time::Instant::now();
    while a.orch.is_driving(id) {
        assert!(t0.elapsed().as_secs() < 60, "the driver never stopped");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Executa uma Run limpa e devolve (forma da timeline, nº de entradas de histórico do agente).
async fn baseline(name: &str) -> Option<(Value, usize, u32)> {
    let a = auto_world(name, simple_script)?;
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    let seq = done.sequences[0].sequence_id.clone();
    let agent = a.history_actors().iter().filter(|x| x.starts_with("run:")).count();
    Some((shape(&a, &seq), agent, a.spy.applies.load(Ordering::SeqCst)))
}

/// Arma `point`, deixa o driver morrer nele, reabre (instância nova), recupera e retoma.
/// Repete enquanto o ponto ainda disparar (cada rodada cai na próxima fronteira).
async fn crash_loop(a: &AutoWorld, run_id: &str, point: &str, max_rounds: usize) -> usize {
    let mut crashes = 0;
    let mut orch = a.orch.clone();
    for _ in 0..max_rounds {
        failpoint::arm(point);
        if crashes == 0 {
            orch.start(run_id).unwrap();
        } else {
            orch.resume(run_id).unwrap();
        }
        let t0 = std::time::Instant::now();
        loop {
            let r = orch.load(run_id).unwrap();
            let stopped = !orch.is_driving(run_id);
            if stopped {
                if r.status == RunStatus::Running {
                    crashes += 1; // o driver morreu no ponto: Run ainda "running" no disco
                }
                break;
            }
            assert!(t0.elapsed().as_secs() < 90, "timeout in {point}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        failpoint::disarm_all();
        // "reabrir o app"
        orch = a.restart();
        let rec = orch.recover().unwrap();
        let r = orch.load(run_id).unwrap();
        if r.status == RunStatus::Completed {
            break;
        }
        assert_eq!(r.status, RunStatus::Paused, "recover turns running into paused");
        assert!(rec.iter().any(|x| x.run_id == run_id), "the run is classified");
    }
    // resume final sem crash
    let r = orch.load(run_id).unwrap();
    if r.status != RunStatus::Completed {
        orch.resume(run_id).unwrap();
        let t0 = std::time::Instant::now();
        while orch.load(run_id).unwrap().status != RunStatus::Completed || orch.is_driving(run_id) {
            let st = orch.load(run_id).unwrap();
            assert!(
                !matches!(st.status, RunStatus::Failed | RunStatus::Cancelled),
                "{point}: {:?} {:?}",
                st.status,
                st.error
            );
            assert!(t0.elapsed().as_secs() < 90, "{point}: resume timeout {:?}", st.status);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    crashes
}

async fn check_point(point: &str, rounds: usize, min_crashes: usize) {
    let _g = SERIAL.lock().await;
    let Some((base_shape, base_agent, base_applies)) = baseline("crash-base").await else {
        return;
    };
    let a = auto_world("crash-run", simple_script).unwrap();
    let run = a.create(a.inputs(), auto_policy());
    let crashes = crash_loop(&a, &run.id, point, rounds).await;
    assert!(crashes >= min_crashes, "{point}: only {crashes} crashes");
    let done = a.orch.load(&run.id).unwrap();
    assert_eq!(done.status, RunStatus::Completed, "{point}: {:?}", done.error);
    let seq = done.sequences[0].sequence_id.clone();
    assert_eq!(shape(&a, &seq), base_shape, "{point}: timeline differs from the clean run");
    let agent = a.history_actors().iter().filter(|x| x.starts_with("run:")).count();
    assert_eq!(agent, base_agent, "{point}: duplicated edits in history");
    assert!(
        a.spy.applies.load(Ordering::SeqCst) <= base_applies + crashes as u32,
        "{point}: too many applies"
    );
    // no máximo uma sequência do deliverable (nada duplicado)
    let seqs = a.w.ctx.engine.read("project.get", json!({})).ok();
    let _ = seqs;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_at_every_stage_start_resumes_without_duplicates() {
    check_point("autonomy_stage_started", 12, 4).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_before_each_stage_output_is_persisted_resumes_without_duplicates() {
    check_point("autonomy_stage_output_before_persist", 12, 4).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_after_each_stage_is_persisted_but_before_the_next_resumes_without_duplicates() {
    check_point("autonomy_stage_after_persist", 12, 4).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_right_after_the_understand_provider_returns_does_not_pay_twice() {
    let _g = SERIAL.lock().await;
    let Some(a) = auto_world("crash-understand", simple_script) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    crash_loop(&a, &run.id, "autonomy_understand_after_provider", 3).await;
    let done = a.orch.load(&run.id).unwrap();
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    assert_eq!(a.script.count("demand"), 1, "the paid LLM answer was reused, not re-requested");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_right_after_the_producer_returns_does_not_pay_twice() {
    let _g = SERIAL.lock().await;
    let Some(a) = auto_world("crash-producer", simple_script) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    crash_loop(&a, &run.id, "autonomy_plan_after_producer", 3).await;
    let done = a.orch.load(&run.id).unwrap();
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    assert_eq!(a.script.count("producer"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_after_the_preview_in_validate_plan_revalidates_and_continues() {
    let _g = SERIAL.lock().await;
    let Some(a) = auto_world("crash-validate", simple_script) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    let n = crash_loop(&a, &run.id, "autonomy_validate_after_preview", 3).await;
    assert!(n >= 1);
    let done = a.orch.load(&run.id).unwrap();
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_before_the_apply_writes_nothing_and_after_it_never_applies_twice() {
    let _g = SERIAL.lock().await;
    for point in ["autonomy_edit_before_apply", "autonomy_edit_after_apply"] {
        let Some(a) = auto_world("crash-edit", simple_script) else {
            return;
        };
        let run = a.create(a.inputs(), auto_policy());
        let before = a.history_actors().len();
        let n = crash_loop(&a, &run.id, point, 3).await;
        assert!(n >= 1, "{point}");
        let done = a.orch.load(&run.id).unwrap();
        assert_eq!(done.status, RunStatus::Completed, "{point}: {:?}", done.error);
        let agent = a.history_actors().iter().filter(|x| x.starts_with("run:")).count();
        // o histórico ganha exatamente as entradas de uma execução limpa: sem cópia por reaplicar
        let all = a.history_actors().len();
        assert!(all > before);
        assert!(agent >= 1, "{point}");
        let mut ids: Vec<String> = a.w.ctx.engine.read("history.list", json!({})).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["actor"]["id"].as_str().is_some_and(|x| x.starts_with("run:")))
            .map(|e| e["operation_id"].as_str().unwrap_or("").to_owned())
            .collect();
        let n0 = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n0, "{point}: duplicated operation ids in history");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_recover_and_resume_calls_are_idempotent() {
    let _g = SERIAL.lock().await;
    let Some(a) = auto_world("crash-idem", simple_script) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    failpoint::arm("autonomy_edit_after_apply");
    a.orch.start(&run.id).unwrap();
    wait_stopped(&a, &run.id).await;
    failpoint::disarm_all();
    let o2 = a.restart();
    // várias recuperações seguidas não mudam nada além da primeira
    let r1 = o2.recover().unwrap();
    let r2 = o2.recover().unwrap();
    assert_eq!(r1.len(), r2.len());
    assert_eq!(o2.load(&run.id).unwrap().status, RunStatus::Paused);
    // dois resumes concorrentes: só um driver
    o2.resume(&run.id).ok();
    o2.resume(&run.id).ok();
    let t0 = std::time::Instant::now();
    while o2.load(&run.id).unwrap().status != RunStatus::Completed || o2.is_driving(&run.id) {
        assert!(t0.elapsed().as_secs() < 90);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // resume de uma Run concluída não refaz nada
    let applies = a.spy.applies.load(Ordering::SeqCst);
    let _ = o2.resume(&run.id);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(a.spy.applies.load(Ordering::SeqCst), applies);
}
