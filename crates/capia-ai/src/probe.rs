//! Probe de capabilities **medido** (não só `/models`): auth/conectividade, texto, streaming,
//! tool call, saída estruturada, visão e STT. Nunca imprime segredo.

use crate::cancel::CancelToken;
use crate::capability::{Capabilities, Capability};
use crate::error::{ErrorCode, ProviderError};
use crate::providers::{CallCtx, ModelProvider, collect};
use crate::stt::SttRequest;
use crate::testimg;
use crate::types::{ChatEvent, ChatRequest, Message, Part, ResponseSchema, ToolChoice, ToolSpec};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    /// Conectou, autenticou e nenhum probe tentado falhou.
    pub success: bool,
    pub latency_ms: u64,
    pub verified: Vec<Capability>,
    /// Falhas por capability (mensagens já redigidas).
    pub failures: BTreeMap<Capability, String>,
    pub provider_kind: String,
    pub model_id: String,
    pub timestamp_ms: u64,
    /// Erro de nível de conexão/auth (impede os demais probes).
    pub connection_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeOptions {
    pub vision: bool,
    pub stt: bool,
    pub chat: bool,
}

impl ProbeOptions {
    /// O que vale a pena testar dado o que o modelo **declara**.
    pub fn from_declared(c: &Capabilities) -> Self {
        Self {
            chat: c.has(Capability::TextGeneration) || !c.has(Capability::SpeechToText),
            vision: c.has(Capability::VisionInput),
            stt: c.has(Capability::SpeechToText),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn base_req(model: &str, text: &str) -> ChatRequest {
    let mut r = ChatRequest::new(model, vec![Message::user(text)]);
    r.params.temperature = Some(0.0);
    r.params.max_output_tokens = Some(128);
    r.meta.purpose = Some("probe".into());
    r
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

async fn run<F: std::future::Future<Output = Result<T, ProviderError>>, T>(
    cancel: &CancelToken,
    f: F,
) -> Result<T, ProviderError> {
    tokio::select! {
        () = cancel.cancelled() => Err(ProviderError::cancelled()),
        r = tokio::time::timeout(PROBE_TIMEOUT, f) => r.unwrap_or_else(|_| Err(ProviderError::new(ErrorCode::ProviderTimeout, "probe step timed out"))),
    }
}

pub async fn probe(
    provider: &dyn ModelProvider,
    model: &str,
    opts: ProbeOptions,
    cancel: &CancelToken,
) -> ProbeResult {
    let started = Instant::now();
    let ctx = CallCtx::new(cancel.clone());
    let mut verified = Vec::new();
    let mut failures = BTreeMap::new();
    let mut conn_err: Option<String> = None;

    if opts.chat {
        // 1) texto + streaming (≥ 2 deltas ⇒ chegou incrementalmente)
        let req = base_req(
            model,
            "Count from 1 to 8 separated by single spaces. Output only the numbers.",
        );
        match run(cancel, async {
            let mut stream = provider.chat(&req, &ctx).await?;
            let (mut deltas, mut text, mut finished) = (0u32, String::new(), false);
            while let Some(ev) = stream.next().await {
                match ev? {
                    ChatEvent::TextDelta { text: t } => {
                        deltas += 1;
                        text.push_str(&t);
                    }
                    ChatEvent::Finish { .. } => finished = true,
                    _ => {}
                }
            }
            Ok((deltas, text, finished))
        })
        .await
        {
            Ok((deltas, text, finished)) => {
                if !text.trim().is_empty() && finished {
                    verified.push(Capability::TextGeneration);
                } else {
                    failures.insert(
                        Capability::TextGeneration,
                        "empty or unfinished response".to_owned(),
                    );
                }
                if deltas >= 2 {
                    verified.push(Capability::Streaming);
                } else {
                    failures.insert(
                        Capability::Streaming,
                        "response arrived in a single chunk".to_owned(),
                    );
                }
            }
            Err(e) => {
                // auth/conectividade/modelo inexistente: não adianta seguir
                if matches!(
                    e.code,
                    ErrorCode::AuthFailed
                        | ErrorCode::NotConfigured
                        | ErrorCode::ProviderUnavailable
                        | ErrorCode::ProviderTimeout
                        | ErrorCode::ModelNotFound
                        | ErrorCode::Cancelled
                ) {
                    conn_err = Some(e.to_string());
                } else {
                    failures.insert(Capability::TextGeneration, e.to_string());
                }
            }
        }

        if conn_err.is_none() {
            // 2) tool call
            let mut req = base_req(
                model,
                "Call the `echo` tool with message \"hi\". Do not write any text.",
            );
            req.tools.push(ToolSpec {
                name: "echo".into(),
                description: "Echo a message".into(),
                input_schema: json!({"type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]}),
            });
            req.tool_choice = ToolChoice::Required;
            match run(cancel, async {
                collect(provider.chat(&req, &ctx).await?, cancel).await
            })
            .await
            {
                Ok(r)
                    if r.tool_calls.iter().any(|c| {
                        c.name == "echo"
                            && c.arguments
                                .get("message")
                                .is_some_and(serde_json::Value::is_string)
                    }) =>
                {
                    verified.push(Capability::ToolCalling);
                }
                Ok(_) => {
                    failures.insert(
                        Capability::ToolCalling,
                        "no valid tool call returned".to_owned(),
                    );
                }
                Err(e) => {
                    failures.insert(Capability::ToolCalling, e.to_string());
                }
            }

            // 3) saída estruturada
            let schema = json!({"type": "object", "properties": {"ok": {"type": "boolean"}, "n": {"type": "integer"}},
                "required": ["ok", "n"], "additionalProperties": false});
            let mut req = base_req(model, "Return ok=true and n=7.");
            req.response_schema = Some(ResponseSchema {
                name: "probe".into(),
                schema: schema.clone(),
            });
            match run(cancel, async {
                collect(provider.chat(&req, &ctx).await?, cancel).await
            })
            .await
            {
                Ok(r) => match crate::dispatcher::parse_and_validate(&r.text, &schema) {
                    Ok(v) if v["ok"] == json!(true) && v["n"] == json!(7) => {
                        verified.push(Capability::StructuredOutput)
                    }
                    Ok(_) => {
                        failures.insert(
                            Capability::StructuredOutput,
                            "valid JSON with wrong content".to_owned(),
                        );
                    }
                    Err(e) => {
                        failures.insert(Capability::StructuredOutput, e);
                    }
                },
                Err(e) => {
                    failures.insert(Capability::StructuredOutput, e.to_string());
                }
            }

            // 4) visão
            if opts.vision {
                let mut req = base_req(
                    model,
                    "What is the dominant color of this image? Answer with one lowercase English word.",
                );
                let png = testimg::png_solid(32, 32, [255, 0, 0]);
                req.messages[0].parts.push(Part::Image {
                    mime: "image/png".into(),
                    data_b64: testimg::base64(&png),
                });
                match run(cancel, async {
                    collect(provider.chat(&req, &ctx).await?, cancel).await
                })
                .await
                {
                    Ok(r) if r.text.to_ascii_lowercase().contains("red") => {
                        verified.push(Capability::VisionInput)
                    }
                    Ok(r) => {
                        failures.insert(
                            Capability::VisionInput,
                            format!(
                                "unexpected answer: {}",
                                r.text.chars().take(60).collect::<String>()
                            ),
                        );
                    }
                    Err(e) => {
                        failures.insert(Capability::VisionInput, e.to_string());
                    }
                }
            }
        }
    }

    // 5) STT
    if opts.stt && conn_err.is_none() {
        let req = SttRequest {
            model: model.to_owned(),
            audio: testimg::wav_silence(600, 16_000),
            mime: "audio/wav".into(),
            filename: "probe.wav".into(),
            language: None,
            word_timestamps: false,
            prompt: None,
        };
        match run(cancel, async { provider.transcribe(&req, &ctx).await }).await {
            Ok(_) => verified.push(Capability::SpeechToText),
            Err(e) if e.code == ErrorCode::Cancelled => conn_err = Some(e.to_string()),
            Err(e) => {
                failures.insert(Capability::SpeechToText, e.to_string());
            }
        }
    }

    ProbeResult {
        success: conn_err.is_none() && failures.is_empty() && !verified.is_empty(),
        latency_ms: started.elapsed().as_millis() as u64,
        verified,
        failures,
        provider_kind: format!("{:?}", provider.kind()),
        model_id: model.to_owned(),
        timestamp_ms: now_ms(),
        connection_error: conn_err,
    }
}
