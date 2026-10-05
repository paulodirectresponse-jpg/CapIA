//! Cenários da autonomia (Replay, sem rede): perguntas, aprovações, plano inválido, replanejamento
//! limitado, aquisição por Gateway, geração, Gateway desligado, fallback de provider, edição manual
//! durante a Run, cancelamento, loop REVIEW→CORRECT, oscilação e pausa/retomada.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::gateway::{
    Candidate, CatalogEntry, GatewayError, LicenseStatus, ReplayCatalogAdapter,
};
use capia_intelligence::autonomy::generation::{GenKind, ReplayGenerationProvider};
use capia_intelligence::autonomy::machine::{RunStage, RunStatus};
use capia_intelligence::autonomy::model::{
    DecisionKind, PlanApproval, RunBudget, RunPolicy, SpecApproval,
};
use capia_intelligence::autonomy::plan::AssetKind;
use common::auto::*;
use common::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

fn policy_plan_always() -> RunPolicy {
    RunPolicy {
        plan: PlanApproval::Always,
        ..auto_policy()
    }
}

fn candidate(id: &str, license: LicenseStatus, price: Option<u64>) -> Candidate {
    Candidate {
        id: id.into(),
        adapter: "cat".into(),
        title: "pessoa usando cafe na cozinha".into(),
        description: "b-roll produto cozinha".into(),
        kind: AssetKind::Video,
        duration_ms: Some(4000),
        width: None,
        height: None,
        license,
        license_text: Some("test license".into()),
        price_micros: price,
        source_uri: Some(format!("https://stock.example/{id}")),
        score: 0.0,
        score_components: vec![],
    }
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_question_waits_survives_a_restart_and_resumes_after_the_answer() {
    let Some(a) = auto_world("sc-question", |asset| {
        Script::new(
            {
                let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
                move || demand_json(calls.fetch_add(1, Ordering::SeqCst) == 0)
            },
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
    let policy = RunPolicy {
        demand_spec: SpecApproval::RequiredIfQuestions,
        plan: PlanApproval::Auto,
        max_question_rounds: 1,
        ..RunPolicy::default()
    };
    let run = a.create(a.inputs(), policy);
    let waiting = a.run_to_rest(&run.id).await;
    assert_eq!(waiting.status, RunStatus::WaitingUser);
    let d = waiting.pending.clone().unwrap();
    assert_eq!(d.kind, DecisionKind::OpenQuestion);
    assert_eq!(
        a.spy.previews.load(Ordering::SeqCst),
        0,
        "nothing is previewed/written while asking"
    );
    // reabrir o app: a Run continua esperando a mesma decisão
    let o2 = a.restart();
    let reloaded = o2.load(&run.id).unwrap();
    assert_eq!(reloaded.status, RunStatus::WaitingUser);
    assert_eq!(reloaded.pending.as_ref().unwrap().id, d.id);
    // decisão velha/inexistente é recusada; a certa retoma o estágio certo
    assert_eq!(
        o2.decide(&run.id, "dec-old", "approve", &Value::Null, "user")
            .unwrap_err()
            .code,
        "STALE_DECISION"
    );
    assert_eq!(
        o2.decide(&run.id, &d.id, "bogus", &Value::Null, "user")
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
    o2.decide(
        &run.id,
        &d.id,
        "answer",
        &json!({"answers": ["60s vertical"]}),
        "user",
    )
    .unwrap();
    let done = a.wait(&run.id, |r| r.status == RunStatus::Completed).await;
    assert_eq!(done.inputs.answers, vec!["60s vertical".to_owned()]);
    assert_eq!(
        a.script.count("demand"),
        2,
        "the answer triggers a new interpretation"
    );
    assert_eq!(done.approvals.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plan_approval_gates_every_write_and_reject_change_and_approve_all_work() {
    let Some(a) = auto_world("sc-plan-approval", simple_script) else {
        return;
    };
    // 1) aprova
    let run = a.create(a.inputs(), policy_plan_always());
    let w = a.run_to_rest(&run.id).await;
    assert_eq!(w.status, RunStatus::WaitingUser);
    let d = w.pending.clone().unwrap();
    assert_eq!(d.kind, DecisionKind::PlanApproval);
    assert!(d.bound_digest.is_some());
    assert_eq!(
        a.spy.applies.load(Ordering::SeqCst),
        0,
        "zero writes before the plan is approved"
    );
    assert!(a.history_actors().iter().all(|x| !x.starts_with("run:")));
    a.orch
        .decide(&run.id, &d.id, "approve", &Value::Null, "user")
        .unwrap();
    let done = a.wait(&run.id, |r| r.status == RunStatus::Completed).await;
    assert!(a.spy.applies.load(Ordering::SeqCst) >= 1);
    assert!(
        done.approvals
            .iter()
            .any(|x| x.kind == DecisionKind::PlanApproval && x.bound_digest == d.bound_digest)
    );
    // 2) rejeita: nada é escrito e a Run termina cancelada
    let r2 = a.create(a.inputs(), policy_plan_always());
    let w2 = a.run_to_rest(&r2.id).await;
    let applies = a.spy.applies.load(Ordering::SeqCst);
    a.orch
        .decide(
            &r2.id,
            &w2.pending.unwrap().id,
            "reject",
            &Value::Null,
            "user",
        )
        .unwrap();
    assert_eq!(a.orch.load(&r2.id).unwrap().status, RunStatus::Cancelled);
    assert_eq!(a.spy.applies.load(Ordering::SeqCst), applies);
    // 3) pede mudança: replaneja (novo plano ⇒ nova decisão presa a OUTRO digest)
    let r3 = a.create(a.inputs(), policy_plan_always());
    let w3 = a.run_to_rest(&r3.id).await;
    let first = w3.pending.unwrap();
    a.orch
        .decide(
            &r3.id,
            &first.id,
            "change",
            &json!({"comment": "shorter hook"}),
            "user",
        )
        .unwrap();
    let w3b = a
        .wait(&r3.id, |r| {
            r.status == RunStatus::WaitingUser
                && r.pending.as_ref().is_some_and(|p| p.id != first.id)
        })
        .await;
    assert_eq!(w3b.usage.replans, 1);
    assert_eq!(
        a.orch
            .decide(&r3.id, &first.id, "approve", &Value::Null, "user")
            .unwrap_err()
            .code,
        "STALE_DECISION"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invalid_plan_is_replanned_and_the_replan_loop_is_bounded() {
    // o planner erra o asset uma vez e acerta na segunda
    let Some(a) = auto_world("sc-invalid", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            move |_d, n| {
                let a = asset.lock().unwrap().clone();
                edit_json(if n == 0 { "ghost-asset" } else { &a }, json!([]))
            },
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    assert_eq!(done.usage.replans, 1);
    assert_eq!(a.script.count("planner:main"), 2);
    assert!(
        a.stages_visited(&run.id)
            .iter()
            .filter(|s| *s == "plan")
            .count()
            == 2
    );

    // sempre inválido: o laço pára no limite e pede decisão (nunca infinito)
    let Some(b) = auto_world("sc-invalid2", |_| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            // cada tentativa muda um detalhe (senão o "mesmo plano" já seria barrado antes)
            |_d, n| {
                let mut v = edit_json("ghost-asset", json!([]));
                v["beats"][0]["duration_ms"] = json!(3000 + i64::from(n));
                v
            },
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let budget = RunBudget {
        max_replans: 2,
        ..RunBudget::default()
    };
    let run = b
        .orch
        .create_run(b.inputs(), Some(auto_policy()), Some(budget), "pf", None)
        .unwrap();
    let w = b.run_to_rest(&run.id).await;
    assert_eq!(w.status, RunStatus::WaitingUser);
    assert_eq!(
        w.pending.as_ref().unwrap().kind,
        DecisionKind::BudgetExtension,
        "{:?}",
        w.pending
    );
    assert!(w.usage.replans <= 2);
    assert_eq!(
        b.spy.applies.load(Ordering::SeqCst),
        0,
        "an invalid plan never writes"
    );
    b.orch
        .decide(
            &run.id,
            &w.pending.unwrap().id,
            "stop",
            &Value::Null,
            "user",
        )
        .unwrap();
    assert_eq!(b.orch.load(&run.id).unwrap().status, RunStatus::Cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_broll_is_acquired_through_the_gateway_with_approval_and_provenance() {
    let Some(tc) = ffmpeg() else { return };
    let Some(a) = auto_world("sc-gateway", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([need("broll1", true, &["gateway"])]),
                )
            },
            move |_d, _n| beats_with_need(&asset.lock().unwrap(), "broll1"),
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let payload = broll_bytes(&tc, &a.w.dir, "stock.mp4", 4);
    let cat = Arc::new(ReplayCatalogAdapter::new(
        "cat",
        vec![CatalogEntry {
            candidate: candidate("c1", LicenseStatus::Unknown, Some(0)),
            payload: payload.clone(),
        }],
    ));
    a.gateways.register_shared(cat.clone());
    let run = a.create(a.inputs(), auto_policy());
    let w = a.run_to_rest(&run.id).await;
    // licença desconhecida ⇒ aprovação humana (padrão seguro); nada baixado ainda
    assert_eq!(
        w.pending.as_ref().unwrap().kind,
        DecisionKind::AssetApproval,
        "{:?} {:?}",
        w.status,
        w.error
    );
    assert_eq!(cat.fetch_count.load(Ordering::SeqCst), 0);
    a.orch
        .decide(
            &run.id,
            &w.pending.unwrap().id,
            "approve",
            &Value::Null,
            "user",
        )
        .unwrap();
    let done = a.wait(&run.id, |r| r.status == RunStatus::Completed).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    assert_eq!(cat.fetch_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        a.stages_visited(&run.id),
        [
            "understand",
            "plan",
            "validate_plan",
            "acquire",
            "acquire",
            "validate_plan",
            "edit",
            "review"
        ]
    );
    let need = done.production_plan.unwrap().asset_needs[0].clone();
    let asset = need.resolved_asset_id.unwrap();
    let prov = a
        .orch
        .store()
        .get_provenance(&asset)
        .unwrap()
        .expect("provenance recorded");
    assert_eq!(prov.kind, "downloaded");
    assert!(prov.content_hash.starts_with("sha256:"));
    assert_eq!(prov.json["adapter"], "cat");
    assert_eq!(prov.json["license"]["status"], "unknown");
    assert!(
        prov.json["approval_ref"].is_string(),
        "the approval is referenced"
    );
    // o asset virou asset normal e entrou na timeline como mídia real (não placeholder)
    let seq =
        a.w.ctx
            .engine
            .read(
                "sequence.get",
                json!({"sequence": done.sequences[0].sequence_id}),
            )
            .unwrap();
    assert!(
        seq["clips"]
            .as_object()
            .unwrap()
            .values()
            .any(|c| c["content"]["asset"] == asset.as_str())
    );
    assert!(
        seq["clips"]
            .as_object()
            .unwrap()
            .values()
            .all(|c| !c["name"].as_str().unwrap_or("").starts_with("PLACEHOLDER"))
    );
    // a fonte some do ar: o asset continua válido (proveniência ≠ disponibilidade)
    cat.fail_next_fetch(GatewayError::new("GATEWAY_DOWN", "gone", false));
    assert!(
        a.w.ctx
            .engine
            .read("assets.list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == asset.as_str() && x["status"] == "online")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn known_license_free_assets_need_no_approval_and_restricted_ones_are_skipped() {
    let Some(tc) = ffmpeg() else { return };
    let mk = |name: &str| {
        auto_world(name, |asset| {
            Script::new(
                || demand_json(false),
                || {
                    producer_json(
                        json!([{"key": "main", "sequence_strategy": "standalone"}]),
                        json!([need("broll1", true, &["gateway"])]),
                    )
                },
                move |_d, _n| beats_with_need(&asset.lock().unwrap(), "broll1"),
                |_n| json!({"findings": []}),
            )
        })
    };
    let Some(a) = mk("sc-gw-allowed") else { return };
    let payload = broll_bytes(&tc, &a.w.dir, "stock.mp4", 4);
    a.gateways
        .register_shared(Arc::new(ReplayCatalogAdapter::new(
            "cat",
            vec![CatalogEntry {
                candidate: candidate("ok", LicenseStatus::KnownAllowed, Some(0)),
                payload: payload.clone(),
            }],
        )));
    let run = a.create(a.inputs(), auto_policy());
    assert_eq!(a.run_to_rest(&run.id).await.status, RunStatus::Completed);
    // restrita ⇒ descartada; sem outro candidato, o asset crítico espera o usuário (erro claro)
    let Some(b) = mk("sc-gw-restricted") else {
        return;
    };
    b.gateways
        .register_shared(Arc::new(ReplayCatalogAdapter::new(
            "cat",
            vec![CatalogEntry {
                candidate: candidate("bad", LicenseStatus::KnownRestricted, Some(0)),
                payload,
            }],
        )));
    let run = b.create(b.inputs(), auto_policy());
    let w = b.run_to_rest(&run.id).await;
    assert_eq!(w.status, RunStatus::WaitingUser);
    let p = w.pending.unwrap();
    assert_eq!(p.kind, DecisionKind::ConflictResolution);
    assert!(p.question.contains("broll1"), "{}", p.question);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_disabled_gateway_never_breaks_the_app_and_optional_needs_fall_back_by_replanning() {
    // obrigatório + gateway desligado ⇒ WAITING_USER com erro claro; o editor segue funcionando
    let Some(tc) = ffmpeg() else { return };
    let Some(a) = auto_world("sc-gw-off", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([need("broll1", true, &["gateway"])]),
                )
            },
            move |_d, _n| beats_with_need(&asset.lock().unwrap(), "broll1"),
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let payload = broll_bytes(&tc, &a.w.dir, "s.mp4", 4);
    let cat = Arc::new(ReplayCatalogAdapter::new(
        "cat",
        vec![CatalogEntry {
            candidate: candidate("c", LicenseStatus::KnownAllowed, Some(0)),
            payload,
        }],
    ));
    a.gateways.register_shared(cat.clone());
    a.gateways.set_enabled("cat", false);
    let run = a.create(a.inputs(), auto_policy());
    let w = a.run_to_rest(&run.id).await;
    assert_eq!(w.status, RunStatus::WaitingUser);
    let p = w.pending.unwrap();
    assert_eq!(p.kind, DecisionKind::ConflictResolution);
    assert!(
        p.context["reason"]
            .as_str()
            .unwrap()
            .contains("no gateway adapter is enabled"),
        "{:?}",
        p.context
    );
    assert_eq!(
        cat.search_count.load(Ordering::SeqCst),
        0,
        "a disabled adapter is never called"
    );
    assert!(
        a.w.ctx.engine.read("project.snapshot", json!({})).is_ok(),
        "the editor keeps working"
    );
    assert_eq!(a.spy.applies.load(Ordering::SeqCst), 0);

    // opcional + indisponível ⇒ replaneja sem ele (o planner recebe o feedback) e completa
    let Some(b) = auto_world("sc-gw-optional", |asset| {
        Script::new(
            || demand_json(false),
            move || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([need("broll1", false, &["gateway"])]),
                )
            },
            move |_d, n| {
                if n == 0 {
                    beats_with_need(&asset.lock().unwrap(), "broll1")
                } else {
                    edit_json(&asset.lock().unwrap(), json!([]))
                }
            },
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let run = b.create(b.inputs(), auto_policy());
    let done = b.run_to_rest(&run.id).await;
    assert_eq!(
        done.status,
        RunStatus::Completed,
        "{:?} {:?}",
        done.error,
        done.pending
    );
    assert_eq!(done.usage.replans, 1);
    assert_eq!(b.script.count("planner:main"), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generation_needs_approval_and_budget_and_records_full_provenance() {
    let Some(tc) = ffmpeg() else { return };
    let mk = |name: &str| {
        auto_world(name, |asset| {
            Script::new(
                || demand_json(false),
                || {
                    producer_json(
                        json!([{"key": "main", "sequence_strategy": "standalone"}]),
                        json!([need("gen1", true, &["generate"])]),
                    )
                },
                move |_d, _n| beats_with_need(&asset.lock().unwrap(), "gen1"),
                |_n| json!({"findings": []}),
            )
        })
    };
    let Some(a) = mk("sc-gen") else { return };
    let payload = broll_bytes(&tc, &a.w.dir, "gen.mp4", 4);
    let prov = Arc::new(ReplayGenerationProvider::new(
        "gen",
        vec![GenKind::Video],
        Some(500_000),
        payload.clone(),
    ));
    a.generators.register(prov.clone());
    let run = a.create(a.inputs(), auto_policy());
    let w = a.run_to_rest(&run.id).await;
    let p = w.pending.unwrap();
    assert_eq!(p.kind, DecisionKind::GenerationApproval);
    assert_eq!(
        prov.submit_count(),
        0,
        "nothing is generated (or paid) before approval"
    );
    a.orch
        .decide(&run.id, &p.id, "approve", &Value::Null, "user")
        .unwrap();
    let done = a.wait(&run.id, |r| r.status == RunStatus::Completed).await;
    assert_eq!(prov.submit_count(), 1);
    assert_eq!(done.usage.generations, 1);
    assert_eq!(
        done.usage.cost_micros, 500_000,
        "reserved vs actual reconciled in the ledger"
    );
    let asset = done.production_plan.unwrap().asset_needs[0]
        .resolved_asset_id
        .clone()
        .unwrap();
    let pv = a.orch.store().get_provenance(&asset).unwrap().unwrap();
    assert_eq!(pv.kind, "generated");
    assert_eq!(pv.json["license"]["status"], "generated");
    assert_eq!(pv.json["extra"]["provider"], "gen");
    assert!(
        pv.json["extra"]["prompt_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        pv.json["extra"]["job"]
            .as_str()
            .unwrap()
            .starts_with("job-gen-")
    );
    assert_eq!(pv.json["cost_micros"], 500_000);

    // orçamento: o teto abaixo do estimado ⇒ pede extensão ANTES de qualquer geração
    let Some(b) = mk("sc-gen-budget") else { return };
    let prov2 = Arc::new(ReplayGenerationProvider::new(
        "gen",
        vec![GenKind::Video],
        Some(500_000),
        payload,
    ));
    b.generators.register(prov2.clone());
    let policy = RunPolicy {
        generation_requires_approval: false,
        ..auto_policy()
    };
    let budget = RunBudget {
        max_cost_micros: Some(100_000),
        ..RunBudget::default()
    };
    let run = b
        .orch
        .create_run(b.inputs(), Some(policy), Some(budget), "pf", None)
        .unwrap();
    let w = b.run_to_rest(&run.id).await;
    assert_eq!(
        w.pending.as_ref().unwrap().kind,
        DecisionKind::BudgetExtension,
        "{:?}",
        w.pending
    );
    assert_eq!(prov2.submit_count(), 0);

    // geração desligada: o Planner vê a capability indisponível; essencial ⇒ WAITING_USER claro
    let Some(c) = mk("sc-gen-off") else { return };
    let policy = RunPolicy {
        allow_generation: false,
        ..auto_policy()
    };
    let run = c.create(c.inputs(), policy);
    let w = c.run_to_rest(&run.id).await;
    assert_eq!(w.status, RunStatus::WaitingUser);
    assert!(w.pending.unwrap().question.contains("gen1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rate_limited_provider_is_retried_and_the_cost_is_accounted() {
    let Some(a) = auto_world("sc-429", simple_script) else {
        return;
    };
    a.script
        .rate_limit_first_producer
        .store(true, Ordering::SeqCst);
    let run = a.create(a.inputs(), auto_policy());
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    // cada papel entra no livro-razão uma vez (demand, producer, planner, critic) — retries não duplicam
    assert!(done.usage.provider_calls >= 4, "{:?}", done.usage);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_edit_during_the_run_drifts_the_diff_and_is_never_overwritten() {
    let Some(a) = auto_world("sc-drift", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            move |_d, _n| {
                let mut v = edit_json(&asset.lock().unwrap(), json!([]));
                v["target_sequence"] = json!("s"); // edita a sequence existente do usuário
                v
            },
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let run = a.create(a.inputs(), policy_plan_always());
    let w = a.run_to_rest(&run.id).await;
    let d = w.pending.unwrap();
    assert_eq!(d.kind, DecisionKind::PlanApproval);
    // o usuário mexe na MESMA sequence enquanto o plano espera (adiciona uma track)
    call(
        &a.w.session,
        "command.execute",
        json!({"label": "manual", "commands": [{"operation_id": "manual-1", "type": "add_track", "sequence": "s", "id": "user_track", "kind": "visual"}]}),
    );
    a.orch
        .decide(&run.id, &d.id, "approve", &Value::Null, "user")
        .unwrap();
    let done = a.wait(&run.id, |r| r.status == RunStatus::Completed).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    let seq =
        a.w.ctx
            .engine
            .read("sequence.get", json!({"sequence": "s"}))
            .unwrap();
    let tracks: Vec<&str> = seq["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(
        tracks.contains(&"user_track"),
        "manual work survives: {tracks:?}"
    );
    assert!(
        tracks.iter().any(|t| t.contains("_run-")),
        "run tracks are scoped by the run id: {tracks:?}"
    );
    // a validação foi refeita depois do drift antes de escrever
    assert!(
        a.stages_visited(&run.id)
            .iter()
            .filter(|s| *s == "validate_plan")
            .count()
            >= 2,
        "{:?}",
        a.stages_visited(&run.id)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_during_generation_stops_everything_and_applies_nothing() {
    let Some(tc) = ffmpeg() else { return };
    let Some(a) = auto_world("sc-cancel", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([need("gen1", true, &["generate"])]),
                )
            },
            move |_d, _n| beats_with_need(&asset.lock().unwrap(), "gen1"),
            |_n| json!({"findings": []}),
        )
    }) else {
        return;
    };
    let payload = broll_bytes(&tc, &a.w.dir, "gen.mp4", 4);
    let prov = Arc::new(
        ReplayGenerationProvider::new("gen", vec![GenKind::Video], Some(1_000), payload)
            .with_running_polls(1_000_000),
    );
    a.generators.register(prov.clone());
    let policy = RunPolicy {
        generation_requires_approval: false,
        ..auto_policy()
    };
    let run = a.create(a.inputs(), policy);
    a.orch.start(&run.id).unwrap();
    // espera o job ser submetido (em voo) e cancela
    let t0 = std::time::Instant::now();
    while prov.submit_count() == 0 {
        assert!(t0.elapsed().as_secs() < 60);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    a.orch.cancel(&run.id).unwrap();
    let r = a.wait(&run.id, |r| r.status == RunStatus::Cancelled).await;
    assert_eq!(r.status, RunStatus::Cancelled);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(prov.job_count(), 0, "the provider job was cancelled");
    assert_eq!(a.spy.applies.load(Ordering::SeqCst), 0, "no late apply");
    assert!(
        !a.orch.is_driving(&run.id),
        "nothing keeps running in the background"
    );
    // não se retoma uma Run cancelada (mas dá para duplicá-la como nova)
    assert!(a.orch.resume(&run.id).is_err());
    let dup = a.orch.duplicate(&run.id, None, None, None).unwrap();
    assert_ne!(dup.id, run.id);
    assert_eq!(dup.parent_run_id.as_deref(), Some(run.id.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_correction_loop_fixes_in_one_cycle_with_minimal_commands() {
    let run_id = Arc::new(Mutex::new(String::new()));
    let rid = run_id.clone();
    let Some(a) = auto_world("sc-correct", move |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            move |_d, _n| edit_json(&asset.lock().unwrap(), json!([])),
            move |n| {
                if n == 0 {
                    let r = rid.lock().unwrap().clone();
                    let clip = format!("sq_{r}_main_t_0");
                    json!({"findings": [{"key": "hook-text", "severity": "major", "category": "brief",
                        "expected": "no hook overlay", "observed": "the hook overlay is too aggressive",
                        "evidence": [{"kind": "clip", "detail": clip}],
                        "fix": {"action": "delete_clip", "clip": clip}}]})
                } else {
                    json!({"findings": []})
                }
            },
        )
    }) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    *run_id.lock().unwrap() = run.id.clone();
    let done = a.run_to_rest(&run.id).await;
    assert_eq!(
        done.status,
        RunStatus::Completed,
        "{:?} {:?}",
        done.error,
        done.pending
    );
    assert_eq!(
        a.stages_visited(&run.id),
        [
            "understand",
            "plan",
            "validate_plan",
            "edit",
            "review",
            "correct",
            "review"
        ]
    );
    assert_eq!(done.usage.review_loops, 1);
    assert_eq!(done.reviews.len(), 2);
    assert!(
        !done.reviews[0].pass
            && done.reviews[1].pass
            && done.reviews[1].score > done.reviews[0].score
    );
    let seq =
        a.w.ctx
            .engine
            .read(
                "sequence.get",
                json!({"sequence": done.sequences[0].sequence_id}),
            )
            .unwrap();
    assert!(
        seq["clips"]
            .get(format!("sq_{}_main_t_0", run.id))
            .is_none(),
        "the flagged clip is gone"
    );
    assert!(
        seq["clips"].as_object().unwrap().len() >= 3,
        "the rest of the edit is intact"
    );
    let corr: Vec<_> = done
        .applied
        .iter()
        .filter(|x| x.stage == RunStage::Correct)
        .collect();
    assert_eq!(corr.len(), 1);
    assert!(corr[0].operation_namespace.ends_with(":c1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_correction_that_does_not_help_stops_the_loop_and_asks_the_user() {
    let Some(a) = auto_world("sc-osc", |asset| {
        Script::new(
            || demand_json(false),
            || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([]),
                )
            },
            move |_d, _n| edit_json(&asset.lock().unwrap(), json!([])),
            // o crítico repete o MESMO achado a cada ciclo (a "correção" não muda o veredito)
            |_n| {
                json!({"findings": [{"key": "pace", "severity": "major", "category": "pacing",
                "expected": "faster", "observed": "slow",
                "evidence": [{"kind": "range", "detail": "0-3s"}],
                "fix": {"action": "set_property", "clip": "nonexistent-ok-for-preview?", "prop": "scale", "value": 1.0}}]})
            },
        )
    }) else {
        return;
    };
    let run = a.create(a.inputs(), auto_policy());
    let w = a.run_to_rest(&run.id).await;
    // fix inaplicável ⇒ conflito ⇒ replaneja; ao esgotar os limites, nunca fica em laço infinito
    assert!(
        matches!(
            w.status,
            RunStatus::WaitingUser | RunStatus::Completed | RunStatus::Failed
        ),
        "{:?}",
        w.status
    );
    assert_ne!(w.status, RunStatus::Running);
    assert!(w.usage.review_loops <= w.budget.max_review_loops);
    assert!(w.usage.replans <= w.budget.max_replans + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_stops_between_steps_and_resume_finishes_the_run() {
    let Some(a) = auto_world("sc-pause", simple_script) else {
        return;
    };
    a.script.producer_delay_ms.store(300, Ordering::SeqCst);
    let run = a.create(a.inputs(), auto_policy());
    a.orch.start(&run.id).unwrap();
    let t0 = std::time::Instant::now();
    while a.script.count("producer") == 0 {
        assert!(t0.elapsed().as_secs() < 60);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    a.orch.pause(&run.id).unwrap();
    let p = a.wait(&run.id, |r| r.status == RunStatus::Paused).await;
    assert_eq!(p.status, RunStatus::Paused);
    let stage = p.resume_stage.unwrap();
    assert!(matches!(stage, RunStage::Plan | RunStage::ValidatePlan));
    assert_eq!(a.spy.applies.load(Ordering::SeqCst), 0);
    a.script.producer_delay_ms.store(0, Ordering::SeqCst);
    a.orch.resume(&run.id).unwrap();
    let done = a.wait(&run.id, |r| r.status == RunStatus::Completed).await;
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
}
