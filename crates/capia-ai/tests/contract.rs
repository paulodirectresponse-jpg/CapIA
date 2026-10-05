//! Suíte de contrato: **os mesmos casos** contra os três adapters reais (OpenAI-compatível, Anthropic,
//! Google) por meio de um servidor HTTP falso que fala o protocolo de cada fabricante. O código que
//! consome o provider é o mesmo — só o adapter muda (critério: trocar o Brain sem mudar código).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_ai::CancelToken;
use capia_ai::capability::Capability;
use capia_ai::error::ErrorCode;
use capia_ai::probe::{ProbeOptions, probe};
use capia_ai::providers::{CallCtx, ModelProvider, build_provider, collect};
use capia_ai::registry::{ProviderConfig, ProviderKind};
use capia_ai::testkit::{MockRequest, MockResponse, MockServer};
use capia_ai::types::{
    ChatEvent, ChatRequest, FinishReason, Message, ResponseSchema, ToolChoice, ToolSpec,
};
use capia_secrets::{CredentialRef, MemoryStore, SecretStore, SecretString};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
enum Family {
    OpenAi,
    Anthropic,
    Google,
}

const KEY: &str = "KEY-contract-0123456789abcdef";

fn sse(events: Vec<(Option<&str>, Value)>) -> MockResponse {
    MockResponse::Sse {
        events: events
            .into_iter()
            .map(|(e, v)| (e.map(str::to_owned), v.to_string()))
            .collect(),
        delay_ms: 0,
        hang: false,
    }
}

fn case_of(r: &MockRequest) -> String {
    let t = String::from_utf8_lossy(&r.body);
    t.find("CASE:").map_or_else(String::new, |p| {
        t[p + 5..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect()
    })
}

fn auth_header(f: Family, r: &MockRequest) -> String {
    match f {
        Family::OpenAi => r.header("authorization").unwrap_or("").to_owned(),
        Family::Anthropic => r.header("x-api-key").unwrap_or("").to_owned(),
        Family::Google => r.header("x-goog-api-key").unwrap_or("").to_owned(),
    }
}

/// Resposta do fabricante para um caso; o conteúdo canônico é igual nos três.
fn wire(f: Family, r: &MockRequest) -> MockResponse {
    if r.method == "GET" {
        return match f {
            Family::OpenAi => MockResponse::Json(200, json!({"data": [{"id": "m1", "context_length": 128000}, {"id": "m2"}]}).to_string()),
            Family::Anthropic => MockResponse::Json(200, json!({"data": [{"id": "claude-x", "display_name": "Claude X"}]}).to_string()),
            Family::Google => MockResponse::Json(200, json!({"models": [{"name": "models/gemini-x", "displayName": "Gemini X", "inputTokenLimit": 1000000}]}).to_string()),
        };
    }
    let case = case_of(r);
    let body = r.json();
    // o probe não usa marcadores: decide por características do pedido
    let has_tools = body.get("tools").is_some();
    let has_schema = body.get("response_format").is_some()
        || body.pointer("/generationConfig/responseSchema").is_some()
        || body.pointer("/tool_choice/name").and_then(Value::as_str)
            == Some("emit_structured_output");
    let has_image = String::from_utf8_lossy(&r.body).contains("image_url")
        || String::from_utf8_lossy(&r.body).contains("inlineData")
        || String::from_utf8_lossy(&r.body).contains("\"type\":\"image\"");
    let case = if !case.is_empty() {
        case
    } else if has_image {
        "vision".into()
    } else if has_schema {
        "structured".into()
    } else if has_tools {
        "tool".into()
    } else {
        "text".into()
    };
    match case.as_str() {
        "auth" => MockResponse::Json(401, json!({"error": {"message": format!("Incorrect API key provided: {}", auth_header(f, r))}}).to_string()),
        "rate" => MockResponse::RetryAfter(429, 2),
        "boom" => MockResponse::Json(500, json!({"error": {"message": "internal"}}).to_string()),
        "malformed" => MockResponse::Sse { events: vec![(None, "this is not json".into())], delay_ms: 0, hang: false },
        "slow" => MockResponse::Sse {
            events: vec![(None, text_chunk(f, "one ")), (None, text_chunk(f, "two "))],
            delay_ms: 150,
            hang: true,
        },
        "hang" => MockResponse::Hang,
        "vision" => text_stream(f, &["re", "d"]),
        "tool" => tool_stream(f),
        "structured" => json_stream(f),
        _ => text_stream(f, &["Hel", "lo"]),
    }
}

fn text_chunk(f: Family, t: &str) -> String {
    match f {
        Family::OpenAi => json!({"choices": [{"delta": {"content": t}}]}).to_string(),
        Family::Anthropic => json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": t}}).to_string(),
        Family::Google => json!({"candidates": [{"content": {"parts": [{"text": t}]}}]}).to_string(),
    }
}

fn text_stream(f: Family, parts: &[&str]) -> MockResponse {
    match f {
        Family::OpenAi => {
            let mut ev: Vec<(Option<&str>, Value)> = parts
                .iter()
                .map(|p| (None, json!({"choices": [{"delta": {"content": p}}]})))
                .collect();
            ev.push((
                None,
                json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
            ));
            ev.push((
                None,
                json!({"choices": [], "usage": {"prompt_tokens": 11, "completion_tokens": 2}}),
            ));
            let mut r = sse(ev);
            if let MockResponse::Sse { events, .. } = &mut r {
                events.push((None, "[DONE]".into()));
            }
            r
        }
        Family::Anthropic => {
            let mut ev: Vec<(Option<&str>, Value)> = vec![
                (
                    Some("message_start"),
                    json!({"type": "message_start", "message": {"usage": {"input_tokens": 11, "output_tokens": 1}}}),
                ),
                (
                    Some("content_block_start"),
                    json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                ),
            ];
            for p in parts {
                ev.push((Some("content_block_delta"), json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": p}})));
            }
            ev.push((
                Some("content_block_stop"),
                json!({"type": "content_block_stop", "index": 0}),
            ));
            ev.push((Some("message_delta"), json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 2}})));
            ev.push((Some("message_stop"), json!({"type": "message_stop"})));
            sse(ev)
        }
        Family::Google => {
            let mut ev: Vec<(Option<&str>, Value)> = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                let mut v =
                    json!({"candidates": [{"content": {"role": "model", "parts": [{"text": p}]}}]});
                if last {
                    v["candidates"][0]["finishReason"] = json!("STOP");
                    v["usageMetadata"] = json!({"promptTokenCount": 11, "candidatesTokenCount": 2});
                }
                ev.push((None, v));
            }
            sse(ev)
        }
    }
}

fn json_stream(f: Family) -> MockResponse {
    match f {
        Family::Anthropic => sse(vec![
            (
                Some("message_start"),
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 11}}}),
            ),
            (
                Some("content_block_start"),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_s", "name": "emit_structured_output", "input": {}}}),
            ),
            (
                Some("content_block_delta"),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"ok\":true,"}}),
            ),
            (
                Some("content_block_delta"),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "\"n\":7}"}}),
            ),
            (
                Some("content_block_stop"),
                json!({"type": "content_block_stop", "index": 0}),
            ),
            (
                Some("message_delta"),
                json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 2}}),
            ),
            (Some("message_stop"), json!({"type": "message_stop"})),
        ]),
        other => text_stream(other, &["{\"ok\":true,", "\"n\":7}"]),
    }
}

fn tool_stream(f: Family) -> MockResponse {
    match f {
        Family::OpenAi => {
            let mut r = sse(vec![
                (
                    None,
                    json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "echo", "arguments": ""}}]}}]}),
                ),
                (
                    None,
                    json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"mess"}}]}}]}),
                ),
                (
                    None,
                    json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "age\":\"hi\"}"}}]}}]}),
                ),
                (
                    None,
                    json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
                ),
                (
                    None,
                    json!({"choices": [], "usage": {"prompt_tokens": 11, "completion_tokens": 2}}),
                ),
            ]);
            if let MockResponse::Sse { events, .. } = &mut r {
                events.push((None, "[DONE]".into()));
            }
            r
        }
        Family::Anthropic => sse(vec![
            (
                Some("message_start"),
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 11}}}),
            ),
            (
                Some("content_block_start"),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "echo", "input": {}}}),
            ),
            (
                Some("content_block_delta"),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"mess"}}),
            ),
            (
                Some("content_block_delta"),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "age\":\"hi\"}"}}),
            ),
            (
                Some("content_block_stop"),
                json!({"type": "content_block_stop", "index": 0}),
            ),
            (
                Some("message_delta"),
                json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 2}}),
            ),
            (Some("message_stop"), json!({"type": "message_stop"})),
        ]),
        Family::Google => sse(vec![(
            None,
            json!({"candidates": [{"content": {"role": "model", "parts": [{"functionCall": {"name": "echo", "args": {"message": "hi"}}}]}, "finishReason": "STOP"}],
                   "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 2}}),
        )]),
    }
}

fn config(f: Family, url: &str, store: &Arc<dyn SecretStore>) -> ProviderConfig {
    let (id, kind) = match f {
        Family::OpenAi => ("oa", ProviderKind::OpenAiCompatible),
        Family::Anthropic => ("an", ProviderKind::Anthropic),
        Family::Google => ("go", ProviderKind::Google),
    };
    let mut c = ProviderConfig::new(id, kind, id);
    c.base_url = Some(url.to_owned());
    c.allow_loopback = true;
    c.enabled = true;
    let cred = CredentialRef::for_provider(id).unwrap();
    store.put(&cred, SecretString::new(KEY)).unwrap();
    c.credential_ref = Some(cred.as_str().to_owned());
    c.bound_host = Some("127.0.0.1".into());
    c
}

struct Rig {
    provider: Arc<dyn ModelProvider>,
    server: MockServer,
}

async fn rig(f: Family, tweak: impl FnOnce(&mut ProviderConfig)) -> Rig {
    let server = MockServer::start(move |r| wire(f, r)).await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let mut c = config(f, &server.url(), &store);
    tweak(&mut c);
    let provider = build_provider(&c, store).unwrap();
    Rig { provider, server }
}

fn req(model: &str, case: &str) -> ChatRequest {
    ChatRequest::new(model, vec![Message::user(format!("CASE:{case} please"))])
}

const FAMILIES: [Family; 3] = [Family::OpenAi, Family::Anthropic, Family::Google];

#[tokio::test]
async fn simple_text_streaming_usage_and_finish() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let ctx = CallCtx::default();
        let mut stream = r.provider.chat(&req("m", "text"), &ctx).await.unwrap();
        let (mut deltas, mut text, mut usage, mut finish) = (0, String::new(), None, None);
        while let Some(ev) = stream.next().await {
            match ev.unwrap() {
                ChatEvent::TextDelta { text: t } => {
                    deltas += 1;
                    text.push_str(&t);
                }
                ChatEvent::Usage { usage: u } => usage = Some(u),
                ChatEvent::Finish { reason } => finish = Some(reason),
                _ => {}
            }
        }
        assert_eq!(text, "Hello", "{f:?}");
        assert!(deltas >= 2, "{f:?}: streaming incremental");
        let u = usage.unwrap_or_else(|| panic!("{f:?}: sem usage"));
        assert_eq!((u.input_tokens, u.output_tokens), (11, 2), "{f:?}");
        assert!(!u.synthetic);
        assert_eq!(finish, Some(FinishReason::Stop), "{f:?}");
        // credencial no header certo e nunca no corpo/URL
        let seen = r.server.seen();
        assert_eq!(seen.len(), 1);
        let raw = seen[0].raw_text();
        let in_header = auth_header(f, &seen[0]).contains(KEY);
        assert!(in_header, "{f:?}: a chave deve ir no header de auth");
        assert!(
            !String::from_utf8_lossy(&seen[0].body).contains(KEY),
            "{f:?}: chave no corpo"
        );
        assert!(!seen[0].path.contains(KEY), "{f:?}: chave na URL");
        assert!(raw.contains("CASE:text"));
    }
}

#[tokio::test]
async fn tool_call_is_normalized_identically() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let ctx = CallCtx::default();
        let mut q = req("m", "tool");
        q.tools.push(ToolSpec { name: "echo".into(), description: "echo".into(), input_schema: json!({"type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]}) });
        q.tool_choice = ToolChoice::Required;
        let out = collect(r.provider.chat(&q, &ctx).await.unwrap(), &ctx.cancel)
            .await
            .unwrap();
        assert_eq!(out.tool_calls.len(), 1, "{f:?}");
        assert_eq!(out.tool_calls[0].name, "echo");
        assert_eq!(
            out.tool_calls[0].arguments,
            json!({"message": "hi"}),
            "{f:?}"
        );
        assert_eq!(out.finish, Some(FinishReason::ToolCalls), "{f:?}");
        // o schema da tool chegou ao fabricante
        assert!(String::from_utf8_lossy(&r.server.seen()[0].body).contains("\"message\""));
    }
}

#[tokio::test]
async fn structured_output_returns_valid_json_for_every_family() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let ctx = CallCtx::default();
        let mut q = req("m", "structured");
        q.response_schema = Some(ResponseSchema {
            name: "probe".into(),
            schema: json!({"type": "object", "properties": {"ok": {"type": "boolean"}, "n": {"type": "integer"}}, "required": ["ok", "n"], "additionalProperties": false}),
        });
        let out = collect(r.provider.chat(&q, &ctx).await.unwrap(), &ctx.cancel)
            .await
            .unwrap();
        let v: Value =
            serde_json::from_str(&out.text).unwrap_or_else(|e| panic!("{f:?}: {e}: {}", out.text));
        assert_eq!(v, json!({"ok": true, "n": 7}), "{f:?}");
        assert!(
            out.tool_calls.is_empty(),
            "{f:?}: a tool sintética não vaza como tool call"
        );
        assert_eq!(out.finish, Some(FinishReason::Stop), "{f:?}");
    }
}

#[tokio::test]
async fn http_errors_are_classified_and_the_key_never_leaks_into_errors() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let ctx = CallCtx::default();
        let e = r
            .provider
            .chat(&req("m", "auth"), &ctx)
            .await
            .err()
            .unwrap();
        assert_eq!(e.code, ErrorCode::AuthFailed, "{f:?}");
        assert!(
            !e.message.contains(KEY) && !format!("{e:?}").contains(KEY),
            "{f:?}: o servidor ecoou a chave e ela vazou: {e}"
        );
        let e = r
            .provider
            .chat(&req("m", "rate"), &ctx)
            .await
            .err()
            .unwrap();
        assert_eq!(
            (e.code, e.retry_after_ms),
            (ErrorCode::RateLimited, Some(2000)),
            "{f:?}"
        );
        let e = r
            .provider
            .chat(&req("m", "boom"), &ctx)
            .await
            .err()
            .unwrap();
        assert_eq!(e.code, ErrorCode::ProviderUnavailable, "{f:?}");
        assert_eq!(e.status, Some(500));
    }
}

#[tokio::test]
async fn malformed_stream_is_an_invalid_response_not_a_panic() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let ctx = CallCtx::default();
        let e = collect(
            r.provider.chat(&req("m", "malformed"), &ctx).await.unwrap(),
            &ctx.cancel,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidProviderResponse, "{f:?}: {e}");
    }
}

#[tokio::test]
async fn first_byte_timeout_and_idle_timeout() {
    for f in FAMILIES {
        let r = rig(f, |c| {
            c.first_byte_timeout_s = Some(1);
            c.idle_timeout_s = Some(1);
            c.timeout_s = 20;
        })
        .await;
        let ctx = CallCtx::default();
        let t0 = Instant::now();
        let e = r
            .provider
            .chat(&req("m", "hang"), &ctx)
            .await
            .err()
            .unwrap();
        assert_eq!(e.code, ErrorCode::ProviderTimeout, "{f:?}");
        assert!(t0.elapsed() < Duration::from_secs(8));
        // stream que começa e nunca termina ⇒ idle timeout
        let r = rig(f, |c| {
            c.idle_timeout_s = Some(1);
            c.timeout_s = 20;
        })
        .await;
        let e = collect(
            r.provider.chat(&req("m", "slow"), &ctx).await.unwrap(),
            &ctx.cancel,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::ProviderTimeout, "{f:?}: {e}");
        assert!(e.message.contains("idle"), "{e}");
    }
}

#[tokio::test]
async fn cancel_aborts_the_http_connection_and_no_late_events_arrive() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let ctx = CallCtx::default();
        let cancel = ctx.cancel.clone();
        let mut stream = r.provider.chat(&req("m", "slow"), &ctx).await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(first, ChatEvent::TextDelta { .. })
                || matches!(first, ChatEvent::ToolCallDelta { .. }),
            "{f:?}: {first:?}"
        );
        cancel.cancel();
        // depois do cancelamento nenhum evento de conteúdo pode surgir
        let h = tokio::spawn(async move { collect(stream, &cancel).await });
        let e = tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled, "{f:?}");
        // o servidor viu a conexão fechar (abort real, não só descarte local)
        let t0 = Instant::now();
        while r
            .server
            .client_aborts
            .load(std::sync::atomic::Ordering::SeqCst)
            == 0
            && t0.elapsed() < Duration::from_secs(5)
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            r.server
                .client_aborts
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "{f:?}: a conexão deveria ter sido fechada"
        );
    }
}

#[tokio::test]
async fn list_models_is_normalized() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let m = r.provider.list_models(&CallCtx::default()).await.unwrap();
        assert!(!m.is_empty(), "{f:?}");
        assert!(
            m.iter()
                .all(|x| !x.id.is_empty() && !x.id.starts_with("models/")),
            "{f:?}: {m:?}"
        );
    }
}

#[tokio::test]
async fn probe_measures_real_capabilities_on_all_families() {
    for f in FAMILIES {
        let r = rig(f, |_| {}).await;
        let res = probe(
            r.provider.as_ref(),
            "m",
            ProbeOptions {
                chat: true,
                vision: true,
                stt: false,
            },
            &CancelToken::new(),
        )
        .await;
        assert!(res.connection_error.is_none(), "{f:?}: {res:?}");
        for c in [
            Capability::TextGeneration,
            Capability::Streaming,
            Capability::ToolCalling,
            Capability::StructuredOutput,
            Capability::VisionInput,
        ] {
            assert!(
                res.verified.contains(&c),
                "{f:?}: {c:?} não verificada: {res:?}"
            );
        }
        assert!(res.success, "{f:?}: {res:?}");
        assert!(!serde_json::to_string(&res).unwrap().contains(KEY));
    }
}

#[tokio::test]
async fn probe_reports_auth_failure_without_leaking_and_stops() {
    let server = MockServer::start(|r| {
        MockResponse::Json(401, json!({"error": {"message": format!("bad key {}", r.header("authorization").unwrap_or(""))}}).to_string())
    })
    .await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let c = config(Family::OpenAi, &server.url(), &store);
    let p = build_provider(&c, store).unwrap();
    let res = probe(
        p.as_ref(),
        "m",
        ProbeOptions {
            chat: true,
            vision: true,
            stt: false,
        },
        &CancelToken::new(),
    )
    .await;
    assert!(!res.success);
    assert!(
        res.connection_error
            .as_deref()
            .is_some_and(|m| m.contains("AUTH_FAILED")),
        "{res:?}"
    );
    assert!(res.verified.is_empty());
    assert_eq!(
        server.seen().len(),
        1,
        "auth falhou: os demais probes não devem ser tentados"
    );
    assert!(!serde_json::to_string(&res).unwrap().contains(KEY));
}

#[tokio::test]
async fn non_streaming_openai_compatible_servers_still_work() {
    let server = MockServer::start(|_| {
        MockResponse::Json(200, json!({"choices": [{"message": {"content": "pong"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 1}}).to_string())
    })
    .await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let c = config(Family::OpenAi, &server.url(), &store);
    let p = build_provider(&c, store).unwrap();
    let ctx = CallCtx::default();
    let out = collect(p.chat(&req("m", "text"), &ctx).await.unwrap(), &ctx.cancel)
        .await
        .unwrap();
    assert_eq!((out.text.as_str(), out.usage.output_tokens), ("pong", 1));
}
