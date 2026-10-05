//! Segurança dos providers (PHASE4_PROVIDERS_SECURITY §13/§14/§28): vazamento de credencial por
//! redirect, SSRF, corpo gigante, vínculo credencial↔host, canário em todos os artefatos.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_ai::CancelToken;
use capia_ai::brain::BrainProfile;
use capia_ai::capability::{Capabilities, Capability};
use capia_ai::dispatcher::{AiRuntime, ChatOptions, RetryPolicy, TaskCtx};
use capia_ai::error::ErrorCode;
use capia_ai::providers::{CallCtx, build_provider};
use capia_ai::registry::{ModelEndpoint, ProviderConfig, ProviderKind, Registry};
use capia_ai::testkit::{MockResponse, MockServer};
use capia_ai::types::{ChatRequest, Message, ToolSpec};
use capia_ai::usage::MemorySink;
use capia_secrets::{CredentialRef, MemoryStore, SecretStore, SecretString};
use serde_json::json;
use std::sync::Arc;

const CANARY: &str = "CNRY-7f3a9c0b1d2e4f56a7b8c9d0";

fn local_cfg(id: &str, url: &str, store: &Arc<dyn SecretStore>) -> ProviderConfig {
    let mut c = ProviderConfig::new(id, ProviderKind::LocalOpenAiCompatible, id);
    c.base_url = Some(url.to_owned());
    c.enabled = true;
    let cred = CredentialRef::for_provider(id).unwrap();
    store.put(&cred, SecretString::new(CANARY)).unwrap();
    c.credential_ref = Some(cred.as_str().to_owned());
    c.bound_host = Some("127.0.0.1".into());
    c
}

fn req() -> ChatRequest {
    ChatRequest::new("m", vec![Message::user("oi")])
}

#[tokio::test]
async fn redirect_to_another_host_never_receives_the_credential() {
    let sink =
        MockServer::start(|_| MockResponse::Json(200, json!({"data": []}).to_string())).await;
    let target = sink.url_localhost(); // host diferente de 127.0.0.1
    let origin =
        MockServer::start(move |_| MockResponse::Redirect(format!("{target}/stolen"))).await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let p = build_provider(&local_cfg("loc", &origin.url(), &store), store).unwrap();
    let e = p.list_models(&CallCtx::default()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::ProviderUnavailable, "{e}");
    assert!(e.message.contains("redirect"), "{e}");
    assert_eq!(origin.seen().len(), 1);
    assert!(
        sink.seen().is_empty(),
        "o host de destino NÃO pode receber nada: {:?}",
        sink.seen().iter().map(|r| r.raw_text()).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn same_host_redirect_is_followed() {
    let state = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s2 = state.clone();
    let srv = MockServer::start(move |r| {
        if r.path.ends_with("/models") && s2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        {
            MockResponse::Redirect("/v1/models2".into())
        } else {
            MockResponse::Json(200, json!({"data": [{"id": "ok"}]}).to_string())
        }
    })
    .await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let p = build_provider(
        &local_cfg("loc", &format!("{}/v1", srv.url()), &store),
        store,
    )
    .unwrap();
    let m = p.list_models(&CallCtx::default()).await.unwrap();
    assert_eq!(m[0].id, "ok");
}

#[tokio::test]
async fn gigantic_response_is_cut_by_the_body_limit() {
    let srv = MockServer::start(|_| MockResponse::Huge(40 * 1024 * 1024)).await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let p = build_provider(&local_cfg("loc", &srv.url(), &store), store).unwrap();
    let e = p.list_models(&CallCtx::default()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidProviderResponse, "{e}");
    assert!(e.message.contains("limit"), "{e}");
}

#[tokio::test]
async fn changing_the_base_url_host_requires_re_entering_the_key() {
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let mut c = local_cfg("loc", "http://127.0.0.1:1234/v1", &store);
    assert!(build_provider(&c, store.clone()).is_ok());
    c.base_url = Some("http://localhost:1234/v1".into()); // outro host, mesma máquina
    let e = build_provider(&c, store).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotAllowed, "{e}");
    assert!(e.message.contains("bound"), "{e}");
}

#[test]
fn public_providers_reject_private_loopback_metadata_and_bad_schemes() {
    for url in [
        "https://127.0.0.1/v1",
        "https://localhost/v1",
        "https://10.1.2.3/v1",
        "https://192.168.0.5/v1",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/v1",
        "https://[fd00::1]/v1",
        "http://api.openai.com/v1",
        "file:///etc/passwd",
        "ftp://x.com/",
        "https://user:pass@api.openai.com/v1",
    ] {
        let mut c = ProviderConfig::new("p", ProviderKind::OpenAiCompatible, "p");
        c.base_url = Some(url.into());
        assert!(c.validate().is_err(), "{url} deveria ser recusada");
    }
    // custom header malicioso (CRLF injection) e headers reservados
    let mut c = ProviderConfig::new("p", ProviderKind::OpenAiCompatible, "p");
    c.extra_headers
        .insert("X-A".into(), "v\r\nX-Injected: 1".into());
    assert!(c.validate().is_err());
    c.extra_headers.clear();
    c.extra_headers
        .insert("Authorization".into(), "Bearer x".into());
    assert!(c.validate().is_err());
}

/// O canário: uma chave reconhecível percorre save → probe → chat → tool → erro → timeout →
/// fallback. Nada que o core produz (erros, registros de uso, registry, probe, Debug, auditoria)
/// pode conter a chave. (Projeto/IPC/diagnóstico do app: testes de `capia-intelligence`/desktop.)
#[tokio::test]
async fn canary_never_appears_in_any_artifact_the_core_produces() {
    // provider A: ecoa a chave nos erros 401 e no corpo de erro; provider B: funciona
    let a = MockServer::start(|r| {
        let hdr = r.header("authorization").unwrap_or("").to_owned();
        if String::from_utf8_lossy(&r.body).contains("probe") || r.method == "GET" {
            return MockResponse::Json(401, json!({"error": {"message": format!("Incorrect API key provided: {hdr}. See ?api_key={hdr}")}}).to_string());
        }
        MockResponse::Json(401, json!({"error": {"message": format!("Authorization: {hdr}")}}).to_string())
    })
    .await;
    let b = MockServer::start(|_| MockResponse::Sse {
        events: vec![
            (
                None,
                json!({"choices": [{"delta": {"content": "ok"}}]}).to_string(),
            ),
            (
                None,
                json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}).to_string(),
            ),
            (None, "[DONE]".into()),
        ],
        delay_ms: 0,
        hang: false,
    })
    .await;
    let hang = MockServer::start(|_| MockResponse::Hang).await;
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let mut reg = Registry::new();
    let mut cfgs = vec![
        local_cfg("pa", &a.url(), &store),
        local_cfg("pb", &b.url(), &store),
        local_cfg("ph", &hang.url(), &store),
    ];
    cfgs[2].first_byte_timeout_s = Some(1);
    cfgs[2].timeout_s = 5;
    for c in cfgs {
        reg.providers.insert(c.id.clone(), c);
    }
    let caps = Capabilities::declared(&[
        Capability::TextGeneration,
        Capability::ToolCalling,
        Capability::StructuredOutput,
        Capability::Streaming,
    ]);
    for pid in ["pa", "ph", "pb"] {
        let mut m = ModelEndpoint::new(format!("{pid}:m"), pid, "m");
        m.capabilities = caps.clone();
        m.context_window = 200_000;
        reg.models.insert(m.id.clone(), m);
    }
    let mut prof = BrainProfile::new("pf", "pf", "pa:m");
    prof.fallbacks.insert(
        Capability::TextGeneration,
        vec!["ph:m".into(), "pb:m".into()],
    );
    reg.profiles.insert("pf".into(), prof);
    reg.active_profile = Some("pf".into());
    let sink = Arc::new(MemorySink::default());
    let mut rt = AiRuntime::new(reg, store.clone(), sink.clone());
    rt.retry = RetryPolicy {
        max_attempts: 2,
        base_ms: 1,
        max_ms: 5,
    };

    let mut artifacts: Vec<(String, String)> = Vec::new();
    // probe (falha de auth que ecoa a chave)
    let probe = rt
        .probe_endpoint("pa:m", &CancelToken::new())
        .await
        .unwrap();
    artifacts.push(("probe".into(), serde_json::to_string(&probe).unwrap()));
    artifacts.push(("probe-debug".into(), format!("{probe:?}")));
    // chat com tool + fallback através de auth-fail → timeout → sucesso
    let t = TaskCtx::new("canary-task", &rt.registry().active().cloned().unwrap());
    let mut q = req();
    q.tools.push(ToolSpec {
        name: "echo".into(),
        description: "d".into(),
        input_schema: json!({"type": "object"}),
    });
    let out = rt.chat(&t, q, ChatOptions::text(), None).await.unwrap();
    assert_eq!(
        out.decision.endpoint_id, "pb:m",
        "caiu por auth-fail e timeout até o provider saudável"
    );
    for att in &out.attempts {
        if let Some(e) = &att.error {
            artifacts.push(("attempt-error".into(), format!("{e} / {e:?}")));
        }
    }
    // erro final isolado (sem fallback)
    let mut only_a = rt.registry();
    only_a.profiles.get_mut("pf").unwrap().fallbacks.clear();
    only_a.models.remove("ph:m");
    only_a.models.remove("pb:m");
    rt.update_registry(|r| *r = only_a);
    let e = rt
        .chat(&t, req(), ChatOptions::text(), None)
        .await
        .unwrap_err();
    artifacts.push((
        "final-error".into(),
        format!("{e} / {e:?} / {}", serde_json::to_string(&e).unwrap()),
    ));
    // registros de uso, registry e Debug dos objetos
    artifacts.push((
        "usage".into(),
        serde_json::to_string(&sink.snapshot()).unwrap(),
    ));
    artifacts.push((
        "registry".into(),
        serde_json::to_string(&rt.registry()).unwrap(),
    ));
    artifacts.push(("runtime-debug".into(), format!("{rt:?}")));
    artifacts.push(("sink-debug".into(), format!("{sink:?}")));
    let provider = build_provider(&local_cfg("dbg", &a.url(), &store), store.clone()).unwrap();
    artifacts.push(("provider-debug".into(), format!("{provider:?}")));
    artifacts.push((
        "secret-debug".into(),
        format!(
            "{:?} {}",
            SecretString::new(CANARY),
            SecretString::new(CANARY)
        ),
    ));

    for (name, text) in &artifacts {
        assert!(!text.contains(CANARY), "CANÁRIO VAZOU em `{name}`: {text}");
        assert!(
            !text.contains("Q05SWS03ZjNh"),
            "canário em base64 vazou em `{name}`"
        );
    }
    // sanidade do teste: o servidor realmente recebeu a chave (ela é enviada ao provider, e só a ele)
    assert!(a.seen().iter().any(|r| r.raw_text().contains(CANARY)));
    // e o provider B (saudável) recebeu a chave só no header de auth
    for r in b.seen() {
        assert!(!String::from_utf8_lossy(&r.body).contains(CANARY));
        assert!(!r.path.contains(CANARY));
    }
}
