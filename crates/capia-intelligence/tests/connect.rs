//! "Conectar IA": uma operação leva provedor + chave a modelo escolhido, probe por capability e
//! Brain Profile ativo — e erros de chave não deixam estado pela metade nem vazam a chave.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_ai::testkit::{MockRequest, MockResponse, MockServer};
use common::*;
use serde_json::{Value, json};

const KEY: &str = "KEY-connect-0123456789abcdef";

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn chunks(events: Vec<Value>) -> MockResponse {
    let mut ev: Vec<(Option<String>, String)> =
        events.into_iter().map(|v| (None, v.to_string())).collect();
    ev.push((None, "[DONE]".into()));
    MockResponse::Sse {
        events: ev,
        delay_ms: 0,
        hang: false,
    }
}

fn openai_like(r: &MockRequest) -> MockResponse {
    if r.method == "GET" {
        return MockResponse::Json(
            200,
            json!({"data": [{"id": "gpt-4o-mini"}, {"id": "gpt-5"}, {"id": "whisper-1"},
                            {"id": "text-embedding-3-small"}]})
            .to_string(),
        );
    }
    if r.header("authorization") != Some(&*format!("Bearer {KEY}")) {
        return MockResponse::Json(
            401,
            json!({"error": {"message": "Incorrect API key provided"}}).to_string(),
        );
    }
    let b = r.json();
    let text = |s: &str| {
        chunks(vec![
            json!({"choices": [{"delta": {"content": s}}]}),
            json!({"choices": [{"delta": {"content": " "}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
        ])
    };
    if String::from_utf8_lossy(&r.body).contains("image_url") {
        return text("red");
    }
    if b.get("response_format").is_some() {
        return chunks(vec![
            json!({"choices": [{"delta": {"content": "{\"ok\":true,\"n\":7}"}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
        ]);
    }
    if b.get("tools").is_some() {
        return chunks(vec![
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "c1",
                "function": {"name": "echo", "arguments": "{\"message\":\"hi\"}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
        ]);
    }
    chunks(vec![
        json!({"choices": [{"delta": {"content": "1 2 3 "}}]}),
        json!({"choices": [{"delta": {"content": "4 5 6 7 8"}}]}),
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
    ])
}

#[test]
fn connect_picks_a_model_probes_it_and_activates_an_automatic_brain_profile() {
    let r = rt();
    let srv = r.block_on(MockServer::start(openai_like));
    let Some(w) = service_world("svc-connect", false) else {
        return;
    };
    assert!(w.ai("ai.status", json!({}))["active_profile"].is_null());
    let t = w.ai(
        "ai.connect",
        json!({"preset": "openai", "api_key": KEY, "base_url": srv.url(), "allow_loopback": true}),
    );
    let ev = w.wait_task(t["task_id"].as_str().unwrap(), &["done", "error"]);
    let last = ev.last().unwrap();
    assert_eq!(last["phase"], "done", "{last}");
    let res = &last["data"]["result"];
    assert_eq!(res["model"], "gpt-5");
    assert_eq!(res["probe"]["status"], "ready", "{res}");
    assert_eq!(res["profile_created"], true);
    let st = w.ai("ai.status", json!({}));
    assert_eq!(
        st["active_profile"], "auto",
        "active_profile nunca fica nulo"
    );
    assert!(st["any_usable_model"].as_bool().unwrap());
    // a chave nunca volta
    assert!(!st.to_string().contains(KEY));
    assert!(!last.to_string().contains(KEY));
}

#[test]
fn a_wrong_key_fails_with_an_auth_error_and_no_profile() {
    let r = rt();
    let srv = r.block_on(MockServer::start(|req: &MockRequest| {
        if req.method == "GET" {
            MockResponse::Json(
                401,
                json!({"error": {"message": "Incorrect API key provided: sk-WRONG"}}).to_string(),
            )
        } else {
            openai_like(req)
        }
    }));
    let Some(w) = service_world("svc-connect-bad", false) else {
        return;
    };
    let t = w.ai(
        "ai.connect",
        json!({"preset": "openai", "api_key": "sk-WRONG-0123456789abcdef",
               "base_url": srv.url(), "allow_loopback": true}),
    );
    let ev = w.wait_task(t["task_id"].as_str().unwrap(), &["done", "error"]);
    let last = ev.last().unwrap();
    assert_eq!(last["phase"], "error", "{last}");
    assert_eq!(last["data"]["code"], "AUTH_FAILED", "{last}");
    assert!(!last.to_string().contains("sk-WRONG-0123456789abcdef"));
    assert!(w.ai("ai.status", json!({}))["active_profile"].is_null());
}
