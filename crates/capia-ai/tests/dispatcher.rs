//! `AiRuntime`: roteamento, retry/backoff, fallback, orçamento, cache, saída estruturada com reparo,
//! cancelamento, AI-Off, privacidade e custo — tudo com o provider Replay (determinístico, sem rede).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_ai::brain::{BrainProfile, PrivacyPolicy};
use capia_ai::capability::{Capabilities, Capability};
use capia_ai::dispatcher::{
    AiRuntime, ChatOptions, MemoryCache, RetryPolicy, StreamNotice, TaskCtx,
};
use capia_ai::error::ErrorCode;
use capia_ai::providers::replay::{ReplayProvider, ReplayResponse};
use capia_ai::registry::{Health, ModelEndpoint, Pricing, ProviderConfig, ProviderKind, Registry};
use capia_ai::router::RouteRequest;
use capia_ai::stt::{Segment, SttRequest, Transcript};
use capia_ai::types::{ChatEvent, ChatRequest, Message};
use capia_ai::usage::{CallStatus, MemorySink};
use capia_secrets::MemoryStore;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn ok(text: &str) -> ReplayResponse {
    ReplayResponse::Chat {
        events: vec![ChatEvent::TextDelta { text: text.into() }],
        chunk_delay_ms: 0,
    }
}

fn err(code: ErrorCode) -> ReplayResponse {
    ReplayResponse::Error {
        code,
        message: "synthetic".into(),
        status: None,
        retry_after_ms: Some(1),
        after_events: vec![],
    }
}

struct World {
    rt: Arc<AiRuntime>,
    sink: Arc<MemorySink>,
    replays: Vec<Arc<ReplayProvider>>,
}

fn full_caps() -> Capabilities {
    Capabilities::declared(&[
        Capability::TextGeneration,
        Capability::ToolCalling,
        Capability::StructuredOutput,
        Capability::Streaming,
    ])
}

/// `providers`: (id, kind, script). O 1º endpoint é o Brain; os demais são fallbacks de texto.
fn world(providers: Vec<(&str, ProviderKind, Vec<ReplayResponse>)>) -> World {
    let mut reg = Registry::new();
    let mut replays = Vec::new();
    let mut ids = Vec::new();
    let sink = Arc::new(MemorySink::default());
    let rt_store = Arc::new(MemoryStore::new());
    for (pid, kind, script) in providers {
        let mut p = ProviderConfig::new(pid, kind, pid);
        p.enabled = true;
        reg.providers.insert(pid.into(), p);
        let mut m = ModelEndpoint::new(format!("{pid}:m"), pid, "m");
        m.capabilities = full_caps();
        m.context_window = 200_000;
        ids.push(m.id.clone());
        reg.models.insert(m.id.clone(), m);
        replays.push(Arc::new(ReplayProvider::scripted(pid, script)));
    }
    let mut prof = BrainProfile::new("pf", "pf", ids[0].clone());
    prof.fallbacks
        .insert(Capability::TextGeneration, ids[1..].to_vec());
    reg.profiles.insert("pf".into(), prof);
    reg.active_profile = Some("pf".into());
    let mut rt =
        AiRuntime::new(reg, rt_store, sink.clone()).with_cache(Arc::new(MemoryCache::default()));
    rt.retry = RetryPolicy {
        max_attempts: 3,
        base_ms: 1,
        max_ms: 5,
    };
    let rt = Arc::new(rt);
    for r in &replays {
        rt.register_replay(r.id_str(), r.clone());
    }
    World { rt, sink, replays }
}

trait IdStr {
    fn id_str(&self) -> &str;
}
impl IdStr for Arc<ReplayProvider> {
    fn id_str(&self) -> &str {
        use capia_ai::providers::ModelProvider;
        self.id()
    }
}

fn task(w: &World) -> TaskCtx {
    TaskCtx::new("task1", &w.rt.registry().active().cloned().unwrap())
}

fn chat_req(t: &str) -> ChatRequest {
    ChatRequest::new("ignored", vec![Message::user(t)])
}

#[tokio::test]
async fn retries_only_transient_errors_and_records_every_attempt() {
    let w = world(vec![(
        "a",
        ProviderKind::Replay,
        vec![
            err(ErrorCode::RateLimited),
            err(ErrorCode::ProviderUnavailable),
            ok("fim"),
        ],
    )]);
    let t = task(&w);
    let out =
        w.rt.chat(&t, chat_req("oi"), ChatOptions::text(), None)
            .await
            .unwrap();
    assert_eq!(out.response.text, "fim");
    assert_eq!(out.attempts.len(), 3);
    assert_eq!(w.replays[0].call_count(), 3);
    let recs = w.sink.snapshot();
    assert_eq!(
        recs.iter()
            .filter(|r| r.status == CallStatus::Failed)
            .count(),
        2
    );
    assert_eq!(
        recs.iter().filter(|r| r.status == CallStatus::Ok).count(),
        1
    );
    assert_eq!(
        recs.iter().map(|r| r.attempt).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    // só a tentativa que respondeu gastou tokens
    assert_eq!(t.budget.spent().calls, 1);
    assert!(recs.iter().all(|r| !r.request_id.is_empty()));
}

#[tokio::test]
async fn retry_is_bounded_then_falls_back_and_notifies() {
    let w = world(vec![
        (
            "a",
            ProviderKind::Replay,
            vec![
                err(ErrorCode::ProviderUnavailable),
                err(ErrorCode::ProviderUnavailable),
                err(ErrorCode::ProviderUnavailable),
            ],
        ),
        ("b", ProviderKind::Replay, vec![ok("do plano B")]),
    ]);
    let notes: Arc<Mutex<Vec<String>>> = Arc::default();
    let n2 = notes.clone();
    let cb = move |n: StreamNotice| {
        n2.lock().unwrap().push(match n {
            StreamNotice::Retry { attempt, .. } => format!("retry{attempt}"),
            StreamNotice::Fallback { endpoint_id, .. } => format!("fallback:{endpoint_id}"),
            StreamNotice::Event(_) => "ev".into(),
        });
    };
    let t = task(&w);
    let out =
        w.rt.chat(&t, chat_req("oi"), ChatOptions::text(), Some(&cb))
            .await
            .unwrap();
    assert_eq!(out.decision.endpoint_id, "b:m");
    assert_eq!(out.response.text, "do plano B");
    assert_eq!(
        w.replays[0].call_count(),
        3,
        "exatamente 3 tentativas, sem retry infinito"
    );
    let n = notes.lock().unwrap().clone();
    assert!(
        n.contains(&"retry2".to_owned()) && n.contains(&"retry3".to_owned()),
        "{n:?}"
    );
    assert!(n.contains(&"fallback:b:m".to_owned()), "{n:?}");
}

#[tokio::test]
async fn invalid_request_never_falls_back_but_auth_failure_does() {
    let w = world(vec![
        (
            "a",
            ProviderKind::Replay,
            vec![err(ErrorCode::InvalidRequest)],
        ),
        ("b", ProviderKind::Replay, vec![ok("x")]),
    ]);
    let e =
        w.rt.chat(&task(&w), chat_req("oi"), ChatOptions::text(), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    assert_eq!(
        w.replays[1].call_count(),
        0,
        "pedido inválido não vai para outro modelo"
    );
    let w = world(vec![
        ("a", ProviderKind::Replay, vec![err(ErrorCode::AuthFailed)]),
        ("b", ProviderKind::Replay, vec![ok("b ok")]),
    ]);
    let out =
        w.rt.chat(&task(&w), chat_req("oi"), ChatOptions::text(), None)
            .await
            .unwrap();
    assert_eq!(out.response.text, "b ok");
    assert_eq!(w.replays[0].call_count(), 1, "auth não tem retry");
}

#[tokio::test]
async fn vision_never_falls_back_to_a_text_only_model() {
    let mut w = world(vec![
        (
            "a",
            ProviderKind::Replay,
            vec![err(ErrorCode::ProviderUnavailable); 3],
        ),
        ("b", ProviderKind::Replay, vec![ok("não deveria")]),
    ]);
    // só "a" tem visão
    w.rt.update_registry(|r| {
        r.models.get_mut("a:m").unwrap().capabilities.set(
            Capability::VisionInput,
            true,
            capia_ai::capability::Origin::Declared,
        );
    });
    let mut req = chat_req("descreva");
    req.messages[0].parts.push(capia_ai::types::Part::Image {
        mime: "image/png".into(),
        data_b64: "AAAA".into(),
    });
    let e =
        w.rt.chat(&task(&w), req, ChatOptions::text(), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::ProviderUnavailable);
    assert_eq!(
        w.replays[1].call_count(),
        0,
        "modelo sem visão não é fallback de tarefa de visão"
    );
    let _ = &mut w;
}

#[tokio::test]
async fn structured_output_is_validated_repaired_and_bounded() {
    let schema = json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"], "additionalProperties": false});
    let w = world(vec![(
        "a",
        ProviderKind::Replay,
        vec![ok("nope not json"), ok("```json\n{\"n\": 3}\n```")],
    )]);
    let (v, out) =
        w.rt.chat_structured(
            &task(&w),
            chat_req("dê n"),
            "spec",
            &schema,
            ChatOptions::text(),
            1,
            None,
        )
        .await
        .unwrap();
    assert_eq!(v, json!({"n": 3}));
    assert_eq!(w.replays[0].call_count(), 2, "1 chamada + 1 reparo");
    assert!(out.response.text.contains("```json"));
    // o reparo contém o motivo
    let reqs = w.replays[0].requests.lock().unwrap().clone();
    assert!(
        reqs[1]
            .messages
            .last()
            .unwrap()
            .text_of()
            .contains("invalid")
    );

    // reparo esgotado ⇒ erro estruturado (nunca JSON parcial aceito)
    let w = world(vec![(
        "a",
        ProviderKind::Replay,
        vec![ok("{\"n\": \"x\"}"), ok("{\"n\": \"y\"}")],
    )]);
    let e =
        w.rt.chat_structured(
            &task(&w),
            chat_req("dê n"),
            "spec",
            &schema,
            ChatOptions::text(),
            1,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::StructuredOutputInvalid);
    assert_eq!(w.replays[0].call_count(), 2);
}

#[tokio::test]
async fn native_structured_output_is_used_when_available_and_emulated_otherwise() {
    let schema =
        json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]});
    let w = world(vec![("a", ProviderKind::Replay, vec![ok("{\"n\":1}")])]);
    w.rt.chat_structured(
        &task(&w),
        chat_req("x"),
        "spec",
        &schema,
        ChatOptions::text(),
        0,
        None,
    )
    .await
    .unwrap();
    let r = w.replays[0].requests.lock().unwrap()[0].clone();
    assert!(r.response_schema.is_some(), "nativo");
    // endpoint sem StructuredOutput ⇒ emulação com o schema no system prompt
    let w = world(vec![("a", ProviderKind::Replay, vec![ok("{\"n\":1}")])]);
    w.rt.update_registry(|reg| {
        reg.models.get_mut("a:m").unwrap().capabilities =
            Capabilities::declared(&[Capability::TextGeneration, Capability::ToolCalling]);
    });
    w.rt.chat_structured(
        &task(&w),
        chat_req("x"),
        "spec",
        &schema,
        ChatOptions::text(),
        0,
        None,
    )
    .await
    .unwrap();
    let r = w.replays[0].requests.lock().unwrap()[0].clone();
    assert!(r.response_schema.is_none());
    assert!(r.messages[0].text_of().contains("JSON Schema"));
}

#[tokio::test]
async fn deterministic_cache_skips_the_provider_and_costs_nothing() {
    let w = world(vec![("a", ProviderKind::Replay, vec![ok("resposta")])]);
    let t = task(&w);
    let mut opts = ChatOptions::text();
    opts.cacheable = true;
    let mut req = chat_req("determinístico");
    req.params.temperature = Some(0.0);
    let first =
        w.rt.chat(&t, req.clone(), opts.clone(), None)
            .await
            .unwrap();
    let second = w.rt.chat(&t, req, opts, None).await.unwrap();
    assert!(!first.cache_hit && second.cache_hit);
    assert_eq!(second.response.text, "resposta");
    assert_eq!(w.replays[0].call_count(), 1);
    let recs = w.sink.snapshot();
    assert_eq!(recs.last().unwrap().status, CallStatus::CacheHit);
    assert!(recs.last().unwrap().cost.known && recs.last().unwrap().cost.micros == 0);
    assert_eq!(
        t.budget.spent().calls,
        1,
        "cache hit não conta como gasto de provider"
    );
}

#[tokio::test]
async fn budget_blocks_before_calling_the_provider() {
    let w = world(vec![("a", ProviderKind::Replay, vec![ok("1"), ok("2")])]);
    w.rt.update_registry(|r| {
        r.profiles.get_mut("pf").unwrap().budgets.max_calls_per_task = Some(1)
    });
    let t = TaskCtx::new("t", &w.rt.registry().active().cloned().unwrap());
    w.rt.chat(&t, chat_req("um"), ChatOptions::text(), None)
        .await
        .unwrap();
    let e =
        w.rt.chat(&t, chat_req("dois"), ChatOptions::text(), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::BudgetExceeded);
    assert_eq!(w.replays[0].call_count(), 1);
}

#[tokio::test]
async fn cancel_mid_stream_aborts_records_cancelled_and_does_not_retry() {
    let events: Vec<ChatEvent> = (0..100)
        .map(|i| ChatEvent::TextDelta {
            text: format!("{i} "),
        })
        .collect();
    let w = world(vec![(
        "a",
        ProviderKind::Replay,
        vec![
            ReplayResponse::Chat {
                events,
                chunk_delay_ms: 20,
            },
            ok("não deve rodar"),
        ],
    )]);
    let t = task(&w);
    let cancel = t.cancel.clone();
    let rt = w.rt.clone();
    let t2 = t.clone();
    let h = tokio::spawn(async move {
        rt.chat(&t2, chat_req("longo"), ChatOptions::text(), None)
            .await
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    cancel.cancel();
    let e = h.await.unwrap().unwrap_err();
    assert_eq!(e.code, ErrorCode::Cancelled);
    assert_eq!(w.replays[0].call_count(), 1, "cancelado não repete");
    assert_eq!(
        w.sink.snapshot().last().unwrap().status,
        CallStatus::Cancelled
    );
    // tarefa cancelada não inicia chamada nova
    let e =
        w.rt.chat(&t, chat_req("depois"), ChatOptions::text(), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::Cancelled);
    assert_eq!(w.replays[0].call_count(), 1);
}

#[tokio::test]
async fn ai_off_makes_zero_provider_calls() {
    let w = world(vec![("a", ProviderKind::Replay, vec![ok("x")])]);
    w.rt.update_registry(|r| r.ai_enabled = false);
    let e =
        w.rt.chat(&task(&w), chat_req("oi"), ChatOptions::text(), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotConfigured);
    assert_eq!(w.replays[0].call_count(), 0);
    assert!(w.sink.snapshot().is_empty());
}

#[tokio::test]
async fn provider_disabled_or_model_removed_mid_task_is_handled_cleanly() {
    let w = world(vec![
        ("a", ProviderKind::Replay, vec![ok("1"), ok("2")]),
        ("b", ProviderKind::Replay, vec![ok("b1")]),
    ]);
    let t = task(&w);
    assert_eq!(
        w.rt.chat(&t, chat_req("um"), ChatOptions::text(), None)
            .await
            .unwrap()
            .decision
            .endpoint_id,
        "a:m"
    );
    // o provider do Brain é desligado no meio da tarefa ⇒ a próxima chamada vai ao fallback habilitado
    w.rt.update_registry(|r| r.providers.get_mut("a").unwrap().enabled = false);
    let out =
        w.rt.chat(&t, chat_req("dois"), ChatOptions::text(), None)
            .await
            .unwrap();
    assert_eq!(out.decision.endpoint_id, "b:m");
    // modelo removido: sem rota ⇒ erro explícito
    w.rt.update_registry(|r| {
        r.models.remove("b:m");
    });
    let e =
        w.rt.chat(&t, chat_req("três"), ChatOptions::text(), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::NoCapableModel);
}

#[tokio::test]
async fn cost_is_known_only_with_declared_pricing() {
    let w = world(vec![(
        "a",
        ProviderKind::Replay,
        vec![ok("sem preço"), ok("com preço")],
    )]);
    let t = task(&w);
    let unknown =
        w.rt.chat(&t, chat_req("um"), ChatOptions::text(), None)
            .await
            .unwrap();
    assert!(!unknown.cost.known);
    w.rt.update_registry(|r| {
        r.models.get_mut("a:m").unwrap().pricing = Some(Pricing {
            currency: "USD".into(),
            input_micros_per_mtok: 1_000_000,
            output_micros_per_mtok: 2_000_000,
            cached_input_micros_per_mtok: None,
            audio_micros_per_second: None,
            image_micros_each: None,
            source: "teste".into(),
            effective_date: "2026-01-01".into(),
        });
    });
    let known =
        w.rt.chat(&t, chat_req("dois"), ChatOptions::text(), None)
            .await
            .unwrap();
    assert!(known.cost.known, "{:?}", known.cost);
    assert_eq!(known.cost.currency.as_deref(), Some("USD"));
    let recs = w.sink.snapshot();
    assert_eq!(
        recs.last().unwrap().pricing_date.as_deref(),
        Some("2026-01-01")
    );
    assert!(
        recs.last().unwrap().usage.synthetic,
        "uso do Replay é sintético e marcado"
    );
    assert_eq!(t.budget.spent().unknown_cost_calls, 1);
}

#[tokio::test]
async fn repeated_transient_failures_degrade_health_but_do_not_remove_the_model() {
    let w = world(vec![
        (
            "a",
            ProviderKind::Replay,
            vec![err(ErrorCode::ProviderUnavailable); 3],
        ),
        ("b", ProviderKind::Replay, vec![ok("b")]),
    ]);
    w.rt.chat(&task(&w), chat_req("oi"), ChatOptions::text(), None)
        .await
        .unwrap();
    let m = w.rt.registry().models["a:m"].clone();
    assert_eq!(m.health, Health::Degraded);
    assert!(m.enabled, "uma falha transitória não remove o modelo");
}

fn wav() -> Vec<u8> {
    capia_ai::testimg::wav_silence(500, 16_000)
}

fn stt_req() -> SttRequest {
    SttRequest {
        model: "x".into(),
        audio: wav(),
        mime: "audio/wav".into(),
        filename: "a.wav".into(),
        language: None,
        word_timestamps: true,
        prompt: None,
    }
}

fn transcript(text: &str) -> Transcript {
    Transcript {
        schema_version: 1,
        language: Some("pt".into()),
        duration_us: Some(500_000),
        segments: vec![Segment {
            start_us: 0,
            end_us: 500_000,
            text: text.into(),
            confidence: None,
            speaker: None,
            words: vec![],
        }],
    }
}

#[tokio::test]
async fn transcription_routes_by_privacy_retries_and_never_uploads_when_local_only() {
    // dois providers STT: um local (whisper), um de nuvem
    let mut reg = Registry::new();
    for (id, kind) in [
        ("loc", ProviderKind::Replay),
        ("cld", ProviderKind::OpenAiCompatible),
    ] {
        let mut p = ProviderConfig::new(id, kind, id);
        p.enabled = true;
        if kind == ProviderKind::OpenAiCompatible {
            p.base_url = Some("https://api.openai.com/v1".into());
        }
        reg.providers.insert(id.into(), p);
        let mut m = ModelEndpoint::new(format!("{id}:stt"), id, "stt");
        m.capabilities = Capabilities::declared(&[Capability::SpeechToText]);
        reg.models.insert(m.id.clone(), m);
    }
    let mut brain = ModelEndpoint::new("loc:brain", "loc", "brain");
    brain.capabilities = full_caps();
    brain.context_window = 200_000;
    reg.models.insert(brain.id.clone(), brain);
    let mut prof = BrainProfile::new("pf", "pf", "loc:brain");
    prof.privacy = PrivacyPolicy {
        transcription_local_only: true,
        ..PrivacyPolicy::default()
    };
    reg.profiles.insert("pf".into(), prof);
    reg.active_profile = Some("pf".into());
    let sink = Arc::new(MemorySink::default());
    let mut rt = AiRuntime::new(reg, Arc::new(MemoryStore::new()), sink.clone());
    rt.retry = RetryPolicy {
        max_attempts: 3,
        base_ms: 1,
        max_ms: 5,
    };
    let local = Arc::new(ReplayProvider::scripted(
        "loc",
        vec![
            ReplayResponse::Error {
                code: ErrorCode::RateLimited,
                message: "x".into(),
                status: None,
                retry_after_ms: Some(1),
                after_events: vec![],
            },
            ReplayResponse::Transcript {
                transcript: transcript("olá"),
            },
        ],
    ));
    rt.register_replay("loc", local.clone());
    let t = TaskCtx::new("stt", &rt.registry().active().cloned().unwrap());
    let out = rt
        .transcribe(&t, stt_req(), RouteRequest::default())
        .await
        .unwrap();
    assert_eq!(
        out.decision.endpoint_id, "loc:stt",
        "privacidade local-only ⇒ só o provider local"
    );
    assert_eq!(out.transcript.text(), "olá");
    assert_eq!(
        local.call_count(),
        2,
        "retry do 429 no mesmo endpoint local"
    );
    assert!(
        sink.snapshot().iter().all(|r| r.provider_id == "loc"),
        "nenhuma chamada à nuvem"
    );
    // local indisponível ⇒ erro de privacidade, nunca fallback para a nuvem
    rt.update_registry(|r| r.providers.get_mut("loc").unwrap().enabled = false);
    let e = rt
        .transcribe(&t, stt_req(), RouteRequest::default())
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PrivacyPolicyBlocked, "{e}");
}
