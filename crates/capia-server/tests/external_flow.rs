//! Fluxo externo canônico (REST e MCP): cliente → projeto → raw + referência + briefing → Run com
//! aprovações → variantes → export → webhooks assinados → estado relido. REST e MCP têm que chegar
//! ao MESMO estado autoritativo, o mesmo que a UI (Session/Engine API) enxerga.
#![allow(
    unreachable_pub,
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

mod common;
#[path = "common/flow.rs"]
mod flow;
#[path = "common/receiver.rs"]
mod receiver;

use common::*;
use flow::*;
use receiver::Receiver;
use serde_json::{Value, json};
use std::time::Duration;

fn demo(c: &mut capia_server::config::ServerConfig) {
    c.demo_brain = true;
}

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn json_of(r: capia_editor_api::Reply) -> Value {
    match r {
        capia_editor_api::Reply::Json(v) => v,
        capia_editor_api::Reply::Binary { .. } => panic!("unexpected binary reply"),
    }
}

/// Confere, do lado do receptor, tudo o que o fluxo prometeu entregar.
fn check_webhooks(rx: &Receiver, f: &FlowResult, s: &TestServer) {
    let got = rx.wait_for(3, Duration::from_secs(30));
    assert_eq!(got.len(), 3, "run.completed x2 + export.completed expected: {got:?}");
    let mut runs = Vec::new();
    let mut exports = 0;
    for r in &got {
        assert!(r.signature_ok_independent(&f.webhook_secret), "bad signature");
        assert_eq!(r.verify(&f.webhook_secret, now_s()), Ok(()));
        let b = r.json();
        assert_eq!(b["version"], 1);
        assert_eq!(b["project_id"], f.project_id);
        assert_eq!(b["id"], r.event_id());
        match b["type"].as_str().unwrap() {
            "run.completed" => {
                assert_eq!(b["run_id"], b["data"]["run"]["id"]);
                assert_eq!(b["data"]["run"]["status"], "completed");
                runs.push(b["run_id"].as_str().unwrap().to_owned());
            }
            "export.completed" => {
                exports += 1;
                assert!(b["export_id"].is_string());
                assert_eq!(b["data"]["state"], "completed");
                assert_eq!(b["data"]["export_id"], b["export_id"]);
            }
            other => panic!("unexpected event {other}"),
        }
    }
    runs.sort();
    let mut want = vec![f.master_run.clone(), f.variant_run.clone()];
    want.sort();
    assert_eq!(runs, want);
    assert_eq!(exports, 1);
    // nenhum evento repetido (mesmo id nunca chega duas vezes se tudo deu certo na 1ª tentativa)
    let mut ids: Vec<&str> = got.iter().map(receiver::Request::event_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    // log de entrega: tudo entregue
    let hook = f.webhook_id.as_str();
    let d = poll("delivery log", Duration::from_secs(10), || {
        let r = s.call("GET", &format!("/v1/webhooks/{hook}/deliveries"), None).json();
        let ds = r["deliveries"].as_array().unwrap().clone();
        (ds.len() == 3 && ds.iter().all(|d| d["state"] == "delivered")).then_some(ds)
    });
    assert!(d.iter().all(|x| x["attempt"] == 1));
}

/// A mesma sequence que o cliente externo leu é a que o Engine/Session (a UI) enxerga, em memória
/// e depois de reabrir o arquivo do projeto.
fn check_editable_state_identical(s: &mut TestServer, f: &FlowResult) {
    for (id, body) in &f.sequences {
        let live = json_of(s.core().lock_session().call("sequence.get", json!({"sequence": id})).unwrap());
        assert_eq!(&live, &body["sequence"], "live Engine API differs from the API body for {id}");
    }
    let path = s.core().db.project_get(&f.project_id).unwrap().unwrap().path;
    s.shutdown();
    let mut fresh = capia_editor_api::Session::new(capia_editor_api::SessionConfig::default());
    fresh.call("project.open", json!({"path": path})).unwrap();
    for (id, body) in &f.sequences {
        let reopened = json_of(fresh.call("sequence.get", json!({"sequence": id})).unwrap());
        assert_eq!(&reopened, &body["sequence"], "reopened project differs for {id}");
    }
    let snap = json_of(fresh.call("project.snapshot", json!({})).unwrap());
    assert_eq!(snap["sequences"].as_array().unwrap().len(), f.sequences.len());
    // a história guarda o ator da Run (agente), editável/desfazível como qualquer outra
    let hist = json_of(fresh.call("history.list", json!({})).unwrap());
    assert!(hist["entries"].as_array().unwrap().iter().any(|e| e["actor"]["id"].as_str().is_some_and(|a| a.starts_with("run:"))));
    let _ = fresh.call("project.close", json!({}));
}

#[test]
fn canonical_flow_over_rest_and_mcp_reaches_the_same_authoritative_state() {
    if !ffmpeg_or_skip("canonical_flow") {
        return;
    }
    // ---- REST
    let mut s_rest = start("flow-rest", demo);
    let rx_rest = Receiver::start();
    let rest = RestClient { s: &s_rest, token: s_rest.admin.clone() };
    let f_rest = run_flow(&rest, &s_rest, &rx_rest);
    check_webhooks(&rx_rest, &f_rest, &s_rest);
    // ---- MCP (outro diretório de dados, mesmo roteiro)
    let mut s_mcp = start("flow-mcp", demo);
    let rx_mcp = Receiver::start();
    let mcp = McpClient { s: &s_mcp, token: s_mcp.admin.clone() };
    let f_mcp = run_flow(&mcp, &s_mcp, &rx_mcp);
    check_webhooks(&rx_mcp, &f_mcp, &s_mcp);
    // ---- paridade semântica (revisão, sequences, grafo de clips, variantes, histórico, export)
    assert_eq!(
        f_rest.summary, f_mcp.summary,
        "REST and MCP produced different authoritative state"
    );
    let sum = &f_rest.summary;
    assert_eq!(sum["variant_sequences"], 2);
    assert_eq!(sum["sequences"].as_array().unwrap().len(), 3);
    assert!(sum["review_loops_ge_1"].as_bool().unwrap());
    assert!(sum["run_actor_in_history"].as_bool().unwrap());
    assert_eq!(sum["export"]["state"], "completed");
    assert_eq!(sum["webhook_types"], json!(["export.completed", "run.completed", "run.completed"]));
    // ---- o que a UI veria: Engine API ao vivo e arquivo reaberto
    check_editable_state_identical(&mut s_rest, &f_rest);
    check_editable_state_identical(&mut s_mcp, &f_mcp);
}

#[test]
fn a_dead_webhook_endpoint_never_blocks_run_completion_and_recovers_later() {
    if !ffmpeg_or_skip("dead_webhook") {
        return;
    }
    let s = start("flow-dead", |c| {
        demo(c);
        c.webhook_max_attempts = 500;
    });
    // porta que ninguém escuta
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}/hook", l.local_addr().unwrap().port())
    };
    let rc = RestClient { s: &s, token: s.admin.clone() };
    let c: &dyn Client = &rc;
    let t0 = std::time::Instant::now();
    let (pid, run) = quick_run(c, Some((&dead, &["run.completed", "run.started"])));
    // a Run terminou normalmente apesar do endpoint morto
    let v = c.ok("runs.get", json!({"project_id": pid, "run_id": run}));
    assert_eq!(v["run"]["status"], "completed");
    assert!(t0.elapsed() < Duration::from_secs(120));
    let hooks = c.ok("webhooks.list", json!({}));
    let hook = hooks["webhooks"][0]["id"].as_str().unwrap().to_owned();
    let ds = poll("failing deliveries", Duration::from_secs(20), || {
        let ds = c.ok("webhooks.deliveries", json!({"webhook_id": hook}))["deliveries"].as_array().unwrap().clone();
        (ds.len() >= 2 && ds.iter().all(|d| d["state"] == "retrying" && d["attempt"].as_u64().unwrap() >= 1)).then_some(ds)
    });
    assert!(ds.iter().all(|d| d["last_error"].as_str().unwrap().contains("connect")), "{ds:?}");
    // o servidor segue saudável e aceitando escritas
    assert!(c.call("projects.list", json!({})).is_ok());
    // o endpoint volta (URL corrigida): as entregas pendentes chegam assinadas, at-least-once
    let rx = Receiver::start();
    c.ok("webhooks.update", json!({"webhook_id": hook, "url": rx.url()}));
    let got = rx.wait_for(2, Duration::from_secs(20));
    assert!(got.len() >= 2);
    let types: std::collections::BTreeSet<String> = got.iter().map(|r| r.json()["type"].as_str().unwrap().to_owned()).collect();
    assert!(types.contains("run.completed"));
    poll("all delivered", Duration::from_secs(20), || {
        let ds = c.ok("webhooks.deliveries", json!({"webhook_id": hook}))["deliveries"].as_array().unwrap().clone();
        ds.iter().all(|d| d["state"] == "delivered").then_some(())
    });
}
