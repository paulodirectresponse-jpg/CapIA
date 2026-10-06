//! OpenAI nativo (`/v1/responses`) contra um servidor falso que fala o protocolo real:
//! streaming de texto + uso, tool call, erro de autenticação e cancelamento.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_ai::CancelToken;
use capia_ai::error::ErrorCode;
use capia_ai::providers::{CallCtx, build_provider, collect};
use capia_ai::registry::{ProviderConfig, ProviderKind};
use capia_ai::testkit::{MockResponse, MockServer};
use capia_ai::types::{ChatRequest, FinishReason, Message, ToolSpec};
use capia_secrets::{CredentialRef, MemoryStore, SecretStore, SecretString};
use serde_json::{Value, json};
use std::sync::Arc;

const KEY: &str = "KEY-responses-0123456789abcdef";

fn sse(events: Vec<Value>) -> MockResponse {
    MockResponse::Sse {
        events: events
            .into_iter()
            .map(|v| (v["type"].as_str().map(str::to_owned), v.to_string()))
            .collect(),
        delay_ms: 0,
        hang: false,
    }
}

async fn rig(
    f: impl Fn(&capia_ai::testkit::MockRequest) -> MockResponse + Send + Sync + 'static,
) -> (Arc<dyn capia_ai::providers::ModelProvider>, MockServer) {
    let server = MockServer::start(f).await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let mut c = ProviderConfig::new("oa", ProviderKind::OpenAiCompatible, "oa");
    c.base_url = Some(server.url());
    c.api_style = Some("responses".into());
    c.allow_loopback = true;
    c.enabled = true;
    let cred = CredentialRef::for_provider("oa").unwrap();
    store.put(&cred, SecretString::new(KEY)).unwrap();
    c.credential_ref = Some(cred.as_str().to_owned());
    c.bound_host = Some("127.0.0.1".into());
    (build_provider(&c, store).unwrap(), server)
}

#[tokio::test]
async fn streams_text_and_usage_through_the_responses_endpoint() {
    let (p, server) = rig(|r| {
        assert!(r.path.ends_with("/responses"), "path {}", r.path);
        assert_eq!(r.header("authorization"), Some(&*format!("Bearer {KEY}")));
        assert_eq!(r.json()["stream"], true);
        sse(vec![
            json!({"type": "response.output_text.delta", "delta": "Olá, "}),
            json!({"type": "response.output_text.delta", "delta": "mundo"}),
            json!({"type": "response.completed", "response": {"status": "completed",
                "usage": {"input_tokens": 11, "output_tokens": 4}}}),
        ])
    })
    .await;
    let cancel = CancelToken::new();
    let stream = p
        .chat(
            &ChatRequest::new("gpt-x", vec![Message::user("oi")]),
            &CallCtx::new(cancel.clone()),
        )
        .await
        .unwrap();
    let r = collect(stream, &cancel).await.unwrap();
    assert_eq!(r.text, "Olá, mundo");
    assert_eq!((r.usage.input_tokens, r.usage.output_tokens), (11, 4));
    assert_eq!(r.finish, Some(FinishReason::Stop));
    drop(server);
}

#[tokio::test]
async fn streamed_function_call_is_assembled() {
    let (p, _s) = rig(|_| {
        sse(vec![
            json!({"type": "response.output_item.added", "output_index": 0,
                   "item": {"type": "function_call", "call_id": "call_1", "name": "trim_clip"}}),
            json!({"type": "response.function_call_arguments.delta", "output_index": 0,
                   "delta": "{\"seconds\":"}),
            json!({"type": "response.function_call_arguments.delta", "output_index": 0,
                   "delta": "2}"}),
            json!({"type": "response.completed", "response": {"status": "completed"}}),
        ])
    })
    .await;
    let mut req = ChatRequest::new("gpt-x", vec![Message::user("corte")]);
    req.tools = vec![ToolSpec {
        name: "trim_clip".into(),
        description: "d".into(),
        input_schema: json!({"type": "object"}),
    }];
    let cancel = CancelToken::new();
    let r = collect(
        p.chat(&req, &CallCtx::new(cancel.clone())).await.unwrap(),
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(r.tool_calls.len(), 1);
    let call = &r.tool_calls[0];
    assert_eq!(call.id, "call_1");
    assert_eq!(call.name, "trim_clip");
    assert_eq!(call.arguments, json!({"seconds": 2}));
    assert_eq!(r.finish, Some(FinishReason::ToolCalls));
}

#[tokio::test]
async fn auth_failure_is_classified_and_never_leaks_the_key() {
    let (p, _s) = rig(|_| {
        MockResponse::Json(
            401,
            json!({"error": {"message": format!("Incorrect API key provided: {KEY}")}}).to_string(),
        )
    })
    .await;
    let err = match p
        .chat(
            &ChatRequest::new("gpt-x", vec![Message::user("oi")]),
            &CallCtx::default(),
        )
        .await
    {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    };
    assert_eq!(err.code, ErrorCode::AuthFailed);
    assert!(!format!("{err:?} {err}").contains(KEY));
}
