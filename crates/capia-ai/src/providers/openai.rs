//! Adapter OpenAI-compatível (OpenAI, OpenRouter, Groq, vLLM, Ollama/LM Studio em loopback…).
//! Não assume que todo endpoint implementa tudo: o probe mede o que existe de fato.

use super::{CallCtx, ModelProvider, RemoteModelInfo, SecretHandle, ToolAccumulator};
use crate::error::{ErrorCode, ProviderError};
use crate::http::{HttpClient, SentResponse};
use crate::registry::{ProviderConfig, ProviderKind};
use crate::stt::{Segment, SttRequest, TRANSCRIPT_SCHEMA_VERSION, Transcript, Word, secs_to_us};
use crate::types::{
    ChatEvent, ChatRequest, ChatStream, FinishReason, Part, Role, ToolChoice, Usage,
};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct OpenAiProvider {
    id: String,
    kind: ProviderKind,
    http: HttpClient,
    secret: SecretHandle,
    extra: Vec<(String, String)>,
    /// `api.openai.com` usa `max_completion_tokens`; os demais, `max_tokens`.
    new_token_param: bool,
    /// OpenAI nativo (`api.openai.com`): `POST /v1/responses` em vez de `chat/completions`.
    responses_api: bool,
}

impl core::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl OpenAiProvider {
    pub fn new(cfg: &ProviderConfig, secret: SecretHandle) -> Result<Self, ProviderError> {
        let http = HttpClient::new(cfg)?;
        let new_token_param = cfg
            .effective_base_url()
            .is_some_and(|u| u.contains("api.openai.com"));
        Ok(Self {
            id: cfg.id.clone(),
            kind: cfg.kind,
            http,
            secret,
            extra: cfg
                .extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            new_token_param,
            responses_api: match cfg.api_style.as_deref() {
                Some("responses") => true,
                Some("chat_completions") => false,
                _ => new_token_param,
            },
        })
    }

    fn headers(
        &self,
        mut rb: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        if let Some(key) = self.secret.resolve()? {
            let mut v = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key.expose()))
                .map_err(|_| {
                    ProviderError::new(ErrorCode::InvalidRequest, "invalid credential characters")
                })?;
            v.set_sensitive(true);
            rb = rb.header(reqwest::header::AUTHORIZATION, v);
        } else if self.kind == ProviderKind::OpenAiCompatible {
            return Err(ProviderError::new(
                ErrorCode::NotConfigured,
                "credential not configured",
            ));
        }
        for (k, v) in &self.extra {
            rb = rb.header(k, v);
        }
        Ok(rb)
    }

    pub(crate) fn build_body(&self, req: &ChatRequest) -> Result<Value, ProviderError> {
        let mut messages = Vec::new();
        for m in &req.messages {
            match m.role {
                Role::System => messages.push(json!({"role": "system", "content": m.text_of()})),
                Role::User => {
                    let mut parts = Vec::new();
                    for p in &m.parts {
                        match p {
                            Part::Text { text } => {
                                parts.push(json!({"type": "text", "text": text}))
                            }
                            Part::Image { mime, data_b64 } => parts.push(json!({
                                "type": "image_url",
                                "image_url": {"url": format!("data:{mime};base64,{data_b64}")}
                            })),
                            Part::Audio { mime, data_b64 } => {
                                let format = match mime.as_str() {
                                    "audio/wav" | "audio/x-wav" => "wav",
                                    "audio/mpeg" | "audio/mp3" => "mp3",
                                    _ => {
                                        return Err(ProviderError::unsupported(
                                            "audio input format (wav/mp3 only)",
                                        ));
                                    }
                                };
                                parts.push(json!({"type": "input_audio", "input_audio": {"data": data_b64, "format": format}}));
                            }
                            _ => {}
                        }
                    }
                    messages.push(json!({"role": "user", "content": parts}));
                }
                Role::Assistant => {
                    let text = m.text_of();
                    let calls: Vec<Value> = m
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            Part::ToolCall {
                                id,
                                name,
                                arguments,
                            } => Some(json!({
                                "id": id, "type": "function",
                                "function": {"name": name, "arguments": arguments.to_string()}
                            })),
                            _ => None,
                        })
                        .collect();
                    let mut o = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) }});
                    if !calls.is_empty() {
                        o["tool_calls"] = json!(calls);
                    }
                    messages.push(o);
                }
                Role::Tool => {
                    for p in &m.parts {
                        if let Part::ToolResult {
                            call_id, content, ..
                        } = p
                        {
                            messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": content}));
                        }
                    }
                }
            }
        }
        let mut body = json!({
            "model": req.model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|t| json!({"type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.input_schema}}))
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = match &req.tool_choice {
                ToolChoice::Auto => json!("auto"),
                ToolChoice::None => json!("none"),
                ToolChoice::Required => json!("required"),
                ToolChoice::Named(n) => json!({"type": "function", "function": {"name": n}}),
            };
        }
        if let Some(rs) = &req.response_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": rs.name, "schema": rs.schema, "strict": true}
            });
        }
        let p = &req.params;
        if let Some(t) = p.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(t) = p.top_p {
            body["top_p"] = json!(t);
        }
        if let Some(n) = p.max_output_tokens {
            let key = if self.new_token_param {
                "max_completion_tokens"
            } else {
                "max_tokens"
            };
            body[key] = json!(n);
        }
        if let Some(s) = p.seed {
            body["seed"] = json!(s);
        }
        if let Some(r) = &p.reasoning_effort {
            body["reasoning_effort"] = json!(r);
        }
        Ok(body)
    }
}

fn usage_from(v: &Value) -> Usage {
    let g = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input_tokens: g("/prompt_tokens"),
        output_tokens: g("/completion_tokens"),
        cached_input_tokens: g("/prompt_tokens_details/cached_tokens"),
        synthetic: false,
    }
}

fn finish_from(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

fn invalid(msg: impl AsRef<str>) -> ProviderError {
    ProviderError::new(ErrorCode::InvalidProviderResponse, msg)
}

/// Resposta **não** em streaming (alguns servidores ignoram `stream: true`).
fn events_from_complete(v: &Value) -> Result<Vec<ChatEvent>, ProviderError> {
    let msg = v
        .pointer("/choices/0/message")
        .ok_or_else(|| invalid("response has no choices[0].message"))?;
    let mut out = Vec::new();
    if let Some(t) = msg.get("content").and_then(Value::as_str)
        && !t.is_empty()
    {
        out.push(ChatEvent::TextDelta { text: t.to_owned() });
    }
    let mut acc = ToolAccumulator::default();
    if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
        for (i, c) in calls.iter().enumerate() {
            acc.push(
                i as u32,
                c.get("id").and_then(Value::as_str),
                c.pointer("/function/name").and_then(Value::as_str),
                c.pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
        }
    }
    out.extend(acc.finish()?);
    if let Some(u) = v.get("usage") {
        out.push(ChatEvent::Usage {
            usage: usage_from(u),
        });
    }
    let fr = v
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    out.push(ChatEvent::Finish {
        reason: finish_from(fr),
    });
    Ok(out)
}

struct SseState {
    sse: crate::http::SseReader,
    acc: ToolAccumulator,
    queue: std::collections::VecDeque<Result<ChatEvent, ProviderError>>,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
    done: bool,
}

fn sse_stream(resp: SentResponse, cancel: crate::CancelToken) -> ChatStream {
    let st = SseState {
        sse: resp.sse(cancel),
        acc: ToolAccumulator::default(),
        queue: Default::default(),
        finish: None,
        usage: None,
        done: false,
    };
    Box::pin(futures_util::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(ev) = st.queue.pop_front() {
                return Some((ev, st));
            }
            if st.done {
                return None;
            }
            match st.sse.next().await {
                Err(e) => {
                    st.done = true;
                    return Some((Err(e), st));
                }
                Ok(None) => {
                    st.done = true;
                    flush(&mut st);
                }
                Ok(Some(ev)) => {
                    if ev.data.trim() == "[DONE]" {
                        st.done = true;
                        flush(&mut st);
                        continue;
                    }
                    let v: Value = match serde_json::from_str(&ev.data) {
                        Ok(v) => v,
                        Err(e) => {
                            st.done = true;
                            return Some((
                                Err(invalid(format!("malformed stream event: {e}"))),
                                st,
                            ));
                        }
                    };
                    if let Some(err) = v.get("error") {
                        st.done = true;
                        let m = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("provider stream error");
                        return Some((
                            Err(ProviderError::new(ErrorCode::ProviderUnavailable, m)),
                            st,
                        ));
                    }
                    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                        st.usage = Some(usage_from(u));
                    }
                    let Some(choice) = v.pointer("/choices/0") else {
                        continue;
                    };
                    if let Some(t) = choice.pointer("/delta/content").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        st.queue
                            .push_back(Ok(ChatEvent::TextDelta { text: t.to_owned() }));
                    }
                    if let Some(calls) = choice
                        .pointer("/delta/tool_calls")
                        .and_then(Value::as_array)
                    {
                        for c in calls {
                            let idx = c.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                            let name = c.pointer("/function/name").and_then(Value::as_str);
                            let args = c
                                .pointer("/function/arguments")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            st.acc
                                .push(idx, c.get("id").and_then(Value::as_str), name, args);
                            st.queue.push_back(Ok(ChatEvent::ToolCallDelta {
                                index: idx,
                                name: name.map(str::to_owned),
                                arguments_fragment: args.to_owned(),
                            }));
                        }
                    }
                    if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                        st.finish = Some(finish_from(fr));
                    }
                }
            }
        }
    }))
}

fn flush(st: &mut SseState) {
    let acc = std::mem::take(&mut st.acc);
    match acc.finish() {
        Ok(calls) => st.queue.extend(calls.into_iter().map(Ok)),
        Err(e) => st.queue.push_back(Err(e)),
    }
    if let Some(u) = st.usage.take() {
        st.queue.push_back(Ok(ChatEvent::Usage { usage: u }));
    }
    st.queue.push_back(Ok(ChatEvent::Finish {
        reason: st.finish.take().unwrap_or(FinishReason::Stop),
    }));
}

#[async_trait]
impl ModelProvider for OpenAiProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(&self, req: &ChatRequest, ctx: &CallCtx) -> Result<ChatStream, ProviderError> {
        if self.responses_api {
            let body = self.build_responses_body(req)?;
            let rb = self.headers(self.http.post("responses")?)?.json(&body);
            let resp = self.http.send(rb, &ctx.cancel).await?;
            return if resp.is_event_stream() {
                Ok(super::openai_responses::sse_stream(
                    resp,
                    ctx.cancel.clone(),
                ))
            } else {
                let v: Value = resp.json(&ctx.cancel).await?;
                let events = super::openai_responses::events_from_response(&v)?;
                Ok(Box::pin(futures_util::stream::iter(
                    events.into_iter().map(Ok),
                )))
            };
        }
        let body = self.build_body(req)?;
        let rb = self
            .headers(self.http.post("chat/completions")?)?
            .json(&body);
        let resp = self.http.send(rb, &ctx.cancel).await?;
        if resp.is_event_stream() {
            Ok(sse_stream(resp, ctx.cancel.clone()))
        } else {
            let v: Value = resp.json(&ctx.cancel).await?;
            let events = events_from_complete(&v)?;
            Ok(Box::pin(futures_util::stream::iter(
                events.into_iter().map(Ok),
            )))
        }
    }

    async fn transcribe(
        &self,
        req: &SttRequest,
        ctx: &CallCtx,
    ) -> Result<Transcript, ProviderError> {
        let part = reqwest::multipart::Part::bytes(req.audio.clone())
            .file_name(req.filename.clone())
            .mime_str(&req.mime)
            .map_err(|_| ProviderError::new(ErrorCode::InvalidRequest, "invalid audio mime"))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", req.model.clone())
            .text("response_format", "verbose_json")
            .text("timestamp_granularities[]", "segment");
        if req.word_timestamps {
            form = form.text("timestamp_granularities[]", "word");
        }
        if let Some(l) = &req.language {
            form = form.text("language", l.clone());
        }
        if let Some(p) = &req.prompt {
            form = form.text("prompt", p.clone());
        }
        let rb = self
            .headers(self.http.post("audio/transcriptions")?)?
            .multipart(form);
        let v: Value = self
            .http
            .send(rb, &ctx.cancel)
            .await?
            .json(&ctx.cancel)
            .await?;
        parse_verbose_json(&v)
    }

    async fn list_models(&self, ctx: &CallCtx) -> Result<Vec<RemoteModelInfo>, ProviderError> {
        let rb = self.headers(self.http.get("models")?)?;
        let v: Value = self
            .http
            .send(rb, &ctx.cancel)
            .await?
            .json(&ctx.cancel)
            .await?;
        let data = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("models response has no `data` array"))?;
        Ok(data
            .iter()
            .filter_map(|m| {
                Some(RemoteModelInfo {
                    id: m.get("id")?.as_str()?.to_owned(),
                    display_name: None,
                    context_window: m
                        .get("context_length")
                        .or_else(|| m.get("context_window"))
                        .and_then(Value::as_u64)
                        .map(|n| n as u32),
                })
            })
            .collect())
    }
}

/// `verbose_json` do endpoint de transcrição (segundos float → microssegundos inteiros).
pub(crate) fn parse_verbose_json(v: &Value) -> Result<Transcript, ProviderError> {
    let mut segments = Vec::new();
    let words: Vec<Word> = v
        .get("words")
        .and_then(Value::as_array)
        .map(|ws| {
            ws.iter()
                .filter_map(|w| {
                    Some(Word {
                        start_us: secs_to_us(w.get("start")?.as_f64()?),
                        end_us: secs_to_us(w.get("end")?.as_f64()?),
                        text: w.get("word")?.as_str()?.trim().to_owned(),
                        confidence: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(segs) = v.get("segments").and_then(Value::as_array) {
        for s in segs {
            let start = secs_to_us(s.get("start").and_then(Value::as_f64).unwrap_or(0.0));
            let end = secs_to_us(s.get("end").and_then(Value::as_f64).unwrap_or(0.0)).max(start);
            let seg_words: Vec<Word> = words
                .iter()
                .filter(|w| w.start_us >= start && w.start_us < end.max(start + 1))
                .cloned()
                .collect();
            segments.push(Segment {
                start_us: start,
                end_us: end,
                text: s
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned(),
                confidence: s
                    .get("avg_logprob")
                    .and_then(Value::as_f64)
                    .map(|lp| lp.exp().clamp(0.0, 1.0) as f32),
                speaker: None,
                words: seg_words,
            });
        }
    } else if let Some(t) = v.get("text").and_then(Value::as_str) {
        let dur = secs_to_us(v.get("duration").and_then(Value::as_f64).unwrap_or(0.0));
        segments.push(Segment {
            start_us: 0,
            end_us: dur,
            text: t.trim().to_owned(),
            confidence: None,
            speaker: None,
            words,
        });
    } else {
        return Err(invalid(
            "transcription response has neither segments nor text",
        ));
    }
    let t = Transcript {
        schema_version: TRANSCRIPT_SCHEMA_VERSION,
        language: v.get("language").and_then(Value::as_str).map(str::to_owned),
        duration_us: v.get("duration").and_then(Value::as_f64).map(secs_to_us),
        segments,
    };
    t.validate()
        .map_err(|m| invalid(format!("transcript invalid: {m}")))?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::types::{Message, ToolSpec};

    fn prov() -> OpenAiProvider {
        let mut c = ProviderConfig::new("o", ProviderKind::OpenAiCompatible, "o");
        c.base_url = Some("https://api.openai.com/v1".into());
        OpenAiProvider::new(&c, SecretHandle::none()).unwrap()
    }

    #[test]
    fn body_maps_messages_tools_schema_and_params() {
        let mut r = ChatRequest::new(
            "gpt-x",
            vec![
                Message::system("sys"),
                Message {
                    role: Role::User,
                    parts: vec![
                        Part::text("olha"),
                        Part::Image {
                            mime: "image/png".into(),
                            data_b64: "AAAA".into(),
                        },
                    ],
                },
                Message {
                    role: Role::Assistant,
                    parts: vec![Part::ToolCall {
                        id: "c1".into(),
                        name: "t".into(),
                        arguments: json!({"a": 1}),
                    }],
                },
                Message {
                    role: Role::Tool,
                    parts: vec![Part::ToolResult {
                        call_id: "c1".into(),
                        name: "t".into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
            ],
        );
        r.tools.push(ToolSpec {
            name: "t".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        });
        r.tool_choice = ToolChoice::Named("t".into());
        r.response_schema = Some(crate::types::ResponseSchema {
            name: "s".into(),
            schema: json!({"type": "object"}),
        });
        r.params.max_output_tokens = Some(100);
        r.params.temperature = Some(0.0);
        let b = prov().build_body(&r).unwrap();
        assert_eq!(
            b["messages"][1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
        assert_eq!(
            b["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{\"a\":1}"
        );
        assert_eq!(b["messages"][3]["role"], "tool");
        assert_eq!(b["tool_choice"]["function"]["name"], "t");
        assert_eq!(b["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            b["max_completion_tokens"], 100,
            "api.openai.com usa max_completion_tokens"
        );
        assert_eq!(b["stream_options"]["include_usage"], true);
    }

    #[test]
    fn complete_response_parses_tools_and_usage() {
        let v = json!({"choices": [{"message": {"content": "oi", "tool_calls": [
            {"id": "x", "function": {"name": "f", "arguments": "{\"k\":2}"}}]}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "prompt_tokens_details": {"cached_tokens": 4}}});
        let ev = events_from_complete(&v).unwrap();
        assert!(matches!(&ev[0], ChatEvent::TextDelta { text } if text == "oi"));
        assert!(
            matches!(&ev[1], ChatEvent::ToolCall { name, arguments, .. } if name == "f" && arguments["k"] == 2)
        );
        assert!(
            matches!(&ev[2], ChatEvent::Usage { usage } if usage.input_tokens == 10 && usage.cached_input_tokens == 4)
        );
        assert!(matches!(
            &ev[3],
            ChatEvent::Finish {
                reason: FinishReason::ToolCalls
            }
        ));
    }

    #[test]
    fn verbose_json_to_canonical_transcript() {
        let v = json!({"language": "pt", "duration": 3.2, "segments": [
            {"start": 0.0, "end": 1.5, "text": " Olá mundo", "avg_logprob": -0.1},
            {"start": 1.5, "end": 3.2, "text": " tudo bem"}],
            "words": [{"word": "Olá", "start": 0.0, "end": 0.6}, {"word": "mundo", "start": 0.7, "end": 1.4},
                      {"word": "tudo", "start": 1.6, "end": 2.0}, {"word": "bem", "start": 2.1, "end": 3.0}]});
        let t = parse_verbose_json(&v).unwrap();
        assert_eq!(t.language.as_deref(), Some("pt"));
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].words.len(), 2);
        assert_eq!(t.segments[1].words[0].text, "tudo");
        assert_eq!(t.segments[0].end_us, 1_500_000);
    }
}
