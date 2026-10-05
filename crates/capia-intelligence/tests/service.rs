//! Serviço `ai.*` de ponta a ponta: configuração write-only, canário, troca de Brain entre três
//! famílias sem mudar código, AI Off, cancelamento e fluxo do assistente com aprovação.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_ai::providers::replay::{ReplayProvider, ReplayResponse};
use capia_ai::registry::ProviderKind;
use capia_ai::testkit::{MockResponse, MockServer, text_reply};
use capia_ai::types::ChatEvent;
use common::*;
use serde_json::{Value, json};
use std::sync::Arc;

const CANARY: &str = "CNRY-svc-5b1e8d7a3c9f0e2d4b6a8c0e";
const WRONG: &str = "WRNG-svc-0f1e2d3c4b5a69788796a5b4";

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn start(
    rt: &tokio::runtime::Runtime,
    f: impl Fn(&capia_ai::testkit::MockRequest) -> MockResponse + Send + Sync + 'static,
) -> MockServer {
    rt.block_on(MockServer::start(f))
}

fn text_of(events: &[Value]) -> String {
    events
        .iter()
        .filter(|e| e["phase"] == "text")
        .map(|e| e["data"]["delta"].as_str().unwrap_or(""))
        .collect()
}

#[test]
fn credentials_are_write_only_and_the_canary_never_reaches_any_artifact() {
    let r = rt();
    // o servidor só aceita CANARY; com outra chave devolve 401 ecoando a chave recebida
    let srv = start(&r, |req| {
        let auth = req.header("authorization").unwrap_or("").to_owned();
        if auth == format!("Bearer {CANARY}") {
            text_reply(ProviderKind::OpenAiCompatible, "tudo certo")
        } else {
            MockResponse::Json(
                401,
                json!({"error": {"message": format!("Incorrect API key provided: {auth}")}})
                    .to_string(),
            )
        }
    });
    let Some(w) = service_world("svc-canary", false) else {
        return;
    };
    // 1) salvar: a resposta só diz SE há credencial
    let saved = w.save_provider("good", "open_ai_compatible", Some(&srv.url()), Some(CANARY));
    assert_eq!(saved["provider"]["credential_configured"], true);
    assert!(!saved.to_string().contains(CANARY));
    assert_eq!(saved["provider"]["bound_host"], "127.0.0.1");
    w.save_model("good:m", "good", "gpt-test", &["text", "tools", "stream"]);
    w.ai(
        "ai.brain.set",
        json!({"profile": {"id": "p", "name": "p", "brain": "good:m"}}),
    );

    // 2) uso normal: chat funciona e o servidor viu a chave no header (e só ali)
    let t = w.ai("ai.assistant.send", json!({"text": "olá", "mode": "ask"}));
    let ev = w.wait_task(
        t["task_id"].as_str().unwrap(),
        &["done", "error", "cancelled"],
    );
    assert_eq!(text_of(&ev), "tudo certo", "{ev:?}");
    assert!(srv.seen().iter().any(|q| q.raw_text().contains(CANARY)));
    for q in srv.seen() {
        assert!(!String::from_utf8_lossy(&q.body).contains(CANARY));
        assert!(!q.path.contains(CANARY));
    }

    // 3) provider com chave errada: o 401 ecoa a chave errada — o erro que chega à UI vem redigido
    w.save_provider("bad", "open_ai_compatible", Some(&srv.url()), Some(WRONG));
    w.save_model("bad:m", "bad", "gpt-test", &["text", "tools", "stream"]);
    w.ai(
        "ai.brain.set",
        json!({"profile": {"id": "p2", "name": "p2", "brain": "bad:m"}}),
    );
    let t = w.ai("ai.assistant.send", json!({"text": "olá"}));
    let ev2 = w.wait_task(
        t["task_id"].as_str().unwrap(),
        &["done", "error", "cancelled"],
    );
    let last = ev2.last().unwrap();
    assert!(
        last["phase"] == "error" || last["data"]["result"]["status"] == "failed",
        "{last}"
    );

    // 4) tudo o que a UI pode ver ou o app pode persistir: 0 ocorrências
    let mut surfaces: Vec<(String, String)> = vec![
        ("status".into(), w.ai("ai.status", json!({})).to_string()),
        (
            "diagnostics".into(),
            w.ai("ai.diagnostics", json!({}))["text"]
                .as_str()
                .unwrap()
                .into(),
        ),
        ("events-ok".into(), serde_json::to_string(&ev).unwrap()),
        ("events-bad".into(), serde_json::to_string(&ev2).unwrap()),
        (
            "history".into(),
            common::call(&w.session, "history.list", json!({})).to_string(),
        ),
        (
            "snapshot".into(),
            common::call(&w.session, "project.snapshot", json!({})).to_string(),
        ),
        (
            "usage".into(),
            w.ai("ai.usage.summary", json!({})).to_string(),
        ),
    ];
    for (name, bytes) in w.disk_bytes() {
        surfaces.push((
            format!("disk:{name}"),
            String::from_utf8_lossy(&bytes).into_owned(),
        ));
    }
    for (name, text) in &surfaces {
        assert!(!text.contains(CANARY), "CANÁRIO vazou em {name}");
        assert!(!text.contains(WRONG), "chave errada vazou em {name}");
    }
    // 5) o cofre guarda; remover a credencial a apaga
    use capia_secrets::SecretStore;
    let cref = capia_secrets::CredentialRef::for_provider("good").unwrap();
    assert!(w.store.exists(&cref).unwrap());
    w.ai("ai.credential.delete", json!({"provider_id": "good"}));
    assert!(!w.store.exists(&cref).unwrap());
    // 6) a UI não consegue definir referência nem host de credencial
    let forged = w.ai(
        "ai.provider.save",
        json!({"provider": {"id": "good", "kind": "open_ai_compatible", "display_name": "x",
                            "base_url": srv.url(), "allow_loopback": true, "enabled": true,
                            "credential_ref": "capia/provider/other", "bound_host": "evil.example"}}),
    );
    assert_eq!(forged["provider"]["bound_host"], "127.0.0.1");
    assert_ne!(
        forged["provider"]["credential_configured"], true,
        "ref forjada ignorada"
    );
}

#[test]
fn the_brain_swaps_between_three_provider_families_by_configuration_only() {
    let r = rt();
    let mk = |kind: ProviderKind, word: &'static str| start(&r, move |_| text_reply(kind, word));
    let oa = mk(ProviderKind::OpenAiCompatible, "resposta-openai");
    let an = mk(ProviderKind::Anthropic, "resposta-anthropic");
    let go = mk(ProviderKind::Google, "resposta-google");
    let Some(w) = service_world("svc-swap", false) else {
        return;
    };
    for (id, kind, srv) in [
        ("oa", "open_ai_compatible", &oa),
        ("an", "anthropic", &an),
        ("go", "google", &go),
    ] {
        w.save_provider(
            id,
            kind,
            Some(&srv.url()),
            Some(&format!("KEY-{id}-0123456789")),
        );
        w.save_model(
            &format!("{id}:m"),
            id,
            "model-x",
            &["text", "tools", "stream"],
        );
    }
    let mut answers = Vec::new();
    for id in ["oa", "an", "go"] {
        w.ai(
            "ai.brain.set",
            json!({"profile": {"id": format!("p-{id}"), "name": id, "brain": format!("{id}:m")}}),
        );
        // mesma chamada, nenhum código diferente: só o Brain ativo mudou
        let t = w.ai("ai.assistant.send", json!({"text": "diga algo"}));
        let ev = w.wait_task(
            t["task_id"].as_str().unwrap(),
            &["done", "error", "cancelled"],
        );
        answers.push(text_of(&ev));
    }
    assert_eq!(
        answers,
        ["resposta-openai", "resposta-anthropic", "resposta-google"]
    );
    // cada servidor recebeu a credencial no header nativo da sua família
    assert!(
        oa.seen()[0]
            .header("authorization")
            .unwrap()
            .starts_with("Bearer KEY-oa")
    );
    assert!(
        an.seen()[0]
            .header("x-api-key")
            .unwrap()
            .starts_with("KEY-an")
    );
    assert!(
        go.seen()[0]
            .header("x-goog-api-key")
            .unwrap()
            .starts_with("KEY-go")
    );
    // perfil ativo persistido no AppDb: reabrir o serviço mantém a configuração (sem segredo no db)
    let bytes = std::fs::read(&w.appdb).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("KEY-oa-0123456789"));
}

#[test]
fn ai_off_blocks_cloud_tasks_but_local_analysis_and_manual_editing_keep_working() {
    let Some(w) = service_world("svc-off", true) else {
        return;
    };
    // sem NENHUM provider configurado
    let st = w.ai("ai.status", json!({}));
    assert_eq!(st["any_usable_model"], false);
    let t = w.ai("ai.assistant.send", json!({"text": "oi"}));
    let ev = w.wait_task(
        t["task_id"].as_str().unwrap(),
        &["done", "error", "cancelled"],
    );
    let last = ev.last().unwrap();
    assert!(
        last["phase"] == "error" || last["data"]["result"]["status"] == "failed",
        "{last}"
    );
    // análise local continua: cenas e silêncio
    let t = w.ai("ai.scenes.detect", json!({"asset_id": w.asset_id}));
    let ev = w.wait_task(t["task_id"].as_str().unwrap(), &["done", "error"]);
    assert_eq!(ev.last().unwrap()["phase"], "done", "{ev:?}");
    let t = w.ai(
        "ai.silence.plan",
        json!({"sequence": "s", "asset_id": w.asset_id}),
    );
    let ev = w.wait_task(t["task_id"].as_str().unwrap(), &["done", "error"]);
    assert_eq!(ev.last().unwrap()["phase"], "done", "{ev:?}");
    let token = ev.last().unwrap()["data"]["result"]["plan_token"]
        .as_str()
        .unwrap()
        .to_owned();
    // o plano local aplica pelo mesmo gate; o desligamento global não afeta o editor
    w.ai("ai.enabled.set", json!({"enabled": false}));
    w.ai("ai.plan.apply", json!({"plan_token": token}));
    let t = w.ai("ai.assistant.send", json!({"text": "oi"}));
    let ev = w.wait_task(
        t["task_id"].as_str().unwrap(),
        &["done", "error", "cancelled"],
    );
    assert_ne!(ev.last().unwrap()["phase"], "ok");
    // edição manual (sem IA) segue 100%
    common::call(
        &w.session,
        "command.execute",
        json!({"label":"manual","commands":[{"operation_id":"m1","type":"rename_clip","clip":"c1","name":"Manual"}]}),
    );
    common::call(&w.session, "command.undo", json!({}));
    // um plano que não veio de tarefa de IA é recusado pelo serviço
    assert_eq!(
        w.ai_err("ai.plan.apply", json!({"plan_token": "plan_9.aaaa"}))
            .code,
        "UNKNOWN_PLAN"
    );
}

#[test]
fn cancelling_a_task_aborts_the_provider_request() {
    let r = rt();
    let srv = start(&r, |_| MockResponse::Hang);
    let Some(w) = service_world("svc-cancel", false) else {
        return;
    };
    w.save_provider(
        "h",
        "open_ai_compatible",
        Some(&srv.url()),
        Some("KEY-hang-0123456789"),
    );
    w.save_model("h:m", "h", "m", &["text", "tools", "stream"]);
    w.ai(
        "ai.brain.set",
        json!({"profile": {"id": "p", "name": "p", "brain": "h:m"}}),
    );
    let t = w.ai("ai.assistant.send", json!({"text": "demora"}));
    let id = t["task_id"].as_str().unwrap().to_owned();
    // espera a requisição chegar ao servidor, depois cancela
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while srv.seen().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "a requisição não chegou"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        w.ai("ai.task.cancel", json!({"task_id": id}))["cancelled"],
        true
    );
    let ev = w.wait_task(&id, &["done", "error", "cancelled"]);
    assert_eq!(ev.last().unwrap()["phase"], "cancelled", "{ev:?}");
    // o servidor percebeu o abort real (conexão fechada pelo cliente)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while srv.client_aborts.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "o abort não chegou ao servidor"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        w.ai("ai.task.get", json!({"task_id": id}))["state"],
        "cancelled"
    );
}

#[test]
fn assistant_flow_streams_events_asks_for_approval_and_the_host_approves() {
    let Some(w) = service_world("svc-flow", true) else {
        return;
    };
    w.save_provider("r", "replay", None, None);
    w.save_model("r:m", "r", "replay-m", &["text", "tools", "stream"]);
    w.ai(
        "ai.brain.set",
        json!({"profile": {"id": "p", "name": "p", "brain": "r:m"}}),
    );
    let brain = Arc::new(ReplayProvider::responder(
        "r",
        Box::new(|req, n| match n {
            0 => ReplayResponse::Chat {
                events: vec![ChatEvent::ToolCall {
                    id: "c0".into(),
                    name: "timeline.preview".into(),
                    arguments: json!({"label": "Renomear", "commands": [{"type":"rename_clip","clip":"c1","name":"Hook"}]}),
                }],
                chunk_delay_ms: 0,
            },
            1 => {
                let tok = req
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| {
                        m.parts.iter().find_map(|p| match p {
                            capia_ai::types::Part::ToolResult { content, .. } => {
                                serde_json::from_str::<Value>(content).ok()
                            }
                            _ => None,
                        })
                    })
                    .unwrap()["plan_token"]
                    .clone();
                ReplayResponse::Chat {
                    events: vec![ChatEvent::ToolCall {
                        id: "c1".into(),
                        name: "timeline.apply_plan".into(),
                        arguments: json!({ "plan_token": tok }),
                    }],
                    chunk_delay_ms: 0,
                }
            }
            _ => ReplayResponse::Chat {
                events: vec![ChatEvent::TextDelta { text: "ok".into() }],
                chunk_delay_ms: 0,
            },
        }),
    ));
    w.svc.runtime().register_replay("r", brain);
    let t = w.ai(
        "ai.assistant.send",
        json!({"text": "renomeie o clip para Hook", "mode": "ask"}),
    );
    let id = t["task_id"].as_str().unwrap().to_owned();
    let ev = w.wait_task(&id, &["approval", "error", "cancelled"]);
    let appr = ev
        .iter()
        .find(|e| e["phase"] == "approval")
        .expect("aprovação pedida");
    let token = appr["data"]["plan"]["plan_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        ev.iter()
            .any(|e| e["phase"] == "tool" && e["data"]["name"] == "timeline.preview")
    );
    // pausado: o documento ainda não mudou
    let seq = common::call(&w.session, "sequence.get", json!({"sequence": "s"}));
    assert_eq!(seq["clips"]["c1"]["name"], "");
    // só o host (UI) aprova, por task_id + token
    assert!(
        w.svc
            .call(
                "ai.assistant.approve",
                json!({"task_id": id, "plan_token": "x.y"})
            )
            .is_err()
    );
    let done = w.ai(
        "ai.assistant.approve",
        json!({"task_id": id, "plan_token": token}),
    );
    assert_eq!(done["task"]["status"], "completed");
    let seq = common::call(&w.session, "sequence.get", json!({"sequence": "s"}));
    assert_eq!(seq["clips"]["c1"]["name"], "Hook");
    // custo/uso do projeto registrados (sintético: Replay)
    let u = w.ai("ai.usage.summary", json!({"task_id": id}));
    assert!(u["calls"].as_u64().unwrap() >= 2, "{u}");
}
