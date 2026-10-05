//! Variantes e undo seletivo ponta a ponta (PHASE5_ORCHESTRATOR §variantes): master + variantes
//! de gancho/formato na MESMA Run, linhagem, 100% editáveis à mão, e desfazer só o que a IA fez.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::machine::RunStatus;
use capia_intelligence::autonomy::model::VariantRequest;
use common::auto::*;
use common::*;
use serde_json::{Value, json};

fn variants_world(name: &str) -> Option<AutoWorld> {
    auto_world(name, |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([
                        {"key": "main", "sequence_strategy": "standalone"},
                        {"key": "hookb", "sequence_strategy": "hook_plus_master", "master": "main", "hooks": ["h2"]},
                        {"key": "v916", "sequence_strategy": "format_variant", "variant_of": "main"}
                    ]),
                    json!([]),
                )
            },
            move |_d, _n| edit_json(&asset.lock().unwrap(), json!([])),
            |_n| json!({"findings": []}),
        )
    })
}

fn clips_of(a: &AutoWorld, seq: &str) -> Vec<String> {
    let s =
        a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": seq}))
            .unwrap();
    s["clips"].as_object().unwrap().keys().cloned().collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_run_makes_a_master_and_variants_that_are_distinct_editable_sequences() {
    let Some(a) = variants_world("var-one") else {
        return;
    };
    let mut inputs = a.inputs();
    inputs.variants = Some(VariantRequest {
        count: 2,
        axis: vec!["hooks".into(), "formats".into()],
    });
    let run = a.create(inputs, auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    let seqs = &done.sequences;
    assert_eq!(seqs.len(), 3, "{seqs:?}");
    let mut ids: Vec<_> = seqs.iter().map(|s| s.sequence_id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "distinct sequence ids");
    let roles: Vec<_> = seqs.iter().map(|s| s.role.as_str()).collect();
    assert!(
        roles.contains(&"standalone")
            && roles.contains(&"hook")
            && roles.contains(&"format_variant"),
        "{roles:?}"
    );
    for s in seqs {
        assert!(
            !clips_of(&a, &s.sequence_id).is_empty(),
            "{} has clips",
            s.sequence_id
        );
    }
    // 100% editável à mão: o usuário adiciona uma trilha numa variante pelo mesmo Command Engine
    let v = &seqs
        .iter()
        .find(|s| s.role == "format_variant")
        .unwrap()
        .sequence_id;
    call(
        &a.w.session,
        "command.execute",
        json!({"label": "manual", "commands": [{"operation_id": "man1", "type": "add_track", "sequence": v, "id": "manual_track", "kind": "visual"}]}),
    );
    let s =
        a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": v}))
            .unwrap();
    assert!(
        s["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "manual_track")
    );
}

fn undo(a: &AutoWorld, method: &str, p: Value) -> Value {
    call(&a.w.session, method, p)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn undoing_a_run_removes_only_what_the_ai_made_and_manual_edits_win_conflicts() {
    let Some(a) = variants_world("var-undo") else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    let actor = format!("run:{}", run.id);
    // edição manual do usuário em OUTRA sequência (a do fixture "s"): sobrevive ao undo da Run
    call(
        &a.w.session,
        "command.execute",
        json!({"label": "manual elsewhere", "commands": [{"operation_id": "man0", "type": "add_track", "sequence": "s", "id": "mine", "kind": "visual"}]}),
    );
    let before = serde_json::to_string(
        &a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": "s"}))
            .unwrap(),
    )
    .unwrap();
    assert!(before.contains("\"mine\""));
    let report = undo(&a, "history.undo_report", json!({"actor_id": actor}));
    assert!(
        report["conflicts"].as_array().unwrap().is_empty(),
        "{report}"
    );
    let r = undo(
        &a,
        "history.undo_selective",
        json!({"actor_id": actor, "label": "Undo AI run"}),
    );
    assert!(r.to_string().contains("undone"), "{r}");
    // as sequences da IA sumiram; a edição manual permanece
    for s in &done.sequences {
        assert!(
            a.w.ctx
                .engine
                .read("sequence.get", json!({"sequence": s.sequence_id}))
                .is_err(),
            "{} should be gone",
            s.sequence_id
        );
    }
    let after = serde_json::to_string(
        &a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": "s"}))
            .unwrap(),
    )
    .unwrap();
    assert!(after.contains("\"mine\""), "the manual edit is untouched");
    // o histórico NÃO foi reescrito: o undo é uma entrada nova
    let h = a.w.ctx.engine.read("history.list", json!({})).unwrap();
    assert!(h["entries"].as_array().unwrap().len() >= 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_edit_on_the_ai_output_is_reported_as_a_conflict_and_never_erased_in_safe_mode() {
    let Some(a) = variants_world("var-conflict") else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    let actor = format!("run:{}", run.id);
    let seq = done.sequences[0].sequence_id.clone();
    call(
        &a.w.session,
        "command.execute",
        json!({"label": "manual on ai output", "commands": [{"operation_id": "man2", "type": "add_track", "sequence": seq, "id": "manual_on_ai", "kind": "visual"}]}),
    );
    let report = undo(&a, "history.undo_report", json!({"actor_id": actor}));
    assert!(
        !report["conflicts"].as_array().unwrap().is_empty(),
        "{report}"
    );
    // modo seguro: não desfaz nada que conflite com o trabalho manual
    let res =
        a.w.session
            .lock()
            .unwrap()
            .call("history.undo_selective", json!({"actor_id": actor}));
    let still =
        a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": seq}))
            .unwrap();
    assert!(
        still["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "manual_on_ai"),
        "the manual track survived (result: {res:?})"
    );
}
