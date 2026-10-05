//! AI Run ponta a ponta (Replay, sem rede): UNDERSTAND → PLAN → VALIDATE_PLAN → EDIT → REVIEW →
//! DONE sobre projeto e mídia reais.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::machine::RunStatus;
use common::auto::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy_path_runs_the_whole_pipeline_and_leaves_editable_clips() {
    let Some(a) = auto_world("auto-happy", simple_script) else { return };
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?} {:?}", done.error, done.pending);
    assert_eq!(a.stages_visited(&run.id), ["understand", "plan", "validate_plan", "edit", "review"]);
    // saída = sequence normal com clips editáveis (ator da Run no histórico)
    assert_eq!(done.sequences.len(), 1);
    let seq = a.w.ctx.engine.read("sequence.get", json!({"sequence": done.sequences[0].sequence_id})).unwrap();
    assert!(seq["clips"].as_object().unwrap().len() >= 4, "{seq}");
    assert!(a.history_actors().iter().all(|x| x == &format!("run:{}", run.id) || x == "user" || !x.starts_with("run:")));
    assert!(a.history_actors().contains(&format!("run:{}", run.id)));
    // nenhuma escrita antes do VALIDATE_PLAN concluir
    let validate = a.orch.store().list_stages(&run.id).unwrap().into_iter().find(|s| s.stage == "validate_plan").unwrap();
    let first_apply = a.spy.log.lock().unwrap().iter().find(|e| e.0 == "apply").unwrap().1;
    assert!(first_apply >= validate.ended_ms.unwrap(), "apply before validation finished");
    assert!(done.report.is_some());
}
