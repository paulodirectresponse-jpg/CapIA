//! Pipeline headless ponta a ponta pelo serviço `ai.*` (o mesmo caminho da UI/devserver/desktop):
//! projeto + mídia → `ai.run.create` → plano pede aprovação → `ai.run.decide` → timeline editável
//! → undo seletivo da Run → memória/gateway/IA desligada. Sem rede, sem chave (cérebro demo).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn wait_run(w: &ServiceWorld, id: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let t0 = Instant::now();
    loop {
        let r = w.ai("ai.run.get", json!({"run_id": id}));
        if pred(&r["run"]) && r["driving"] != true {
            return r;
        }
        assert!(t0.elapsed() < Duration::from_secs(60), "timeout: {r}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn a_run_goes_from_brief_to_an_editable_timeline_through_the_service() {
    let Some(w) = service_world("svc-run", true) else {
        return;
    };
    w.svc.install_demo_autonomy();
    let created = w.ai(
        "ai.run.create",
        json!({"inputs": {"brief_text": "Produto: Demo. Faça um anúncio curto.",
                           "assets": [w.asset_id],
                           "deliverables": [{"key": "main", "max_duration_s": 30}]},
               "policy": {"plan": "always", "demand_spec": "auto"}}),
    );
    let id = created["run"]["id"].as_str().unwrap().to_owned();
    // 1) o plano espera o humano; nada foi escrito
    let r = wait_run(&w, &id, |r| r["status"] == "waiting_user");
    assert_eq!(r["run"]["pending"]["kind"], "plan_approval", "{r}");
    let before = call(&w.session, "history.list", json!({}));
    assert!(
        !before.to_string().contains(&format!("run:{id}")),
        "no write before the plan is approved"
    );
    // 2) aprovar com o id da decisão certo
    let dec = r["run"]["pending"]["id"].as_str().unwrap();
    w.ai(
        "ai.run.decide",
        json!({"run_id": id, "decision_id": dec, "option": "approve"}),
    );
    let r = wait_run(&w, &id, |r| r["status"] == "completed");
    assert_eq!(r["run"]["status"], "completed", "{r}");
    // 3) a timeline é comum: sequence com clips, atribuída à Run
    let seq = r["run"]["sequences"][0]["sequence_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let s = call(&w.session, "sequence.get", json!({"sequence": seq}));
    assert!(!s["clips"].as_object().unwrap().is_empty());
    let hist = call(&w.session, "history.list", json!({}));
    assert!(hist.to_string().contains(&format!("run:{id}")));
    // 4) eventos reconectáveis: depois de `after` só vêm os novos
    let all = w.ai("ai.run.events", json!({"run_id": id, "after": 0}));
    let n = all["events"].as_array().unwrap().len();
    assert!(n > 5);
    let last = all["events"][n - 1]["seq"].as_u64().unwrap();
    let none = w.ai("ai.run.events", json!({"run_id": id, "after": last}));
    assert!(none["events"].as_array().unwrap().is_empty());
    // 5) lista e histórico de Runs
    let list = w.ai("ai.run.list", json!({}));
    assert_eq!(list["runs"].as_array().unwrap().len(), 1);
    // 6) undo seletivo desfaz só a Run
    let rep = call(
        &w.session,
        "history.undo_report",
        json!({"actor_id": format!("run:{id}")}),
    );
    assert!(rep["conflicts"].as_array().unwrap().is_empty(), "{rep}");
    call(
        &w.session,
        "history.undo_selective",
        json!({"actor_id": format!("run:{id}")}),
    );
    assert!(w.svc.call("ai.run.get", json!({"run_id": id})).is_ok());
}

#[test]
fn runs_are_refused_with_ai_off_while_the_editor_keeps_working() {
    let Some(w) = service_world("svc-aioff", true) else {
        return;
    };
    w.svc.install_demo_autonomy();
    w.ai("ai.enabled.set", json!({"enabled": false}));
    let e = w.ai_err("ai.run.create", json!({"inputs": {"assets": [w.asset_id]}}));
    assert!(
        e.code.contains("AI_OFF") || e.code.contains("DISABLED"),
        "{e:?}"
    );
    // o editor segue editando
    call(
        &w.session,
        "command.execute",
        json!({"label": "x", "commands": [{"operation_id": "z1", "type": "add_track", "sequence": "s", "id": "t2", "kind": "visual"}]}),
    );
    // listar/consultar Runs antigas continua funcionando
    assert!(w.svc.call("ai.run.list", json!({})).is_ok());
}

#[test]
fn memory_and_gateway_endpoints_are_safe_by_default() {
    let Some(w) = service_world("svc-mem", true) else {
        return;
    };
    w.svc.install_demo_autonomy();
    // proposta de usuário nasce proposta; só `approve` ativa
    let r = w.ai(
        "ai.memory.add",
        json!({"scope": "user", "content": "prefiro CTAs curtos", "kind": "preference"}),
    );
    let id = r["item"]["id"].as_str().unwrap().to_owned();
    // `ai.memory.add` é uma ação explícita do usuário na UI (fonte `user_declared`)
    assert_eq!(r["item"]["source"], "user_declared", "{r}");
    let a = w.ai("ai.memory.approve", json!({"id": id}));
    assert_eq!(a["item"]["status"], "active");
    let st = w.ai("ai.gateway.status", json!({}));
    assert_eq!(
        st["generation"]["enabled"], false,
        "generation is off until opted in: {st}"
    );
    assert!(
        st["adapters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["paid"] != true || a["enabled"] == false)
    );
}
